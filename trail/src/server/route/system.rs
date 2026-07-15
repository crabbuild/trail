use crate::model::{Actor, OperationKind, RecordOptions};
use crate::server::transport::{HttpRequest, HttpResponse, ServerAuth};
use crate::{Error, Result};

use super::utils;

#[derive(serde::Deserialize)]
struct LedgerFenceRequest {
    protocol_version: u16,
    owner_nonce: String,
    workspace_identity: String,
    executable_identity: String,
    scope_id: String,
    expected_epoch: u64,
}

pub(super) fn handle_system_route(
    db: &mut crate::Trail,
    request: &HttpRequest,
    auth: &ServerAuth,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/openapi.json" {
        return Ok(Some(utils::json_response(
            200,
            "OK",
            &super::super::openapi::openapi_spec(),
        )?));
    }

    if request.method == "GET" && path == "/v1/doctor" {
        let report = db.doctor()?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/status" {
        let report = match db.status(None) {
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                db.status(None)?
            }
            result => result?,
        };
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST"
        && matches!(
            path,
            "/v1/ledger/challenge" | "/v1/ledger/fence" | "/v1/ledger/reconcile"
        )
    {
        let body: LedgerFenceRequest = serde_json::from_slice(&request.body)?;
        let expected_identity = auth.daemon_identity.as_ref();
        if body.protocol_version != 2
            || expected_identity.is_none_or(|identity| {
                identity.owner_nonce != body.owner_nonce
                    || identity.workspace_identity != body.workspace_identity
                    || identity.executable_identity != body.executable_identity
            })
        {
            return Err(Error::DaemonUnavailable(
                "changed-path ledger RPC identity mismatch".into(),
            ));
        }
        let proof = if path == "/v1/ledger/challenge" {
            let proof = super::super::workspace_changed_path_ready_proof(db)?;
            if proof.scope_id != body.scope_id || proof.epoch != body.expected_epoch {
                return Err(Error::DaemonUnavailable(
                    "changed-path daemon challenge scope or epoch mismatch".into(),
                ));
            }
            proof
        } else if path == "/v1/ledger/reconcile" {
            super::super::workspace_changed_path_reconcile(
                db,
                Some(&body.scope_id),
                Some(body.expected_epoch),
            )?
        } else {
            super::super::workspace_changed_path_fence(
                db,
                Some(&body.scope_id),
                Some(body.expected_epoch),
            )?
        };
        return Ok(Some(utils::json_response(
            200,
            "OK",
            &serde_json::json!({
                "pid": std::process::id(),
                "process_start_identity": expected_identity
                    .map(|identity| identity.process_start_identity.as_str())
                    .unwrap_or_default(),
                "executable_identity": expected_identity
                    .map(|identity| identity.executable_identity.as_str())
                    .unwrap_or_default(),
                "owner_nonce": body.owner_nonce,
                "workspace_identity": body.workspace_identity,
                "scope_id": proof.scope_id,
                "epoch": proof.epoch,
                "live_fence_sequence": proof.sequence,
                "durable_offset": proof.durable_offset,
                "folded_offset": proof.folded_offset,
                "protocol_version": 2
            }),
        )?));
    }

    if request.method == "POST" && path == "/v1/record" {
        use crate::server::request_types::RecordRequest;
        let body: RecordRequest = if request.body.is_empty() {
            RecordRequest {
                ref_name: None,
                message: None,
                paths: Vec::new(),
                kind: None,
                session_id: None,
                allow_ignored: false,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let kind = body.kind.as_deref().map(parse_record_kind).transpose()?;
        let report = db.record_with_options(
            body.ref_name.as_deref(),
            body.message,
            Actor::human(),
            RecordOptions {
                paths: body.paths,
                kind,
                session_id: body.session_id,
                allow_ignored: body.allow_ignored,
            },
        )?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/diff" {
        let patch = utils::query_flag(query, "patch");
        let line_ids = utils::query_line_ids_flag(query);
        let diff = if utils::query_flag(query, "dirty") {
            if utils::query_value(query, "range").is_some()
                || utils::query_value(query, "root").is_some()
            {
                return Err(Error::InvalidInput(
                    "diff accepts only one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            db.diff_dirty(patch, line_ids)?
        } else if let Some(root) = utils::query_value(query, "root") {
            if utils::query_value(query, "range").is_some() {
                return Err(Error::InvalidInput(
                    "diff accepts only one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            db.diff_roots(root, patch, line_ids)?
        } else {
            let range = utils::required_query(query, "range")?;
            db.diff_range_with_options(range, patch, line_ids)?
        };
        return Ok(Some(utils::json_response(200, "OK", &diff)?));
    }

    if request.method == "GET" && path == "/v1/config" {
        let entries = db.config_entries();
        return Ok(Some(utils::json_response(200, "OK", &entries)?));
    }

    if request.method == "POST" && path == "/v1/config" {
        use crate::server::request_types::ConfigSetRequest;
        let body: ConfigSetRequest = serde_json::from_slice(&request.body)?;
        let report = db.config_set(&body.key, &body.value)?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && parts.len() == 3 && parts[0] == "v1" && parts[1] == "config" {
        let entry = db.config_get(parts[2])?;
        return Ok(Some(utils::json_response(200, "OK", &entry)?));
    }

    if request.method == "GET" && path == "/v1/timeline" {
        let limit = utils::query_usize(query, "limit", 30)?;
        let entries = db.timeline_query(
            utils::query_value(query, "branch"),
            utils::query_value(query, "session"),
            utils::query_value(query, "lane"),
            limit,
        )?;
        return Ok(Some(utils::json_response(200, "OK", &entries)?));
    }

    if request.method == "GET" && path == "/v1/why" {
        let at = utils::query_value(query, "at").or_else(|| utils::query_value(query, "branch"));
        let result = match (
            utils::query_value(query, "path_line"),
            utils::query_value(query, "line_id"),
        ) {
            (Some(path_line), None) => db.why(path_line, at)?,
            (None, Some(line_id)) => db.why_line_id(line_id, at)?,
            (Some(_), Some(_)) => {
                return Err(Error::InvalidInput(
                    "why accepts either `path_line` or `line_id`, not both".to_string(),
                ));
            }
            (None, None) => {
                return Err(Error::InvalidInput(
                    "why requires `path_line` or `line_id`".to_string(),
                ));
            }
        };
        return Ok(Some(utils::json_response(200, "OK", &result)?));
    }

    if request.method == "GET" && path == "/v1/history" {
        let result = if let Some(line_id) = utils::query_value(query, "line_id") {
            db.history_for_line_id(line_id)?
        } else if let Some(file_id) = utils::query_value(query, "file_id") {
            db.history_for_file_id(file_id)?
        } else {
            let selector = utils::query_value(query, "path")
                .or_else(|| utils::query_value(query, "selector"))
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "history requires `path`, `selector`, `file_id`, or `line_id`".to_string(),
                    )
                })?;
            db.history_for_path(selector)?
        };
        return Ok(Some(utils::json_response(200, "OK", &result)?));
    }

    if request.method == "GET" && path == "/v1/code-from" {
        let selector = utils::required_query(query, "selector")?;
        let result = db.code_from(selector)?;
        return Ok(Some(utils::json_response(200, "OK", &result)?));
    }

    if request.method == "GET" && path == "/v1/ignore" {
        let report = db.ignore_list()?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST" && path == "/v1/ignore/patterns" {
        use crate::server::request_types::IgnorePatternRequest;
        let body: IgnorePatternRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_add(&body.pattern)?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "DELETE" && path == "/v1/ignore/patterns" {
        use crate::server::request_types::IgnorePatternRequest;
        let body: IgnorePatternRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_remove(&body.pattern)?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST" && path == "/v1/ignore/check" {
        use crate::server::request_types::IgnoreCheckRequest;
        let body: IgnoreCheckRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_check(&body.path)?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST" && path == "/v1/guardrails/check" {
        use crate::server::request_types::GuardrailCheckRequest;
        let body: GuardrailCheckRequest = serde_json::from_slice(&request.body)?;
        let report = db.guardrail_check(
            body.lane.as_deref(),
            &body.action,
            body.summary.as_deref(),
            body.payload,
            &body.paths,
        )?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/cache/layers" {
        let report = db.list_workspace_layers()?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "GET"
        && parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "cache"
        && parts[2] == "layers"
    {
        let report = db.verify_workspace_layer(parts[3])?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST"
        && parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "cache"
        && parts[2] == "layers"
        && parts[4] == "verify"
    {
        utils::reject_unexpected_body(request, "POST /v1/cache/layers/{layer}/verify")?;
        let report = db.verify_workspace_layer(parts[3])?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    if request.method == "POST" && path == "/v1/cache/gc" {
        use crate::server::request_types::CacheGcRequest;
        let body: CacheGcRequest = if request.body.is_empty() {
            CacheGcRequest {
                dry_run: false,
                retention_secs: None,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.workspace_cache_gc(body.dry_run, body.retention_secs)?;
        return Ok(Some(utils::json_response(200, "OK", &report)?));
    }

    Ok(None)
}

fn parse_record_kind(value: &str) -> Result<OperationKind> {
    match value {
        "file-edit" => Ok(OperationKind::FileEdit),
        "multi-file-edit" => Ok(OperationKind::MultiFileEdit),
        "format" => Ok(OperationKind::Format),
        "manual-checkpoint" => Ok(OperationKind::ManualCheckpoint),
        "manual-record" => Ok(OperationKind::ManualRecord),
        other => Err(Error::InvalidInput(format!(
            "record kind must be file-edit, multi-file-edit, format, manual-checkpoint, or manual-record, got `{other}`"
        ))),
    }
}
