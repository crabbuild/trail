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

pub(crate) fn parse_patch_request(body: &[u8]) -> Result<PatchDocument> {
    let request: ApiPatchRequest = serde_json::from_slice(body)?;
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
        edits,
    })
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
        body: serde_json::to_vec(value)?,
    })
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
        Error::InvalidInput(_) | Error::InvalidPath { .. } | Error::IgnoredPath(_) => 400,
        _ => 500,
    };
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Internal Server Error",
    };
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
        body,
    }
}
