use std::path::PathBuf;

use serde_json::Value;

use crate::{CrabDb, LaneGateOptions, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.lane_spawn" => {
            let args: LaneSpawnArgs = parse_args(arguments)?;
            let materialize = if args.workdir.is_some() || !args.paths.is_empty() {
                args.materialize.unwrap_or(true)
            } else {
                args.materialize
                    .unwrap_or(db.default_lane_materialize_for_ref(args.from_ref.as_deref())?)
            };
            tool_result(db.spawn_lane_with_workdir_paths_and_neighbors(
                &args.name,
                args.from_ref.as_deref(),
                materialize,
                args.provider,
                args.model,
                args.workdir.map(PathBuf::from),
                &args.paths,
                args.include_neighbors,
            )?)
        }
        "crabdb.lane_claim" => {
            let args: LaneClaimArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.claim_lane_path(&lane, &args.path, args.ttl_secs.unwrap_or(600))?)
        }
        "crabdb.lane_list" => tool_result(db.list_lanes()?),
        "crabdb.lane_show" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_details(&lane)?)
        }
        "crabdb.lane_status" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_status(&lane)?)
        }
        "crabdb.lane_review" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_review_packet(&lane, args.limit.unwrap_or(50))?)
        }
        "crabdb.lane_contribution" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_contribution(&lane, args.limit.unwrap_or(50))?)
        }
        "crabdb.lane_readiness" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_readiness(&lane)?)
        }
        "crabdb.lane_refresh_preview" => {
            let args: LaneRefreshPreviewArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.preview_lane_refresh(&lane, args.target.as_deref().unwrap_or("main"))?)
        }
        "crabdb.lane_handoff" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_handoff(&lane, args.limit.unwrap_or(50))?)
        }
        "crabdb.lane_remove" => {
            let args: LaneRemoveArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.remove_lane(&lane, args.force)?)
        }
        "crabdb.lane_rewind" => {
            let args: LaneRewindArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.rewind_lane(&lane, &args.to, args.record_current, args.sync_workdir)?)
        }
        "crabdb.lease_acquire" => {
            let args: LeaseAcquireArgs = parse_args(arguments)?;
            let mode = args.mode.unwrap_or_else(default_lease_mode);
            tool_result(db.acquire_lease(
                &args.lane,
                args.path.as_deref(),
                &mode,
                args.ttl_secs.unwrap_or(3600),
            )?)
        }
        "crabdb.lease_list" => {
            let args: LeaseListArgs = parse_args(arguments)?;
            tool_result(db.list_leases(args.all)?)
        }
        "crabdb.lease_release" => {
            let args: LeaseReleaseArgs = parse_args(arguments)?;
            tool_result(db.release_lease(&args.lease_id)?)
        }
        "crabdb.diff_lane" => {
            let args: DiffLaneArgs = parse_args(arguments)?;
            tool_result(db.diff_lane_with_options(&args.lane, args.patch, args.show_line_ids)?)
        }
        "crabdb.gate_history" => {
            let args: GateHistoryArgs = parse_args(arguments)?;
            tool_result(db.lane_gate_history(
                &args.lane,
                args.kind.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.run_test" => {
            let args: RunTestArgs = parse_args(arguments)?;
            let options = LaneGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_lane_test_with_options(
                &args.lane,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.run_eval" => {
            let args: RunTestArgs = parse_args(arguments)?;
            let options = LaneGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_lane_eval_with_options(
                &args.lane,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.read_file" => {
            let args: ReadFileArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.read_lane_file_with_hydration(
                &lane,
                &args.path,
                args.hydrate,
                args.force,
                args.include_neighbors,
            )?)
        }
        "crabdb.sync_workdir" => {
            let args: SyncWorkdirArgs = parse_args(arguments)?;
            tool_result(db.sync_lane_workdir_with_paths_and_neighbors(
                &args.lane,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
