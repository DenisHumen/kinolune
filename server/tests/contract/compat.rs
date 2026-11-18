// SPDX-License-Identifier: MPL-2.0
//! Backward compatibility of two OpenAPI documents: can a client written
//! against `old` use a server that implements `new`?
//!
//! The rules (ADR-NNNN) for every operation of `old`:
//! - it still exists (same path template and method);
//! - no new required parameter or request body, and every parameter of
//!   `old` is still accepted with at least the old values;
//! - every success status (`101`, `2xx`) of `old` is still documented and no
//!   new success status appears; new error statuses (`4xx`, `5xx`) are fine:
//!   clients handle unknown errors generically;
//! - response bodies: what was required stays required, types and enums do
//!   not widen (the old client could not read the new values); new optional
//!   fields are fine.
//!
//! Descriptions, summaries, tags and operation ids are not part of the
//! contract.

use std::collections::BTreeSet;

use serde_json::Value;

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Guard against reference cycles.
const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    /// Client → server: the new schema must accept everything the old one did.
    Request,
    /// Server → client: the new schema must not produce what the old one
    /// could not.
    Response,
}

/// Breaking changes from `old` to `new`, one line each; empty when `new` is
/// backward compatible.
pub fn breaking_changes(old: &Value, new: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let empty = serde_json::Map::new();
    let paths = old["paths"].as_object().unwrap_or(&empty);
    for (path, old_item) in paths {
        for method in METHODS {
            let Some(old_op) = old_item.get(method) else {
                continue;
            };
            let at = format!("{} {path}", method.to_uppercase());
            let Some(new_op) = new["paths"].get(path).and_then(|item| item.get(method)) else {
                out.push(format!("{at}: operation removed"));
                continue;
            };
            let docs = Docs { old, new };
            docs.parameters(&at, old_op, new_op, &mut out);
            docs.request_body(&at, old_op, new_op, &mut out);
            docs.responses(&at, old_op, new_op, &mut out);
        }
    }
    out
}

/// A success status: the client acts on it, so the set is part of the
/// contract.
fn is_success(status: &str) -> bool {
    status == "101" || status.starts_with('2')
}

struct Docs<'a> {
    old: &'a Value,
    new: &'a Value,
}

