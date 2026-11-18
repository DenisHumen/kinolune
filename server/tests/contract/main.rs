// SPDX-License-Identifier: MPL-2.0
//! SRV-002 "done when": CI fails when the code and the spec differ; contract
//! tests for N-1.
//!
//! - `docs/api/openapi.json` is exactly what the code generates
//!   (regenerate: `just server openapi`);
//! - the running server answers every operation of the spec with a
//!   documented status and a body that matches the schema;
//! - frozen contracts of supported older versions (`docs/api/compat/`, the
//!   previous major N-1 and released minors of the current one) still hold:
//!   the current spec is backward compatible with each, and the running
//!   server answers their operations as they describe.

mod compat;
mod live;

use std::path::{Path, PathBuf};

use pa_server::api::{API_MAJOR, openapi_json};
use pa_server::{Config, Server};
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// Set to rewrite `docs/api/openapi.json` instead of comparing it.
const BLESS: &str = "KINOLUNE_BLESS_OPENAPI";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

fn spec_path() -> PathBuf {
    repo().join("docs/api/openapi.json")
}

fn current() -> Value {
    serde_json::from_str(&openapi_json()).unwrap()
}

/// Frozen specs: `docs/api/compat/openapi-<major>.<minor>.json`.
fn frozen() -> Vec<(String, (u32, u32), Value)> {
    let dir = repo().join("docs/api/compat");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let version = name
            .strip_prefix("openapi-")
            .and_then(|n| n.strip_suffix(".json"))
            .and_then(|v| v.split_once('.'))
            .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)))
            .unwrap_or_else(|| panic!("{name}: expected openapi-<major>.<minor>.json"));
        let spec: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            spec["info"]["version"],
            json!(format!("{}.{}", version.0, version.1)),
            "{name}: info.version does not match the file name"
        );
        out.push((name, version, spec));
    }
    out.sort_by_key(|(_, version, _)| *version);
    out
}

/// Every operation of a spec as (`METHOD /path`, operation).
fn operations(spec: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            if op.get("responses").is_some() {
                out.push((format!("{} {path}", method.to_uppercase()), op.clone()));
            }
        }
    }
    out
}

#[test]
fn committed_spec_matches_code() {
    let generated = openapi_json();
    let path = spec_path();
    if std::env::var_os(BLESS).is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    if committed != generated {
        let line = committed
            .lines()
            .zip(generated.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| committed.lines().count().min(generated.lines().count()));
        panic!(
            "docs/api/openapi.json differs from the code (first at line {}). \
             Regenerate it with `just server openapi` (or `{BLESS}=1 cargo test -p pa-server \
             --test contract`) and commit it; a change that breaks a frozen contract in \
             docs/api/compat/ needs a new major version (ADR-0118).",
            line + 1
        );
    }
}

#[test]
fn spec_is_openapi_3_1_of_this_major() {
    let spec = current();
    assert!(spec["openapi"].as_str().unwrap().starts_with("3.1."));
    let version = spec["info"]["version"].as_str().unwrap();
    assert_eq!(
        version.split('.').next().unwrap(),
        API_MAJOR.to_string(),
        "info.version {version}"
    );
    for (key, op) in operations(&spec) {
        assert!(op["operationId"].is_string(), "{key}: no operationId");
        let path = key.split(' ').nth(1).unwrap();
        assert!(
            path.starts_with(&format!("/api/v{API_MAJOR}/")) || path.starts_with("/health"),
            "{key}: API paths live under /api/v{API_MAJOR}"
        );
    }
}

/// N-1: the previous major stays supported, and only supported versions are
/// kept frozen.
#[test]
fn frozen_contracts_are_the_supported_ones() {
    let frozen = frozen();
    assert!(
        !frozen.is_empty(),
        "docs/api/compat/ must hold the frozen contract of the current major"
    );
    for (name, (major, _), _) in &frozen {
        assert!(
            *major + 1 >= API_MAJOR && *major <= API_MAJOR,
            "{name}: API {API_MAJOR} supports majors {} and {API_MAJOR} only; delete it",
            API_MAJOR.saturating_sub(1)
        );
    }
    if API_MAJOR > 1 {
        assert!(
            frozen
                .iter()
                .any(|(_, (major, _), _)| *major == API_MAJOR - 1),
            "the last contract of API {} must be frozen in docs/api/compat/ (N-1)",
            API_MAJOR - 1
        );
    }
}

#[test]
fn current_spec_keeps_frozen_contracts() {
    let current = current();
    for (name, _, old) in frozen() {
        let broken = compat::breaking_changes(&old, &current);
        assert!(
            broken.is_empty(),
            "the API breaks the contract {name}:\n  {}",
            broken.join("\n  ")
        );
    }
}

