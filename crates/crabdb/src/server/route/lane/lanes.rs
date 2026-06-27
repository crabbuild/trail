use std::path::PathBuf;

use crate::model::LaneGateOptions;
use crate::server::request_types::{
    LaneClaimRequest, LaneReadFileRequest, LaneRecordRequest, LaneRewindRequest, LaneTestRequest,
    SpawnLaneRequest, SyncWorkdirRequest,
};
use crate::server::route::utils::{
    json_response, parse_patch_request, query_flag, query_line_ids_flag, query_usize, query_value,
    reject_unexpected_body,
};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Error, Result};

pub(super) fn handle_lane_resources(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/lanes" {
        let lanes = db.list_lanes()?;
        return Ok(Some(json_response(200, "OK", &lanes)?));
    }

    if request.method == "POST" && path == "/v1/lanes" {
        let body: SpawnLaneRequest = serde_json::from_slice(&request.body)?;
        let materialize = if body.workdir.is_some() || !body.paths.is_empty() {
            body.materialize.unwrap_or(true)
        } else {
            body.materialize
                .unwrap_or(db.default_lane_materialize_for_ref(body.from.as_deref())?)
        };
        let report = db.spawn_lane_with_workdir_paths_and_neighbors(
            &body.name,
            body.from.as_deref(),
            materialize,
            body.provider,
            body.model,
            body.workdir.map(PathBuf::from),
            &body.paths,
            body.include_neighbors,
        )?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "lanes" {
        let lane = db.resolve_lane_handle(parts[2])?;
        if request.method == "GET" {
            let details = db.lane_details(&lane)?;
            return Ok(Some(json_response(200, "OK", &details)?));
        }
        if request.method == "DELETE" {
            reject_unexpected_body(request, "DELETE /v1/lanes/{lane_or_id}")?;
            let report = db.remove_lane(&lane, query_flag(query, "force"))?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "claims"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body: LaneClaimRequest = serde_json::from_slice(&request.body)?;
        let report = db.claim_lane_path(&lane, &body.path, body.ttl_secs.unwrap_or(600))?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4 && parts[0] == "v1" && parts[1] == "lanes" && request.method == "GET" {
        let lane = db.resolve_lane_handle(parts[2])?;
        return Ok(Some(match parts[3] {
            "status" => {
                let report = db.lane_status(&lane)?;
                json_response(200, "OK", &report)?
            }
            "review" => {
                let report = db.lane_review_packet(&lane, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)?
            }
            "contribution" => {
                let report = db.lane_contribution(&lane, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)?
            }
            "gates" => {
                let report = db.lane_gate_history(
                    &lane,
                    query_value(query, "kind"),
                    query_usize(query, "limit", 50)?,
                )?;
                json_response(200, "OK", &report)?
            }
            "readiness" => {
                let report = db.lane_readiness(&lane)?;
                json_response(200, "OK", &report)?
            }
            "refresh-preview" => {
                let target = query_value(query, "target").unwrap_or("main");
                let report = db.preview_lane_refresh(&lane, target)?;
                json_response(200, "OK", &report)?
            }
            "handoff" => {
                let report = db.lane_handoff(&lane, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)?
            }
            "workdir" => {
                let report = db.lane_workdir(&lane)?;
                json_response(200, "OK", &report)?
            }
            "diff" => {
                let diff = db.diff_lane_with_options(
                    &lane,
                    query_flag(query, "patch"),
                    query_line_ids_flag(query),
                )?;
                json_response(200, "OK", &diff)?
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "unknown API endpoint `{}`",
                    request.path
                )))
            }
        }));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "record"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body = if request.body.is_empty() {
            LaneRecordRequest {
                message: None,
                preview: false,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        if body.preview {
            let report = db.preview_lane_workdir_record(&lane)?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
        let report = db.record_lane_workdir(&lane, body.message)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "rewind"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body: LaneRewindRequest = serde_json::from_slice(&request.body)?;
        let report = db.rewind_lane(&lane, &body.to, body.record_current, body.sync_workdir)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "read-file"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body: LaneReadFileRequest = serde_json::from_slice(&request.body)?;
        let report = db.read_lane_file_with_hydration(
            &lane,
            &body.path,
            body.hydrate,
            body.force,
            body.include_neighbors,
        )?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "sync-workdir"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body: SyncWorkdirRequest = if request.body.is_empty() {
            SyncWorkdirRequest {
                force: false,
                paths: Vec::new(),
                include_neighbors: false,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.sync_lane_workdir_with_paths_and_neighbors(
            &lane,
            body.force,
            &body.paths,
            body.include_neighbors,
        )?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && (parts[3] == "tests" || parts[3] == "evals")
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let body: LaneTestRequest = serde_json::from_slice(&request.body)?;
        let options = LaneGateOptions {
            suite: body.suite,
            score: body.score,
            threshold: body.threshold,
        };
        let report = if parts[3] == "evals" {
            db.run_lane_eval_with_options(
                &lane,
                body.command,
                body.turn_id.as_deref(),
                body.timeout_secs.unwrap_or(600),
                options,
            )?
        } else {
            db.run_lane_test_with_options(
                &lane,
                body.command,
                body.turn_id.as_deref(),
                body.timeout_secs.unwrap_or(600),
                options,
            )?
        };
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "patches"
        && request.method == "POST"
    {
        let lane = db.resolve_lane_handle(parts[2])?;
        let patch = parse_patch_request(&request.body)?;
        let report = db.apply_lane_patch(&lane, patch)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    Ok(None)
}