impl Docs<'_> {
    fn parameters(&self, at: &str, old_op: &Value, new_op: &Value, out: &mut Vec<String>) {
        let old_params = params(self.old, old_op);
        let new_params = params(self.new, new_op);
        for ((name, place), new_p) in &new_params {
            let required = new_p["required"].as_bool().unwrap_or(false);
            let old_required = old_params
                .iter()
                .find(|(key, _)| key.0 == *name && key.1 == *place)
                .is_some_and(|(_, p)| p["required"].as_bool().unwrap_or(false));
            if required && !old_required {
                out.push(format!("{at}: new required {place} parameter `{name}`"));
            }
        }
        for ((name, place), old_p) in &old_params {
            match new_params
                .iter()
                .find(|(key, _)| key.0 == *name && key.1 == *place)
            {
                None => out.push(format!("{at}: {place} parameter `{name}` removed")),
                Some((_, new_p)) => self.schema(
                    &format!("{at}: {place} parameter `{name}`"),
                    &old_p["schema"],
                    &new_p["schema"],
                    Dir::Request,
                    0,
                    out,
                ),
            }
        }
    }

    fn request_body(&self, at: &str, old_op: &Value, new_op: &Value, out: &mut Vec<String>) {
        let old_body = resolve(self.old, &old_op["requestBody"]);
        let new_body = resolve(self.new, &new_op["requestBody"]);
        let new_required = new_body["required"].as_bool().unwrap_or(false);
        let old_required = old_body["required"].as_bool().unwrap_or(false);
        if new_body.is_object() && new_required && !old_required {
            out.push(format!("{at}: request body became required"));
        }
        if old_body.is_object() && !new_body.is_object() {
            out.push(format!("{at}: request body removed"));
            return;
        }
        for (media, old_media) in old_body["content"].as_object().into_iter().flatten() {
            match new_body["content"].get(media) {
                None => out.push(format!("{at}: request body {media} no longer accepted")),
                Some(new_media) => self.schema(
                    &format!("{at}: request body"),
                    &old_media["schema"],
                    &new_media["schema"],
                    Dir::Request,
                    0,
                    out,
                ),
            }
        }
    }

    fn responses(&self, at: &str, old_op: &Value, new_op: &Value, out: &mut Vec<String>) {
        let empty = serde_json::Map::new();
        let old_responses = old_op["responses"].as_object().unwrap_or(&empty);
        let new_responses = new_op["responses"].as_object().unwrap_or(&empty);
        for status in new_responses.keys() {
            if is_success(status) && !old_responses.contains_key(status) {
                out.push(format!("{at}: new success status {status}"));
            }
        }
        for (status, old_resp) in old_responses {
            let Some(new_resp) = new_responses.get(status) else {
                if is_success(status) {
                    out.push(format!("{at}: success status {status} removed"));
                }
                continue;
            };
            let old_resp = resolve(self.old, old_resp);
            let new_resp = resolve(self.new, new_resp);
            for (media, new_media) in new_resp["content"].as_object().into_iter().flatten() {
                match old_resp["content"].get(media) {
                    None if old_resp["content"].is_object() => {
                        out.push(format!("{at}: {status} answers with new type {media}"));
                    }
                    None => out.push(format!("{at}: {status} got a body ({media})")),
                    Some(old_media) => self.schema(
                        &format!("{at}: {status}"),
                        &old_media["schema"],
                        &new_media["schema"],
                        Dir::Response,
                        0,
                        out,
                    ),
                }
            }
            for media in old_resp["content"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|m| m.0)
            {
                if new_resp["content"].get(media).is_none() {
                    out.push(format!("{at}: {status} no longer answers with {media}"));
                }
            }
        }
    }

    /// Compares two schemas at the same place of the contract.
    fn schema(
        &self,
        at: &str,
        old: &Value,
        new: &Value,
        dir: Dir,
        depth: usize,
        out: &mut Vec<String>,
    ) {
        if depth > MAX_DEPTH || old.is_null() {
            return;
        }
        let old = resolve(self.old, old);
        let new = resolve(self.new, new);
        // `narrow` must not allow anything `wide` does not.
        let (narrow, wide) = match dir {
            Dir::Request => (old, new),
            Dir::Response => (new, old),
        };
        if let Some(wide_types) = types(wide) {
            match types(narrow) {
                None => out.push(format!("{at}: type is no longer restricted")),
                Some(narrow_types) => {
                    let extra: Vec<_> = narrow_types
                        .iter()
                        .filter(|t| !wide_types.contains(*t))
                        .filter(|t| !(*t == "integer" && wide_types.contains("number")))
                        .collect();
                    if !extra.is_empty() {
                        out.push(format!(
                            "{at}: type {extra:?} is not accepted on the other side"
                        ));
                    }
                }
            }
        }
        if let Some(wide_enum) = wide["enum"].as_array() {
            match narrow["enum"].as_array() {
                None => out.push(format!("{at}: enum constraint removed")),
                Some(narrow_enum) => {
                    let extra: Vec<_> = narrow_enum
                        .iter()
                        .filter(|v| !wide_enum.contains(v))
                        .collect();
                    if !extra.is_empty() {
                        out.push(format!("{at}: enum values {extra:?} are new"));
                    }
                }
            }
        }
        let old_required = required(old);
        let new_required = required(new);
        match dir {
            Dir::Response => {
                for name in old_required.difference(&new_required) {
                    out.push(format!("{at}: field `{name}` is no longer required"));
                }
            }
            Dir::Request => {
                for name in new_required.difference(&old_required) {
                    out.push(format!("{at}: field `{name}` became required"));
                }
            }
        }
        for (name, old_prop) in old["properties"].as_object().into_iter().flatten() {
            if let Some(new_prop) = new["properties"].get(name) {
                self.schema(
                    &format!("{at}.{name}"),
                    old_prop,
                    new_prop,
                    dir,
                    depth + 1,
                    out,
                );
            }
        }
        if old["items"].is_object() {
            self.schema(
                &format!("{at}[]"),
                &old["items"],
                &new["items"],
                dir,
                depth + 1,
                out,
            );
        }
    }
}

/// Parameters of an operation by (name, location).
fn params<'a>(doc: &'a Value, op: &'a Value) -> Vec<((String, String), &'a Value)> {
    op["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| resolve(doc, p))
        .filter_map(|p| {
            let name = p["name"].as_str()?.to_owned();
            let place = p["in"].as_str()?.to_owned();
            Some(((name, place), p))
        })
        .collect()
}

/// Follows local `$ref`s (`#/components/...`).
pub fn resolve<'a>(doc: &'a Value, value: &'a Value) -> &'a Value {
    let mut value = value;
    for _ in 0..MAX_DEPTH {
        let Some(pointer) = value["$ref"].as_str().and_then(|r| r.strip_prefix('#')) else {
            break;
        };
        value = doc.pointer(pointer).unwrap_or(&Value::Null);
    }
    value
}

/// The allowed JSON types; `None` when the schema does not restrict them.
fn types(schema: &Value) -> Option<BTreeSet<String>> {
    match &schema["type"] {
        Value::String(t) => Some(BTreeSet::from([t.clone()])),
        Value::Array(ts) => Some(
            ts.iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    }
}

fn required(schema: &Value) -> BTreeSet<String> {
    schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}
