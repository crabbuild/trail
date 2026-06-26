use std::path::PathBuf;

use serde_json::Value;

use crate::{AgentGateOptions, CrabDb, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.agent_spawn" => {
            let args: AgentSpawnArgs = parse_args(arguments)?;
            let materialize = if args.workdir.is_some() || !args.paths.is_empty() {
                args.materialize.unwrap_or(true)
            } else {
                args.materialize
                    .unwrap_or(db.default_agent_materialize_for_ref(args.from_ref.as_deref())?)
            };
            tool_result(db.spawn_agent_with_workdir_paths_and_neighbors(
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
        "crabdb.agent_claim" => {
            let args: AgentClaimArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.claim_agent_path(&agent, &args.path, args.ttl_secs.unwrap_or(600))?)
        }
        "crabdb.agent_list" => tool_result(db.list_agents()?),
        "crabdb.agent_show" => {
            let args: AgentHandleArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_details(&agent)?)
        }
        "crabdb.agent_status" => {
            let args: AgentHandleArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_status(&agent)?)
        }
        "crabdb.agent_review" => {
            let args: AgentContributionArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_review_packet(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_contribution" => {
            let args: AgentContributionArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_contribution(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_readiness" => {
            let args: AgentHandleArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_readiness(&agent)?)
        }
        "crabdb.agent_handoff" => {
            let args: AgentContributionArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_handoff(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_remove" => {
            let args: AgentRemoveArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.remove_agent(&agent, args.force)?)
        }
        "crabdb.agent_rewind" => {
            let args: AgentRewindArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.rewind_agent(
                &agent,
                &args.to,
                args.record_current,
                args.sync_workdir,
            )?)
        }
        "crabdb.lease_acquire" => {
            let args: LeaseAcquireArgs = parse_args(arguments)?;
            let mode = args.mode.unwrap_or_else(default_lease_mode);
            tool_result(db.acquire_lease(
                &args.agent,
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
        "crabdb.diff_agent" => {
            let args: DiffAgentArgs = parse_args(arguments)?;
            tool_result(db.diff_agent_with_options(&args.agent, args.patch, args.show_line_ids)?)
        }
        "crabdb.gate_history" => {
            let args: GateHistoryArgs = parse_args(arguments)?;
            tool_result(db.agent_gate_history(
                &args.agent,
                args.kind.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.run_test" => {
            let args: RunTestArgs = parse_args(arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_test_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.run_eval" => {
            let args: RunTestArgs = parse_args(arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_eval_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.read_file" => {
            let args: ReadFileArgs = parse_args(arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.read_agent_file_with_hydration(
                &agent,
                &args.path,
                args.hydrate,
                args.force,
                args.include_neighbors,
            )?)
        }
        "crabdb.sync_workdir" => {
            let args: SyncWorkdirArgs = parse_args(arguments)?;
            tool_result(db.sync_agent_workdir_with_paths_and_neighbors(
                &args.agent,
                args.force,
                &args.paths,
                args.include_neighbors,
            )?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
