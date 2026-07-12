use serde_json::Value;

use crate::{Error, Result, Trail};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut Trail, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "trail.merge_queue_add" => {
            let args: MergeQueueAddArgs = parse_args(arguments)?;
            tool_result(db.enqueue_lane_merge(&args.source, &args.target, args.priority)?)
        }
        "trail.merge_queue_list" => tool_result(db.list_lane_merge_queue()?),
        "trail.merge_queue_run" => {
            let args: MergeQueueRunArgs = parse_args(arguments)?;
            tool_result(db.run_lane_merge_queue(args.limit)?)
        }
        "trail.merge_queue_explain" => {
            let args: MergeQueueExplainArgs = parse_args(arguments)?;
            tool_result(db.explain_lane_merge_queue(&args.selector)?)
        }
        "trail.merge_queue_remove" => {
            let args: MergeQueueRemoveArgs = parse_args(arguments)?;
            tool_result(db.remove_lane_merge_queue(&args.selector)?)
        }
        "trail.conflict_list" => tool_result(db.list_conflicts()?),
        "trail.conflict_show" => {
            let args: ConflictIdArgs = parse_args(arguments)?;
            tool_result(
                db.show_conflict_with_limit(&args.conflict_set_id, args.limit.unwrap_or(50))?,
            )
        }
        "trail.conflict_resolve" => {
            let args: ConflictResolveArgs = parse_args(arguments)?;
            match (args.take, args.manual) {
                (Some(take), None) => {
                    tool_result(db.resolve_conflict(&args.conflict_set_id, &take)?)
                }
                (None, Some(manual)) => {
                    tool_result(db.resolve_conflict_manual(&args.conflict_set_id, manual)?)
                }
                (Some(_), Some(_)) => Err(Error::InvalidInput(
                    "conflict_resolve requires only one of `take` or `manual`".to_string(),
                )),
                (None, None) => Err(Error::InvalidInput(
                    "conflict_resolve requires `take` or `manual`".to_string(),
                )),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
