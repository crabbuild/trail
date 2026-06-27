use crate::model::validate_external_patch_edit_sources;
use crate::model::ConflictResolveReport;
use crate::server::request_types::{
    ApiPatchFile, ApiPatchRequest, ApiTextEdit, ConflictResolveRequest,
};
use crate::server::transport::HttpRequest;
use crate::server::transport::{HttpResponse, ServerAuth};
use crate::{Error, PatchDocument, PatchEdit, Result};
use serde::Serialize;

pub(crate) fn resolve_conflict_request(
    db: &mut crate::CrabDb,
    conflict_set_id: &str,
    body: ConflictResolveRequest,
) -> Result<ConflictResolveReport> {
    match (body.take, body.manual) {
        (Some(take), None) => db.resolve_conflict(conflict_set_id, &take),
        (None, Some(manual)) => db.resolve_conflict_manual(conflict_set_id, manual),
        (Some(_), Some(_)) => Err(Error::InvalidInput(
            "conflict resolve request must include only one of `take` or `manual`".to_string(),
        )),
        (None, None) => Err(Error::InvalidInput(
            "conflict resolve request requires `take` or `manual`".to_string(),
        )),
    }
}

pub(crate) fn reject_unexpected_body(request: &HttpRequest, endpoint: &str) -> Result<()> {
    if request.body.is_empty() {
        return Ok(());
    }
    Err(Error::InvalidInput(format!(
        "{endpoint} does not accept a request body"
    )))
}

