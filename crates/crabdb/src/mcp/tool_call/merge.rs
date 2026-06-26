use serde_json::Value;

use crate::{CrabDb, Error, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.merge_queue_add" => {
            let args: MergeQueueAddArgs = parse_args(arguments)?;
            tool_result(db.enqueue_merge(&args.source, &args.target, args.priority)?)
        }
        "crabdb.merge_queue_list" => tool_result(db.list_merge_queue()?),
        "crabdb.merge_queue_run" => {
            let args: MergeQueueRunArgs = parse_args(arguments)?;
            tool_result(db.run_merge_queue(args.limit)?)
        }
        "crabdb.merge_queue_remove" => {
            let args: MergeQueueRemoveArgs = parse_args(arguments)?;
            tool_result(db.remove_merge_queue(&args.selector)?)
        }
        "crabdb.conflict_list" => tool_result(db.list_conflicts()?),
        "crabdb.conflict_show" => {
            let args: ConflictIdArgs = parse_args(arguments)?;
            tool_result(
                db.show_conflict_with_limit(&args.conflict_set_id, args.limit.unwrap_or(50))?,
            )
        }
        "crabdb.conflict_resolve" => {
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
