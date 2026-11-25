// SPDX-License-Identifier: MPL-2.0
//! Requests to a running server, checked against a spec: the status must be
//! documented for the operation and a JSON body must match its schema.

use std::net::SocketAddr;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::compat::resolve;

/// One request of a scenario.
pub struct Call {
    pub method: &'static str,
    /// Path and query.
    pub target: &'static str,
    /// Sends a WebSocket upgrade and reads only the response head.
    pub upgrade: bool,
    /// JSON request body.
    pub body: Option<&'static str>,
    /// The status this request must get.
    pub expect: u16,
}

const fn call(method: &'static str, target: &'static str, expect: u16) -> Call {
    Call {
        method,
        target,
        upgrade: false,
        body: None,
        expect,
    }
}

const fn json(method: &'static str, target: &'static str, body: &'static str, expect: u16) -> Call {
    Call {
        method,
        target,
        upgrade: false,
        body: Some(body),
        expect,
    }
}

const fn upgrade(target: &'static str) -> Call {
    Call {
        method: "GET",
        target,
        upgrade: true,
        body: None,
        expect: 101,
    }
}

/// Requests for every operation of the API, by `METHOD /path/template`.
/// Each operation of every checked spec must have an entry: an endpoint
/// without a contract test fails the suite.
///
/// The server is fresh: no administrator exists, so loopback administers
/// (SRV-003). No scenario may create one: the scenarios run once per spec.
pub fn scenarios() -> Vec<(&'static str, Vec<Call>)> {
    vec![
        ("GET /health", vec![call("GET", "/health", 200)]),
        ("GET /healthz", vec![call("GET", "/healthz", 200)]),
        (
            "GET /api/v1/sync",
            vec![upgrade("/api/v1/sync"), call("GET", "/api/v1/sync", 400)],
        ),
        (
            "GET /api/v1/pair",
            vec![upgrade("/api/v1/pair"), call("GET", "/api/v1/pair", 400)],
        ),
        (
            "POST /api/v1/pair/invite",
            vec![
                call("POST", "/api/v1/pair/invite", 200),
                call("POST", "/api/v1/pair/invite?kind=code", 200),
                call("POST", "/api/v1/pair/invite?kind=qr", 200),
                call("POST", "/api/v1/pair/invite?kind=bogus", 400),
            ],
        ),
        (
            "GET /api/v1/devices",
            vec![call("GET", "/api/v1/devices", 200)],
        ),
        (
            "DELETE /api/v1/devices/{id}",
            vec![call("DELETE", "/api/v1/devices/nobody", 404)],
        ),
        (
            "POST /api/v1/auth/setup",
            vec![
                json("POST", "/api/v1/auth/setup", "{}", 400),
                json(
                    "POST",
                    "/api/v1/auth/setup",
                    r#"{"name":"Admin","password":"short"}"#,
                    400,
                ),
            ],
        ),
        (
            "POST /api/v1/auth/login",
            vec![
                json(
                    "POST",
                    "/api/v1/auth/login",
                    r#"{"name":"nobody","password":"wrong password"}"#,
                    401,
                ),
                json("POST", "/api/v1/auth/login", "[]", 400),
            ],
        ),
        (
            "POST /api/v1/auth/pin",
            vec![json(
                "POST",
                "/api/v1/auth/pin",
                r#"{"user_id":"nobody","pin":"1234"}"#,
                401,
            )],
        ),
        (
            "POST /api/v1/auth/challenge",
            vec![call("POST", "/api/v1/auth/challenge", 200)],
        ),
        (
            "POST /api/v1/auth/device",
            vec![
                json(
                    "POST",
                    "/api/v1/auth/device",
                    r#"{"device_id":"nobody","nonce":"00","challenge":"00","sig":"00"}"#,
                    401,
                ),
                json("POST", "/api/v1/auth/device", "{}", 400),
            ],
        ),
        (
            "POST /api/v1/auth/refresh",
            vec![
                json(
                    "POST",
                    "/api/v1/auth/refresh",
                    r#"{"refresh_token":"bogus"}"#,
                    401,
                ),
                json("POST", "/api/v1/auth/refresh", "{}", 400),
            ],
        ),
        (
            "POST /api/v1/auth/logout",
            vec![json(
                "POST",
                "/api/v1/auth/logout",
                r#"{"refresh_token":"bogus"}"#,
                204,
            )],
        ),
        (
            "GET /api/v1/auth/me",
            vec![call("GET", "/api/v1/auth/me", 401)],
        ),
        (
            "GET /api/v1/auth/profiles",
            vec![call("GET", "/api/v1/auth/profiles", 401)],
        ),
        ("GET /api/v1/users", vec![call("GET", "/api/v1/users", 200)]),
        (
            "POST /api/v1/users",
            vec![
                json("POST", "/api/v1/users", r#"{"name":"Kid"}"#, 400),
                json("POST", "/api/v1/users", "{}", 400),
            ],
        ),
        (
            "PATCH /api/v1/users/{id}",
            vec![json("PATCH", "/api/v1/users/nobody", "{}", 404)],
        ),
        (
            "DELETE /api/v1/users/{id}",
            vec![call("DELETE", "/api/v1/users/nobody", 404)],
        ),
    ]
}

/// A response as read from the socket.
pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

/// Minimal HTTP/1.1 client: no HTTP client crate needed.
pub async fn send(addr: SocketAddr, call: &Call) -> Reply {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body = call.body.unwrap_or_default();
    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n",
        call.method,
        call.target,
        body.len()
    );
    if call.body.is_some() {
        request.push_str("Content-Type: application/json\r\n");
    }
    if call.upgrade {
        request.push_str(
            "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        );
    } else {
        request.push_str("Connection: close\r\n\r\n");
        request.push_str(body);
    }
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let status = line.split(' ').nth(1).unwrap().parse().unwrap();
    let mut content_type = None;
    loop {
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-type")
        {
            content_type = Some(value.trim().to_owned());
        }
    }
    let mut body = String::new();
    if !call.upgrade {
        reader.read_to_string(&mut body).await.unwrap();
    }
    Reply {
        status,
        content_type,
        body,
    }
}

/// Problems of `reply` to operation `op` of `spec`; empty when it conforms.
/// With `strict`, fields the spec does not describe are problems too (the
/// current spec must describe everything the server sends; older specs may
/// lack the fields added since).
pub fn check(spec: &Value, op: &Value, reply: &Reply, strict: bool) -> Vec<String> {
    let status = reply.status.to_string();
    let Some(response) = op["responses"].get(&status) else {
        return vec![format!("status {status} is not documented")];
    };
    let response = resolve(spec, response);
    let Some(content) = response["content"].as_object() else {
        return Vec::new();
    };
    let Some(schema) = content.get("application/json").map(|m| &m["schema"]) else {
        return Vec::new();
    };
    let mut problems = Vec::new();
    if !reply
        .content_type
        .as_deref()
        .is_some_and(|t| t.starts_with("application/json"))
    {
        problems.push(format!(
            "content type {:?} instead of application/json",
            reply.content_type
        ));
    }
    let body: Value = match serde_json::from_str(&reply.body) {
        Ok(body) => body,
        Err(e) => {
            problems.push(format!("body is not JSON ({e}): {}", reply.body));
            return problems;
        }
    };
    // OpenAPI 3.1 schemas are JSON Schema 2020-12; `$ref`s point into the
    // components of the spec, so they go along.
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "components": spec["components"],
        "allOf": [schema],
    });
    let validator = jsonschema::validator_for(&root).expect("the spec schema compiles");
    problems.extend(
        validator
            .iter_errors(&body)
            .map(|e| format!("{} at {}", e, e.instance_path())),
    );
    if strict {
        undocumented(spec, schema, &body, "", &mut problems);
    }
    problems
}

/// Object fields of `value` that `schema` does not describe.
fn undocumented(spec: &Value, schema: &Value, value: &Value, at: &str, out: &mut Vec<String>) {
    let schema = resolve(spec, schema);
    for part in schema["allOf"].as_array().into_iter().flatten() {
        undocumented(spec, part, value, at, out);
    }
    match value {
        Value::Object(fields) if schema["properties"].is_object() => {
            for (name, field) in fields {
                match schema["properties"].get(name) {
                    None => out.push(format!("field {at}/{name} is not in the spec")),
                    Some(prop) => undocumented(spec, prop, field, &format!("{at}/{name}"), out),
                }
            }
        }
        Value::Array(items) if schema["items"].is_object() => {
            for (i, item) in items.iter().enumerate() {
                undocumented(spec, &schema["items"], item, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}