pub(crate) fn parse_patch_request(body: &[u8]) -> Result<PatchDocument> {
    let request: ApiPatchRequest = serde_json::from_slice(body)?;
    validate_external_patch_edit_sources(
        "patch request",
        request.edits.len(),
        request.files.len(),
    )?;
    let mut edits = request.edits;
    for file in request.files {
        match file {
            ApiPatchFile::AddText {
                path,
                content,
                executable,
            } => edits.push(PatchEdit::Write {
                path,
                content,
                executable,
            }),
            ApiPatchFile::ModifyText {
                path,
                edits: file_edits,
            } => {
                for edit in file_edits {
                    match edit {
                        ApiTextEdit::ModifyLine {
                            line_id,
                            expected_text,
                            new_text,
                        } => edits.push(PatchEdit::ReplaceLine {
                            path: path.clone(),
                            line_id,
                            expected_text,
                            new_text,
                        }),
                    }
                }
            }
            ApiPatchFile::WriteBytes {
                path,
                bytes_hex,
                executable,
            } => edits.push(PatchEdit::WriteBytes {
                path,
                bytes_hex,
                executable,
            }),
            ApiPatchFile::Delete { path } => edits.push(PatchEdit::Delete { path }),
            ApiPatchFile::Rename { from, to } => edits.push(PatchEdit::Rename { from, to }),
        }
    }
    Ok(PatchDocument {
        base_change: request.base_change,
        message: request.message,
        session_id: request.session_id,
        allow_ignored: request.allow_ignored,
        allow_stale: request.allow_stale,
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_patch_request_rejects_empty_or_ambiguous_edit_sources() {
        let empty = parse_patch_request(br#"{"message":"empty"}"#).unwrap_err();
        assert!(empty
            .to_string()
            .contains("requires at least one edit in `edits` or `files`"));

        let ambiguous = parse_patch_request(
            br#"{
                "message":"ambiguous",
                "edits":[{"op":"delete","path":"old.md"}],
                "files":[{"type":"delete","path":"new.md"}]
            }"#,
        )
        .unwrap_err();
        assert!(ambiguous
            .to_string()
            .contains("must use either `edits` or `files`, not both"));
    }

    #[test]
    fn parse_patch_request_fuzz_corpus_preserves_adapter_invariants() {
        for seed in 0..256_u64 {
            let value = generated_api_patch_json(seed);
            let body = serde_json::to_vec(&value).unwrap();
            match parse_patch_request(&body) {
                Ok(document) => {
                    let edits_len = value
                        .get("edits")
                        .and_then(serde_json::Value::as_array)
                        .map_or(0, Vec::len);
                    let files_len = value
                        .get("files")
                        .and_then(serde_json::Value::as_array)
                        .map_or(0, Vec::len);
                    assert_ne!(edits_len + files_len, 0, "seed {seed}");
                    assert!(
                        (edits_len == 0) ^ (files_len == 0),
                        "seed {seed} accepted mixed edit sources"
                    );
                    assert!(!document.edits.is_empty(), "seed {seed}");
                }
                Err(err) => {
                    let message = err.to_string();
                    assert!(
                        message.contains("unknown field")
                            || message.contains("unknown variant")
                            || message.contains("missing field")
                            || message.contains("invalid type")
                            || message.contains("requires at least one edit")
                            || message.contains("must use either `edits` or `files`"),
                        "unexpected parse error for seed {seed}: {message}"
                    );
                }
            }
        }
    }

    #[test]
    fn local_loopback_origin_accepts_well_formed_loopback_hosts() {
        for origin in [
            "http://localhost",
            "https://localhost:8765",
            "http://127.0.0.1:8765",
            "http://[::1]:8765",
        ] {
            assert!(local_loopback_origin(origin), "{origin}");
        }
    }

    #[test]
    fn local_loopback_origin_rejects_malformed_or_non_loopback_hosts() {
        for origin in [
            "null",
            "https://example.com",
            "https://user@localhost",
            "https://localhost/path",
            "http://localhost:999999",
            "http:// localhost",
            "http://localhost :8765",
            "http://127.0.0.1 :8765",
            "http://[ ::1]:8765",
        ] {
            assert!(!local_loopback_origin(origin), "{origin}");
        }
    }

    #[test]
    fn local_loopback_host_accepts_well_formed_loopback_hosts() {
        for host in [
            "localhost",
            "localhost.",
            "localhost:8765",
            "127.0.0.1:8765",
            "[::1]:8765",
        ] {
            assert!(local_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn local_loopback_host_rejects_malformed_or_non_loopback_hosts() {
        for host in [
            "",
            "example.com",
            "localhost:999999",
            " localhost",
            "localhost :8765",
            "127.0.0.1 :8765",
            "127.0.0.1/path",
            "user@localhost",
            "[ ::1]:8765",
        ] {
            assert!(!local_loopback_host(host), "{host}");
        }
    }

    fn generated_api_patch_json(seed: u64) -> serde_json::Value {
        let path = format!("docs/generated-{seed}.md");
        let native_edit = serde_json::json!({
            "op": "write",
            "path": path,
            "content": format!("seed-{seed}\n")
        });
        let api_file = serde_json::json!({
            "type": "modify_text",
            "path": "README.md",
            "edits": [{
                "type": "modify_line",
                "line_id": format!("ch_seed_{seed}:1"),
                "expected_text": format!("old-{seed}"),
                "new_text": format!("new-{seed}")
            }]
        });
        match seed % 8 {
            0 => serde_json::json!({ "message": "native", "edits": [native_edit] }),
            1 => serde_json::json!({ "message": "files", "files": [api_file] }),
            2 => {
                serde_json::json!({ "message": "ambiguous", "edits": [native_edit], "files": [api_file] })
            }
            3 => serde_json::json!({ "message": "empty" }),
            4 => serde_json::json!({ "message": "unknown", "files": [api_file], "surprise": true }),
            5 => serde_json::json!({
                "message": "nested unknown",
                "files": [{
                    "type": "add_text",
                    "path": "docs/nested.md",
                    "content": "ok\n",
                    "surprise": true
                }]
            }),
            6 => serde_json::json!({
                "message": "bad edit type",
                "edits": [{
                    "op": "replace_line",
                    "path": "README.md",
                    "line_id": 42,
                    "expected_text": "old",
                    "new_text": "new"
                }]
            }),
            _ => serde_json::json!({
                "message": "bad file variant",
                "files": [{ "type": "surprise", "path": "README.md" }]
            }),
        }
    }
}

pub(crate) fn query_flag(query: &str, key: &str) -> bool {
    query.split('&').any(|part| {
        let Some((candidate, value)) = part.split_once('=') else {
            return part == key;
        };
        candidate == key && matches!(value, "1" | "true" | "yes")
    })
}

pub(crate) fn query_line_ids_flag(query: &str) -> bool {
    query_flag(query, "show_line_ids") || query_flag(query, "show-line-ids")
}

pub(crate) fn validate_merge_strategy(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    match value {
        "conservative" | "line-id-aware" | "line_id_aware" => Ok(()),
        other => Err(Error::InvalidInput(format!(
            "merge strategy must be conservative, line-id-aware, or line_id_aware, got `{other}`"
        ))),
    }
}

pub(crate) fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=')?;
        (candidate == key && !value.is_empty()).then_some(value)
    })
}

pub(crate) fn required_query<'a>(query: &'a str, key: &str) -> Result<&'a str> {
    query_value(query, key)
        .ok_or_else(|| Error::InvalidInput(format!("missing `{key}` query value")))
}

pub(crate) fn query_usize(query: &str, key: &str, default: usize) -> Result<usize> {
    let Some(value) = query_value(query, key) else {
        return Ok(default);
    };
    value
        .parse()
        .map_err(|_| Error::InvalidInput(format!("invalid `{key}` query value `{value}`")))
}

