use serde_json::Value;

use crate::{CrabDb, Error, Result};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut CrabDb, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "crabdb.doctor" => tool_result(db.doctor()?),
        "crabdb.status" => {
            let args: StatusArgs = parse_args(arguments)?;
            tool_result(db.status_read_only(args.branch.as_deref())?)
        }
        "crabdb.diff" => {
            let args: DiffArgs = parse_args(arguments)?;
            let forms = usize::from(args.range.is_some())
                + usize::from(args.root.is_some())
                + usize::from(args.dirty);
            if forms != 1 {
                return Err(Error::InvalidInput(
                    "diff requires exactly one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            let diff = if args.dirty {
                db.diff_dirty(args.patch, args.show_line_ids)?
            } else if let Some(root) = args.root {
                db.diff_roots(&root, args.patch, args.show_line_ids)?
            } else {
                db.diff_range_with_options(
                    args.range.as_deref().unwrap_or_default(),
                    args.patch,
                    args.show_line_ids,
                )?
            };
            tool_result(diff)
        }
        "crabdb.timeline" => {
            let args: TimelineArgs = parse_args(arguments)?;
            tool_result(db.timeline_query(
                args.branch.as_deref(),
                args.session.as_deref(),
                args.lane.as_deref(),
                args.limit.unwrap_or(30),
            )?)
        }
        "crabdb.why" => {
            let args: WhyArgs = parse_args(arguments)?;
            let at = args.at.as_deref().or(args.branch.as_deref());
            let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
                (Some(path_line), None) => db.why(path_line, at)?,
                (None, Some(line_id)) => db.why_line_id(line_id, at)?,
                (Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why accepts either path_line or line_id, not both".to_string(),
                    ));
                }
                (None, None) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why requires path_line or line_id".to_string(),
                    ));
                }
            };
            tool_result(result)
        }
        "crabdb.history" => {
            let args: HistoryArgs = parse_args(arguments)?;
            let result = if let Some(line_id) = args.line_id {
                db.history_for_line_id(&line_id)?
            } else if let Some(file_id) = args.file_id {
                db.history_for_file_id(&file_id)?
            } else {
                let selector = args.path.or(args.selector).ok_or_else(|| {
                    Error::InvalidInput(
                        "history requires `path`, `selector`, `file_id`, or `line_id`".to_string(),
                    )
                })?;
                db.history_for_path(&selector)?
            };
            tool_result(result)
        }
        "crabdb.code_from" => {
            let args: CodeFromArgs = parse_args(arguments)?;
            tool_result(db.code_from(&args.selector)?)
        }
        "crabdb.config_list" => tool_result(db.config_entries()),
        "crabdb.config_get" => {
            let args: ConfigKeyArgs = parse_args(arguments)?;
            tool_result(db.config_get(&args.key)?)
        }
        "crabdb.config_set" => {
            let args: ConfigSetArgs = parse_args(arguments)?;
            tool_result(db.config_set(&args.key, &args.value)?)
        }
        "crabdb.ignore_list" => tool_result(db.ignore_list()?),
        "crabdb.ignore_add" => {
            let args: IgnorePatternArgs = parse_args(arguments)?;
            tool_result(db.ignore_add(&args.pattern)?)
        }
        "crabdb.ignore_remove" => {
            let args: IgnorePatternArgs = parse_args(arguments)?;
            tool_result(db.ignore_remove(&args.pattern)?)
        }
        "crabdb.ignore_check" => {
            let args: IgnoreCheckArgs = parse_args(arguments)?;
            tool_result(db.ignore_check(&args.path)?)
        }
        "crabdb.guardrail_check" => {
            let args: GuardrailCheckArgs = parse_args(arguments)?;
            tool_result(db.guardrail_check(
                args.lane.as_deref(),
                &args.action,
                args.summary.as_deref(),
                args.payload,
                &args.paths,
            )?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
