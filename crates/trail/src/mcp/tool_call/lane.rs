use std::path::PathBuf;

use serde_json::Value;

use crate::{LaneGateOptions, Result, Trail};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut Trail, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "trail.lane_spawn" => {
            let args: LaneSpawnArgs = parse_args(arguments)?;
            let workdir_mode = db.resolve_lane_spawn_workdir_mode(
                args.from_ref.as_deref(),
                args.workdir_mode.as_deref(),
                args.materialize,
                false,
                args.workdir.is_some(),
                &args.paths,
            )?;
            tool_result(db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                &args.name,
                args.from_ref.as_deref(),
                workdir_mode,
                args.provider,
                args.model,
                args.workdir.map(PathBuf::from),
                &args.paths,
                args.include_neighbors,
            )?)
        }
        "trail.lane_claim" => {
            let args: LaneClaimArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.claim_lane_path(&lane, &args.path, args.ttl_secs.unwrap_or(600))?)
        }
        "trail.lane_list" => tool_result(db.list_lanes()?),
        "trail.lane_show" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_details(&lane)?)
        }
        "trail.lane_status" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_status(&lane)?)
        }
        "trail.lane_review" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_review_packet(&lane, args.limit.unwrap_or(50))?)
        }
        "trail.lane_contribution" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_contribution(&lane, args.limit.unwrap_or(50))?)
        }
        "trail.lane_readiness" => {
            let args: LaneHandleArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_readiness(&lane)?)
        }
        "trail.lane_refresh_preview" => {
            let args: LaneRefreshPreviewArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.preview_lane_refresh(&lane, args.target.as_deref().unwrap_or("main"))?)
        }
        "trail.lane_handoff" => {
            let args: LaneContributionArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.lane_handoff(&lane, args.limit.unwrap_or(50))?)
        }
        "trail.lane_remove" => {
            let args: LaneRemoveArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.remove_lane(&lane, args.force)?)
        }
        "trail.lane_rewind" => {
            let args: LaneRewindArgs = parse_args(arguments)?;
            let lane = db.resolve_lane_handle(&args.lane)?;
            tool_result(db.rewind_lane(&lane, &args.to, args.record_current, args.sync_workdir)?)
        }
        "trail.lease_acquire" => {
            let args: LeaseAcquireArgs = parse_args(arguments)?;
            let mode = args.mode.unwrap_or_else(default_lease_mode);
            tool_result(db.acquire_lease(
                &args.lane,
                args.path.as_deref(),
                &mode,
                args.ttl_secs.unwrap_or(3600),
            )?)
        }
        "trail.lease_list" => {
            let args: LeaseListArgs = parse_args(arguments)?;
            tool_result(db.list_leases(args.all)?)
        }
        "trail.lease_release" => {
            let args: LeaseReleaseArgs = parse_args(arguments)?;
            tool_result(db.release_lease(&args.lease_id)?)
        }
        "trail.diff_lane" => {
            let args: DiffLaneArgs = parse_args(arguments)?;
            tool_result(db.diff_lane_with_options(&args.lane, args.patch, args.show_line_ids)?)
        }
        "trail.gate_history" => {
            let args: GateHistoryArgs = parse_args(arguments)?;
            tool_result(db.lane_gate_history(
                &args.lane,
                args.kind.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "trail.run_test" => {
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
        "trail.run_eval" => {
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
        "trail.read_file" => {
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
        "trail.sync_workdir" => {
            let args: SyncWorkdirArgs = parse_args(arguments)?;
            tool_result(db.sync_lane_workdir_with_paths_and_neighbors(
                &args.lane,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?)
        }
        "trail.lane_hydrate" => {
            let args: SyncWorkdirArgs = parse_args(arguments)?;
            if args.paths.is_empty() {
                return Err(crate::Error::InvalidInput(
                    "lane hydrate requires at least one path".to_string(),
                ));
            }
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