pub(crate) fn json_response<T: Serialize>(
    status: u16,
    reason: &'static str,
    value: &T,
) -> Result<HttpResponse> {
    Ok(HttpResponse {
        status,
        reason,
        extra_headers: Vec::new(),
        body: serde_json::to_vec(value)?,
    })
}

pub(crate) fn reason_for_status(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

pub(crate) fn authorized(request: &HttpRequest, auth: &ServerAuth) -> bool {
    let Some(expected) = auth.token.as_deref() else {
        return true;
    };
    if let Some(value) = request.headers.get("authorization") {
        if let Some((scheme, token)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer")
                && constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
            {
                return true;
            }
        }
    }
    request
        .headers
        .get("x-crabdb-token")
        .is_some_and(|token| constant_time_eq(token.trim().as_bytes(), expected.as_bytes()))
}

pub(crate) fn origin_allowed(request: &HttpRequest) -> bool {
    let Some(origin) = request.headers.get("origin") else {
        return true;
    };
    local_loopback_origin(origin.trim())
}

pub(crate) fn host_allowed(request: &HttpRequest) -> bool {
    let Some(host) = request.headers.get("host") else {
        return false;
    };
    local_loopback_host(host.trim())
}

fn local_loopback_origin(origin: &str) -> bool {
    if origin.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    local_loopback_host(rest)
}

fn local_loopback_host(host: &str) -> bool {
    if host.chars().any(char::is_whitespace)
        || host.is_empty()
        || host.contains('/')
        || host.contains('@')
    {
        return false;
    }
    let Some((host, port)) = split_origin_host_port(host) else {
        return false;
    };
    if !valid_optional_port(port) {
        return false;
    }
    let host = host.trim().trim_end_matches('.');
    if host.is_empty() {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|addr| addr.is_loopback())
}

fn split_origin_host_port(value: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':')?)
        };
        return Some((host, port));
    }
    if value.matches(':').count() > 1 {
        return None;
    }
    match value.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((value, None)),
    }
}

fn valid_optional_port(port: Option<&str>) -> bool {
    let Some(port) = port else {
        return true;
    };
    !port.is_empty() && port.parse::<u16>().is_ok()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let l = left.get(idx).copied().unwrap_or(0);
        let r = right.get(idx).copied().unwrap_or(0);
        diff |= (l ^ r) as usize;
    }
    diff == 0
}

pub(crate) fn error_response(err: &Error) -> HttpResponse {
    let status = match err {
        Error::RefNotFound(_) | Error::OperationNotFound(_) | Error::RootNotFound(_) => 404,
        Error::Conflict(_)
        | Error::DirtyWorktree
        | Error::DirtyWorktreeWithMessage(_)
        | Error::PatchRejected(_)
        | Error::StaleBranch(_)
        | Error::WorkspaceLocked(_) => 409,
        Error::InvalidInput(_)
        | Error::InvalidPath { .. }
        | Error::IgnoredPath(_)
        | Error::Json(_) => 400,
        _ => 500,
    };
    let reason = reason_for_status(status);
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: err.to_string(),
            code: err.exit_code(),
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"serialization failed\",\"code\":1}}".to_vec());
    HttpResponse {
        status,
        reason,
        extra_headers: Vec::new(),
        body,
    }
}

#[derive(Serialize)]
pub(crate) struct ErrorBody {
    error: ErrorDetails,
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorDetails {
    pub(crate) message: String,
    pub(crate) code: i32,
}

pub(crate) fn unauthorized_response() -> HttpResponse {
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: "unauthorized: missing or invalid CrabDB daemon token".to_string(),
            code: 11,
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"unauthorized\",\"code\":11}}".to_vec());
    HttpResponse {
        status: 401,
        reason: "Unauthorized",
        extra_headers: Vec::new(),
        body,
    }
}

pub(crate) fn forbidden_origin_response() -> HttpResponse {
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: "forbidden: request origin is not a local loopback origin".to_string(),
            code: 11,
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"forbidden\",\"code\":11}}".to_vec());
    HttpResponse {
        status: 403,
        reason: "Forbidden",
        extra_headers: Vec::new(),
        body,
    }
}

pub(crate) fn forbidden_host_response() -> HttpResponse {
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: "forbidden: request host is missing or is not a local loopback host"
                .to_string(),
            code: 11,
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"forbidden\",\"code\":11}}".to_vec());
    HttpResponse {
        status: 403,
        reason: "Forbidden",
        extra_headers: Vec::new(),
        body,
    }
}