/// The checker itself: additive changes pass, breaking ones are caught.
#[test]
fn compat_checker_tells_additive_from_breaking() {
    let base = current();
    assert!(compat::breaking_changes(&base, &base).is_empty());

    let additive = |change: fn(&mut Value)| {
        let mut new = base.clone();
        change(&mut new);
        let broken = compat::breaking_changes(&base, &new);
        assert!(broken.is_empty(), "flagged as breaking: {broken:?}");
    };
    // A new endpoint.
    additive(|s| {
        s["paths"]["/api/v1/new"] = json!({"get": {"responses": {"200": {"description": "x"}}}});
    });
    // A new optional response field.
    additive(|s| {
        s["components"]["schemas"]["Health"]["properties"]["uptime_ms"] =
            json!({"type": "integer"});
    });
    // A new error status.
    additive(|s| {
        s["paths"]["/api/v1/devices"]["get"]["responses"]["401"] = json!({"description": "x", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ApiError"}}}});
    });
    // A new optional query value.
    additive(|s| {
        s["paths"]["/api/v1/pair/invite"]["post"]["parameters"][0]["schema"]["enum"]
            .as_array_mut()
            .unwrap()
            .push(json!("long"));
    });

    let breaking = |what: &str, change: fn(&mut Value)| {
        let mut new = base.clone();
        change(&mut new);
        let broken = compat::breaking_changes(&base, &new);
        assert!(!broken.is_empty(), "not caught: {what}");
    };
    breaking("removed operation", |s| {
        s["paths"].as_object_mut().unwrap().remove("/healthz");
    });
    breaking("removed method", |s| {
        s["paths"]["/api/v1/devices/{id}"]
            .as_object_mut()
            .unwrap()
            .remove("delete");
    });
    breaking("required field dropped", |s| {
        s["components"]["schemas"]["Health"]["required"] = json!(["service", "status"]);
    });
    breaking("field type changed", |s| {
        s["components"]["schemas"]["InviteResponse"]["properties"]["expires_ms"] =
            json!({"type": "string"});
    });
    breaking("field became nullable", |s| {
        s["components"]["schemas"]["Devices"]["properties"]["family"] =
            json!({"type": ["string", "null"]});
    });
    breaking("new response enum value", |s| {
        s["components"]["schemas"]["Status"]["enum"] = json!(["ok", "error", "degraded"]);
    });
    breaking("array items changed", |s| {
        s["components"]["schemas"]["DeviceInfo"]["properties"]["added_ms"] =
            json!({"type": "string"});
    });
    breaking("new required query parameter", |s| {
        s["paths"]["/api/v1/devices"]["get"]["parameters"] = json!([{"name": "page", "in": "query", "required": true, "schema": {"type": "integer"}}]);
    });
    breaking("query value dropped", |s| {
        s["paths"]["/api/v1/pair/invite"]["post"]["parameters"][0]["schema"]["enum"] =
            json!(["code"]);
    });
    breaking("new required body", |s| {
        s["paths"]["/api/v1/pair/invite"]["post"]["requestBody"] = json!({
            "required": true,
            "content": {"application/json": {"schema": {"type": "object"}}}
        });
    });
    breaking("success status changed", |s| {
        let responses = s["paths"]["/api/v1/devices/{id}"]["delete"]["responses"]
            .as_object_mut()
            .unwrap();
        let revoked = responses.remove("204").unwrap();
        responses.insert("200".to_owned(), revoked);
    });
    breaking("body type changed", |s| {
        let content = s["paths"]["/health"]["get"]["responses"]["200"]["content"]
            .as_object_mut()
            .unwrap();
        let json = content.remove("application/json").unwrap();
        content.insert("text/plain".to_owned(), json);
    });
}

fn config() -> Config {
    Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        // Plain HTTP: the client here speaks raw HTTP.
        tls: pa_server::config::TlsConfig {
            enabled: false,
            ..Default::default()
        },
        ..Config::default()
    }
}

/// The running server against the current spec (strictly: every field it
/// sends is described) and against every frozen contract (N-1).
#[test]
fn server_answers_as_specs_describe() {
    let mut specs = vec![("openapi.json (current)".to_owned(), current(), true)];
    specs.extend(
        frozen()
            .into_iter()
            .map(|(name, _, spec)| (format!("compat/{name}"), spec, false)),
    );
    let scenarios = live::scenarios();
    let current_ops: Vec<String> = operations(&current()).into_iter().map(|(k, _)| k).collect();
    for (key, _) in &scenarios {
        assert!(
            current_ops.contains(&key.to_string()),
            "scenario for {key}, which is not in the spec"
        );
    }

    let tmp = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let server = Server::bind(&config(), tmp.path()).await.unwrap();
        let addr = server.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(server.run(async {
            let _ = stopped.await;
        }));

        let mut problems = Vec::new();
        for (name, spec, strict) in &specs {
            for (key, op) in operations(spec) {
                let Some((_, calls)) = scenarios.iter().find(|(k, _)| *k == key) else {
                    problems.push(format!("{name}: {key} has no contract scenario (live.rs)"));
                    continue;
                };
                for call in calls {
                    let reply = live::send(addr, call).await;
                    let at = format!("{name}: {} {}", call.method, call.target);
                    if reply.status != call.expect {
                        problems.push(format!(
                            "{at}: status {} instead of {}: {}",
                            reply.status, call.expect, reply.body
                        ));
                    }
                    problems.extend(
                        live::check(spec, &op, &reply, *strict)
                            .into_iter()
                            .map(|p| format!("{at}: {p}")),
                    );
                }
            }
        }

        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(problems.is_empty(), "\n  {}", problems.join("\n  "));
    });
}
