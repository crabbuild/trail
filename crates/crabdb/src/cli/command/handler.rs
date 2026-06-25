use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::thread;

use clap::error::ErrorKind as ClapErrorKind;
use clap::Parser;

use super::{render::*, *};

use crabdb::model::{ConflictManualFile, ConflictManualResolution};
use crabdb::{
    Actor, AgentGateOptions, CrabDb, Error, InitImportMode, OperationKind, PatchDocument,
    RecordOptions, Result,
};

pub(crate) fn run_cli() {
    let json_errors =
        args_request_json_errors(std::env::args_os().skip(1)) || env_requests_json_errors();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => handle_cli_parse_error(err, json_errors),
    };
    let json_errors = cli.json
        || matches!(cli.format.as_ref(), Some(OutputFormat::Json))
        || env_requests_json_errors();
    if let Err(err) = run(cli) {
        render_error(&err, json_errors);
        std::process::exit(err.exit_code());
    }
}

fn args_request_json_errors<I>(args: I) -> bool
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut expect_format = false;
    for arg in args {
        let arg = arg.to_string_lossy();
        if expect_format {
            if arg == "json" {
                return true;
            }
            expect_format = false;
            continue;
        }
        if arg == "--json" || arg == "--format=json" {
            return true;
        }
        if arg == "--format" {
            expect_format = true;
        }
    }
    false
}

fn env_requests_json_errors() -> bool {
    std::env::var("CRABDB_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn resolve_output_format(cli_format: Option<OutputFormat>) -> Result<OutputFormat> {
    if let Some(format) = cli_format {
        return Ok(format);
    }
    let Some(value) = std::env::var("CRABDB_FORMAT").ok() else {
        return Ok(OutputFormat::Human);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "human" => Ok(OutputFormat::Human),
        "json" => Ok(OutputFormat::Json),
        "ndjson" => Ok(OutputFormat::Ndjson),
        other => Err(Error::InvalidInput(format!(
            "CRABDB_FORMAT must be human, json, or ndjson, got `{other}`"
        ))),
    }
}

fn handle_cli_parse_error(err: clap::Error, json: bool) -> ! {
    match err.kind() {
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => err.exit(),
        _ if json => {
            let exit_code = err.exit_code();
            render_cli_parse_error(&err, exit_code);
            std::process::exit(exit_code);
        }
        _ => err.exit(),
    }
}

fn render_cli_parse_error(err: &clap::Error, exit_code: i32) {
    let message = err.to_string();
    let value = serde_json::json!({
        "error": {
            "code": "INVALID_INPUT",
            "message": message.trim(),
            "exit_code": exit_code
        }
    });
    eprintln!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| {
            r#"{"error":{"code":"INVALID_INPUT","message":"invalid CLI input","exit_code":2}}"#
                .to_string()
        })
    );
}

fn render_error(err: &Error, json: bool) {
    if json {
        let value = serde_json::json!({
            "error": {
                "code": err.code(),
                "message": err.to_string(),
                "exit_code": err.exit_code()
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string(&value)
                .unwrap_or_else(|_| format!(r#"{{"error":{{"message":"{err}"}}}}"#))
        );
    } else {
        eprintln!("crabdb: {err}");
    }
}

fn run(cli: Cli) -> Result<()> {
    let format = resolve_output_format(cli.format)?;
    let json = cli.json || matches!(format, OutputFormat::Json);
    let workspace = cli
        .workspace
        .clone()
        .or_else(|| std::env::var_os("CRABDB_WORKSPACE").map(PathBuf::from));
    let db_dir = cli
        .db
        .clone()
        .or_else(|| std::env::var_os("CRABDB_DIR").map(PathBuf::from));
    let branch = cli
        .branch
        .clone()
        .or_else(|| std::env::var("CRABDB_BRANCH").ok());
    let ctx = RuntimeContext {
        workspace,
        db_dir,
        branch,
        json,
        quiet: cli.quiet,
        format,
    };
    match cli.command {
        Command::Init(args) => {
            let workspace = ctx
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir().map_err(Error::from)?);
            let mode = if args.from_git {
                InitImportMode::GitTracked
            } else if args.working_tree {
                InitImportMode::WorkingTree
            } else {
                InitImportMode::Empty
            };
            let report = CrabDb::init_with_text_policy(
                workspace,
                args.branch,
                mode,
                args.force,
                args.text_policy.as_ref().map(TextPolicyArg::as_str),
            )?;
            render_init(&report, ctx.json, ctx.quiet)
        }
        Command::Config(config) => match config.command {
            ConfigSubcommand::List => {
                let db = open_db(&ctx)?;
                let entries = db.config_entries();
                render_config_list(&entries, ctx.json, ctx.quiet)
            }
            ConfigSubcommand::Get(args) => {
                let db = open_db(&ctx)?;
                let entry = db.config_get(&args.key)?;
                render_config_entry(&entry, ctx.json, ctx.quiet)
            }
            ConfigSubcommand::Set(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.config_set(&args.key, &args.value)?;
                render_config_set(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Ignore(ignore) => match ignore.command {
            IgnoreSubcommand::List => {
                let db = open_db(&ctx)?;
                let report = db.ignore_list()?;
                render_ignore_list(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Add(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.ignore_add(&args.pattern)?;
                render_ignore_add(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Remove(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.ignore_remove(&args.pattern)?;
                render_ignore_remove(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Check(args) => {
                let db = open_db(&ctx)?;
                let report = db.ignore_check(&args.path)?;
                render_ignore_check(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Guardrails(guardrails) => match guardrails.command {
            GuardrailsSubcommand::Check(args) => {
                let db = open_db(&ctx)?;
                let payload = parse_optional_json(args.payload_json.as_deref())?;
                let report = db.guardrail_check(
                    args.agent.as_deref(),
                    &args.action,
                    args.summary.as_deref(),
                    payload,
                    &args.paths,
                )?;
                render_guardrail_check(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Status(args) => {
            let db = open_db(&ctx)?;
            let branch = args.branch.as_deref().or(ctx.branch.as_deref());
            let report = db.status(branch)?;
            render_status(&report, ctx.json, ctx.quiet)
        }
        Command::Record(args) => {
            let mut db = open_db(&ctx)?;
            let kind = args
                .kind
                .as_deref()
                .map(parse_record_kind_arg)
                .transpose()?;
            let report = db.record_with_options(
                ctx.branch.as_deref(),
                args.message,
                Actor::human(),
                RecordOptions {
                    paths: args.paths,
                    kind,
                    session_id: args.session,
                    allow_ignored: args.allow_ignored,
                },
            )?;
            render_record(&report, ctx.json, ctx.quiet)
        }
        Command::Watch(args) => {
            let mut db = open_db(&ctx)?;
            let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
            let _include_untracked = args.include_untracked;
            loop {
                let report = db.record_with_options(
                    ctx.branch.as_deref(),
                    args.message.clone(),
                    Actor::human(),
                    RecordOptions {
                        kind: Some(OperationKind::WatchRecord),
                        session_id: args.session.clone(),
                        ..RecordOptions::default()
                    },
                )?;
                if matches!(ctx.format, OutputFormat::Ndjson) {
                    println!("{}", serde_json::to_string(&report)?);
                } else if report.operation.is_some() {
                    render_record(&report, ctx.json, ctx.quiet)?;
                }
                if args.once {
                    break;
                }
                thread::sleep(interval);
            }
            Ok(())
        }
        Command::Timeline(args) => {
            let db = open_db(&ctx)?;
            let branch = args.branch.as_deref().or(ctx.branch.as_deref());
            let entries = db.timeline_query(
                branch,
                args.session.as_deref(),
                args.agent.as_deref(),
                args.limit,
            )?;
            render_timeline(&entries, ctx.json, ctx.quiet)
        }
        Command::Show(args) => {
            let db = open_db(&ctx)?;
            let result = db.show(&args.selector)?;
            render_show(&result, ctx.json, ctx.quiet)
        }
        Command::Object(object) => match object.command {
            ObjectSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_object(&args.object_id)?;
                render_object_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Root(root) => match root.command {
            RootSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_root(&args.root_id)?;
                render_root_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Text(text) => match text.command {
            TextSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_text(&args.text_id, args.limit)?;
                render_text_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Map(map) => match map.command {
            MapSubcommand::Range(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_map_range(
                    &args.map_id,
                    args.map_type.as_str(),
                    args.start.as_deref(),
                    args.end.as_deref(),
                    args.limit,
                )?;
                render_map_range(&report, ctx.json, ctx.quiet)
            }
            MapSubcommand::Diff(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_map_diff(
                    &args.left_map_id,
                    &args.right_map_id,
                    args.map_type.as_str(),
                    args.start.as_deref(),
                    args.end.as_deref(),
                    args.limit,
                )?;
                render_map_diff(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Diff(args) => {
            let mut db = open_db(&ctx)?;
            let summary = diff_from_args(&mut db, &args)?;
            render_diff(&summary, ctx.json, ctx.quiet, args.stat)
        }
        Command::Checkout(args) => {
            let mut db = open_db(&ctx)?;
            let report = db.checkout_with_options(
                &args.target,
                args.force,
                args.dry_run,
                args.workdir.as_deref(),
                args.record_dirty,
            )?;
            render_checkout(&report, ctx.json, ctx.quiet)
        }
        Command::Branch(args) => {
            let mut db = open_db(&ctx)?;
            match (
                args.name.as_deref(),
                args.delete.as_deref(),
                args.rename.as_deref(),
                args.to.as_deref(),
            ) {
                (Some(name), None, None, None) => {
                    let report = db.create_branch(name, args.from.as_deref())?;
                    render_branch(&report, ctx.json, ctx.quiet)
                }
                (None, Some(name), None, None) => {
                    let report = db.delete_branch(name)?;
                    render_branch_delete(&report, ctx.json, ctx.quiet)
                }
                (None, None, Some(old_name), Some(new_name)) => {
                    let report = db.rename_branch(old_name, new_name)?;
                    render_branch_rename(&report, ctx.json, ctx.quiet)
                }
                (None, None, None, None) => {
                    let entries = db.list_branches()?;
                    render_branch_list(&entries, ctx.json, ctx.quiet)
                }
                _ => Err(Error::InvalidInput(
                    "branch accepts either NAME [--from REF], --delete NAME, --rename OLD --to NEW, or no arguments".to_string(),
                )),
            }
        }
        Command::Merge(args) => {
            let mut db = open_db(&ctx)?;
            validate_merge_strategy(args.strategy.as_deref())?;
            let report = db.merge_branches_with_options(&args.source, &args.into, args.dry_run)?;
            render_merge(&report, ctx.json, ctx.quiet)
        }
        Command::Why(args) => {
            let db = open_db(&ctx)?;
            let at = args.at.as_deref().or(ctx.branch.as_deref());
            let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
                (Some(path_line), None) => db.why(path_line, at)?,
                (None, Some(line_id)) => db.why_line_id(line_id, at)?,
                (Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "why accepts either PATH:LINE or --line-id, not both".to_string(),
                    ));
                }
                (None, None) => {
                    return Err(Error::InvalidInput(
                        "why requires PATH:LINE or --line-id".to_string(),
                    ));
                }
            };
            render_why(&result, ctx.json, ctx.quiet)
        }
        Command::History(args) => {
            let db = open_db(&ctx)?;
            let result = match (
                args.selector.as_deref(),
                args.file_id.as_deref(),
                args.line_id.as_deref(),
            ) {
                (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "history accepts one path, --file-id, or --line-id selector".to_string(),
                    ));
                }
                (_, Some(file_id), None) => db.history_for_file_id(file_id)?,
                (_, None, Some(line_id)) => db.history_for_line_id(line_id)?,
                (Some(path), None, None) => db.history_for_path(path)?,
                (None, None, None) => {
                    return Err(Error::InvalidInput(
                        "history requires a path, --file-id, or --line-id".to_string(),
                    ));
                }
            };
            render_history(&result, ctx.json, ctx.quiet)
        }
        Command::CodeFrom(args) => {
            let db = open_db(&ctx)?;
            let result = db.code_from(&args.selector)?;
            render_code_from(&result, ctx.json, ctx.quiet)
        }
        Command::Agent(agent) => match agent.command {
            AgentSubcommand::Spawn(args) => {
                let mut db = open_db(&ctx)?;
                let materialize = args.materialize && !args.no_materialize;
                let report = db.spawn_agent_with_workdir(
                    &args.name,
                    args.from.as_deref(),
                    materialize,
                    args.provider,
                    args.model,
                    args.workdir,
                )?;
                render_agent_spawn(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::List => {
                let db = open_db(&ctx)?;
                let agents = db.list_agents()?;
                render_agent_list(&agents, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let details = db.agent_details(&args.name)?;
                render_agent_details(&details, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Status(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_status(&args.name)?;
                render_agent_status(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Contribution(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_contribution(&args.name, args.limit)?;
                render_agent_contribution(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Gates(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_gate_history(&args.name, args.kind.as_deref(), args.limit)?;
                render_agent_gate_history(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Readiness(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_readiness(&args.name)?;
                render_agent_readiness(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Handoff(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_handoff(&args.name, args.limit)?;
                render_agent_handoff(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Claim(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.claim_agent_path(&args.name, &args.path, args.ttl_secs)?;
                render_agent_claim(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Message(args) => {
                let mut db = open_db(&ctx)?;
                let report =
                    db.add_agent_message(&args.name, &args.role, &args.text, args.session)?;
                render_agent_message(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Turn(turn) => match turn.command {
                AgentTurnSubcommand::Start(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.begin_agent_turn(
                        &args.name,
                        args.from.as_deref(),
                        args.title,
                        args.base_change.as_deref(),
                    )?;
                    render_agent_turn_start(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let details = db.show_agent_turn(&args.turn_id)?;
                    render_agent_turn_details(&details, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Message(args) => {
                    let mut db = open_db(&ctx)?;
                    let report =
                        db.add_agent_turn_message(&args.turn_id, &args.role, &args.text)?;
                    render_agent_message(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Event(args) => {
                    let mut db = open_db(&ctx)?;
                    let payload = parse_optional_json(args.payload_json.as_deref())?;
                    let report = db.add_agent_turn_event(
                        &args.turn_id,
                        &args.event_type,
                        payload,
                        args.change.as_deref(),
                        args.message.as_deref(),
                    )?;
                    render_agent_turn_event(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::ApplyPatch(args) => {
                    let mut db = open_db(&ctx)?;
                    let mut patch: PatchDocument =
                        serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
                    if args.allow_ignored {
                        patch.allow_ignored = true;
                    }
                    let report = db.apply_agent_turn_patch(&args.turn_id, patch)?;
                    render_agent_patch(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::End(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.end_agent_turn(&args.turn_id, &args.status)?;
                    render_agent_turn_end(&report, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Run(run) => match run.command {
                AgentRunSubcommand::Pause(args) => {
                    let mut db = open_db(&ctx)?;
                    let state = parse_optional_json(args.state_json.as_deref())?;
                    let interruption = parse_optional_json(args.interruption_json.as_deref())?;
                    let report = db.pause_agent_run(
                        &args.name,
                        &args.reason,
                        &args.summary,
                        state,
                        interruption,
                        args.session.as_deref(),
                        args.turn.as_deref(),
                    )?;
                    render_agent_run_pause(&report, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::List(args) => {
                    let db = open_db(&ctx)?;
                    let run_states =
                        db.list_agent_run_states(args.agent.as_deref(), args.status.as_deref())?;
                    render_agent_run_list(&run_states, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let run_state = db.show_agent_run_state(&args.run_id)?;
                    render_agent_run_state(&run_state, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::Resume(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.resume_agent_run(&args.run_id, args.reviewer, args.note)?;
                    render_agent_run_resume(&report, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Events(args) => {
                let db = open_db(&ctx)?;
                let events = db.list_agent_events(
                    args.agent.as_deref(),
                    args.session.as_deref(),
                    args.turn.as_deref(),
                    args.event_type.as_deref(),
                    args.limit,
                )?;
                render_agent_events(&events, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Trace(trace) => match trace.command {
                AgentTraceSubcommand::Start(args) => {
                    let mut db = open_db(&ctx)?;
                    let attributes = parse_optional_json(args.attributes_json.as_deref())?;
                    let report = db.start_agent_trace_span(
                        &args.turn_id,
                        &args.span_type,
                        &args.name,
                        args.parent.as_deref(),
                        args.trace_id.as_deref(),
                        attributes,
                    )?;
                    render_agent_trace_span_start(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::End(args) => {
                    let mut db = open_db(&ctx)?;
                    let result = parse_optional_json(args.result_json.as_deref())?;
                    let report = db.end_agent_trace_span(&args.span_id, &args.status, result)?;
                    render_agent_trace_span_end(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::List(args) => {
                    let db = open_db(&ctx)?;
                    let spans = db.list_agent_trace_spans(
                        args.agent.as_deref(),
                        args.session.as_deref(),
                        args.turn.as_deref(),
                        args.trace_id.as_deref(),
                        args.limit,
                    )?;
                    render_agent_trace_spans(&spans, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::Summary(args) => {
                    let db = open_db(&ctx)?;
                    let report = db.summarize_agent_trace_spans(
                        args.agent.as_deref(),
                        args.session.as_deref(),
                        args.turn.as_deref(),
                        args.trace_id.as_deref(),
                        args.slowest_limit,
                    )?;
                    render_agent_trace_summary(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let span = db.show_agent_trace_span(&args.span_id)?;
                    render_agent_trace_span(&span, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Record(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.record_agent_workdir(&args.name, args.message)?;
                render_agent_record(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Watch(args) => {
                let mut db = open_db(&ctx)?;
                let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
                let _include_untracked = args.include_untracked;
                if args.once {
                    let report =
                        db.watch_agent_workdir(&args.name, args.message, interval, Some(1))?;
                    render_agent_watch(&report, ctx.json, ctx.quiet)
                } else {
                    loop {
                        let report = db.record_agent_workdir(&args.name, args.message.clone())?;
                        if matches!(ctx.format, OutputFormat::Ndjson) {
                            println!("{}", serde_json::to_string(&report)?);
                        } else if report.operation.is_some() {
                            render_agent_record(&report, ctx.json, ctx.quiet)?;
                        }
                        thread::sleep(interval);
                    }
                }
            }
            AgentSubcommand::Test(args) => {
                let mut db = open_db(&ctx)?;
                let options = AgentGateOptions {
                    suite: args.suite,
                    score: args.score,
                    threshold: args.threshold,
                };
                let report = db.run_agent_test_with_options(
                    &args.name,
                    args.command,
                    args.turn.as_deref(),
                    args.timeout_secs,
                    options,
                )?;
                let render_result = render_agent_test(&report, ctx.json, ctx.quiet);
                if render_result.is_ok() && !report.success {
                    std::process::exit(command_failure_exit_code(report.exit_code));
                }
                render_result
            }
            AgentSubcommand::Eval(args) => {
                let mut db = open_db(&ctx)?;
                let options = AgentGateOptions {
                    suite: args.suite,
                    score: args.score,
                    threshold: args.threshold,
                };
                let report = db.run_agent_eval_with_options(
                    &args.name,
                    args.command,
                    args.turn.as_deref(),
                    args.timeout_secs,
                    options,
                )?;
                let render_result = render_agent_test(&report, ctx.json, ctx.quiet);
                if render_result.is_ok() && !report.success {
                    std::process::exit(command_failure_exit_code(report.exit_code));
                }
                render_result
            }
            AgentSubcommand::Workdir(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_workdir(&args.name)?;
                render_agent_workdir(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::SyncWorkdir(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.sync_agent_workdir(&args.name, args.force)?;
                render_agent_workdir_sync(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::ApplyPatch(args) => {
                let mut db = open_db(&ctx)?;
                let mut patch: PatchDocument =
                    serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
                if args.allow_ignored {
                    patch.allow_ignored = true;
                }
                let report = db.apply_agent_patch(&args.name, patch)?;
                render_agent_patch(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Diff(args) => {
                let db = open_db(&ctx)?;
                let summary =
                    db.diff_agent_with_options(&args.name, args.patch, args.show_line_ids)?;
                render_diff(&summary, ctx.json, ctx.quiet, false)
            }
            AgentSubcommand::Timeline(args) => {
                let db = open_db(&ctx)?;
                let entries = db.agent_timeline(&args.name, args.limit)?;
                render_timeline(&entries, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Checkout(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.checkout_agent_with_options(
                    &args.name,
                    args.force,
                    args.dry_run,
                    args.workdir.as_deref(),
                )?;
                render_checkout(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Rm(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.remove_agent(&args.name, args.force)?;
                render_agent_remove(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Session(session) => match session.command {
            SessionSubcommand::Start(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.start_agent_session(&args.agent, args.title, args.id)?;
                render_session_start(&report, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Current(args) => {
                let db = open_db(&ctx)?;
                let reports = db.current_agent_sessions(args.agent.as_deref())?;
                render_session_current(&reports, ctx.json, ctx.quiet)
            }
            SessionSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let sessions = db.list_agent_sessions(args.agent.as_deref())?;
                render_session_list(&sessions, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let details = db.show_agent_session(&args.session_id)?;
                render_session_details(&details, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Context(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_session_context(&args.session_id, args.limit)?;
                render_session_context(&report, ctx.json, ctx.quiet)
            }
            SessionSubcommand::End(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.end_agent_session(&args.session_id, &args.status)?;
                render_session_end(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Approvals(approvals) => match approvals.command {
            ApprovalsSubcommand::Request(args) => {
                let mut db = open_db(&ctx)?;
                let payload = args
                    .payload_json
                    .map(|payload| serde_json::from_str::<serde_json::Value>(&payload))
                    .transpose()?;
                let report = db.request_agent_approval(
                    &args.agent,
                    &args.action,
                    &args.summary,
                    payload,
                    args.session.as_deref(),
                    args.turn.as_deref(),
                )?;
                render_approval_request(&report, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let approvals =
                    db.list_agent_approvals(args.agent.as_deref(), args.status.as_deref())?;
                render_approval_list(&approvals, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let approval = db.show_agent_approval(&args.approval_id)?;
                render_approval(&approval, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::Decide(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.decide_agent_approval(
                    &args.approval_id,
                    args.decision.as_str(),
                    args.reviewer,
                    args.note,
                )?;
                render_approval_decision(&report, ctx.json, ctx.quiet)
            }
        },
        Command::MergeAgent(args) => {
            let mut db = open_db(&ctx)?;
            validate_merge_strategy(args.strategy.as_deref())?;
            let report = db.merge_agent_with_options(&args.name, &args.into, args.dry_run)?;
            render_merge(&report, ctx.json, ctx.quiet)
        }
        Command::MergeQueue(queue) => match queue.command {
            MergeQueueSubcommand::Add(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.enqueue_merge(&args.source, &args.into, args.priority)?;
                render_merge_queue_add(&report, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::List => {
                let db = open_db(&ctx)?;
                let entries = db.list_merge_queue()?;
                render_merge_queue_list(&entries, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::Run(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.run_merge_queue(args.limit)?;
                render_merge_queue_run(&report, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::Remove(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.remove_merge_queue(&args.selector)?;
                render_merge_queue_remove(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Conflicts(conflicts) => match conflicts.command {
            ConflictsSubcommand::List => {
                let db = open_db(&ctx)?;
                let conflicts = db.list_conflicts()?;
                render_conflicts(&conflicts, ctx.json, ctx.quiet)
            }
            ConflictsSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let conflict = db.show_conflict(&args.conflict_set_id)?;
                render_conflict(&conflict, ctx.json, ctx.quiet)
            }
            ConflictsSubcommand::Resolve(args) => {
                let mut db = open_db(&ctx)?;
                let report = if let Some(manual_path) = args.manual {
                    let manual = read_manual_conflict_resolution(&manual_path)?;
                    db.resolve_conflict_manual(&args.conflict_set_id, manual)?
                } else if let Some(take) = args.take {
                    db.resolve_conflict(&args.conflict_set_id, take.as_str())?
                } else {
                    return Err(Error::InvalidInput(
                        "conflicts resolve requires `--take` or `--manual`".to_string(),
                    ));
                };
                render_conflict_resolve(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Anchor(anchor) => match anchor.command {
            AnchorSubcommand::Create(args) => {
                let mut db = open_db(&ctx)?;
                let report =
                    db.create_anchor(&args.path_line, args.label, ctx.branch.as_deref())?;
                render_anchor_create(&report, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::Resolve(args) => {
                let db = open_db(&ctx)?;
                let report = db.resolve_anchor(&args.anchor_id, ctx.branch.as_deref())?;
                render_anchor_resolve(&report, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::List => {
                let db = open_db(&ctx)?;
                let anchors = db.list_anchors()?;
                render_anchor_list(&anchors, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::Delete(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.delete_anchor(&args.anchor_id)?;
                render_anchor_delete(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Lease(lease) => match lease.command {
            LeaseSubcommand::Acquire(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.acquire_lease(
                    &args.agent,
                    Some(&args.path),
                    args.mode.as_str(),
                    args.ttl_secs,
                )?;
                render_lease_acquire(&report, ctx.json, ctx.quiet)
            }
            LeaseSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let leases = db.list_leases(args.all)?;
                render_lease_list(&leases, ctx.json, ctx.quiet)
            }
            LeaseSubcommand::Release(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.release_lease(&args.lease_id)?;
                render_lease_release(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Git(git) => match git.command {
            GitSubcommand::Export(args) => {
                if let Some(message) = args.message {
                    if args.output.is_some() {
                        return Err(Error::InvalidInput(
                            "git export -m cannot be combined with --output".to_string(),
                        ));
                    }
                    let mut db = open_db(&ctx)?;
                    let report = db.git_export_commit(&args.range, &message)?;
                    render_git_export(&report, ctx.json, ctx.quiet)
                } else if let Some(output) = args.output {
                    let db = open_db(&ctx)?;
                    db.write_patch_to(&args.range, &output)?;
                    if !ctx.quiet {
                        println!("Wrote patch: {}", output.display());
                    }
                    Ok(())
                } else {
                    let db = open_db(&ctx)?;
                    let patch = db.export_patch(&args.range)?;
                    print!("{patch}");
                    Ok(())
                }
            }
            GitSubcommand::ImportUpdate(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.git_import_update(ctx.branch.as_deref(), args.message)?;
                render_git_import_update(&report, ctx.json, ctx.quiet)
            }
            GitSubcommand::Mappings(args) => {
                let db = open_db(&ctx)?;
                let mappings = db.git_mappings(args.limit)?;
                render_git_mappings(&mappings, ctx.json, ctx.quiet)
            }
        },
        Command::Api(api) => match api.command {
            ApiSubcommand::Openapi(args) => {
                let spec = crabdb::server::openapi_spec();
                let json = serde_json::to_string_pretty(&spec)?;
                if let Some(output) = args.output {
                    fs::write(output, json)?;
                } else {
                    println!("{json}");
                }
                Ok(())
            }
        },
        Command::Daemon(args) => {
            let mut db = open_db(&ctx)?;
            let (auth, token_file) = daemon_auth(&db, &args)?;
            let addr: SocketAddr = format!("{}:{}", args.host, args.port)
                .parse()
                .map_err(|err| Error::InvalidInput(format!("invalid listen address: {err}")))?;
            let listener = TcpListener::bind(addr)?;
            let local_addr = listener.local_addr()?;
            if !ctx.quiet {
                println!("CrabDB API listening on http://{local_addr}");
                if args.no_auth {
                    println!("Daemon auth disabled");
                } else if let Some(path) = token_file {
                    println!("Daemon token file: {}", path.display());
                    println!(
                        "Send requests with: Authorization: Bearer $(cat {})",
                        path.display()
                    );
                } else {
                    println!("Daemon token configured from flag or CRABDB_DAEMON_TOKEN");
                }
            }
            let max_requests = if args.once {
                Some(1)
            } else {
                args.max_requests
            };
            crabdb::server::serve_listener_with_auth(&mut db, listener, max_requests, auth)
        }
        Command::Mcp => {
            let mut db = open_db(&ctx)?;
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            crabdb::mcp::serve_stdio(&mut db, stdin.lock(), &mut stdout)
        }
        Command::Doctor => {
            let db = open_db(&ctx)?;
            let report = db.doctor()?;
            render_doctor(&report, ctx.json, ctx.quiet)
        }
        Command::Backup(backup) => match backup.command {
            BackupSubcommand::Create(args) => {
                let db = open_db(&ctx)?;
                let report = db.create_backup(args.output, args.overwrite)?;
                render_backup_create(&report, ctx.json, ctx.quiet)
            }
            BackupSubcommand::Verify(args) => {
                let report = CrabDb::verify_backup(args.path)?;
                render_backup_verify(&report, ctx.json, ctx.quiet)
            }
            BackupSubcommand::Restore(args) => {
                let workspace = ctx
                    .workspace
                    .clone()
                    .unwrap_or(std::env::current_dir().map_err(Error::from)?);
                let report = CrabDb::restore_backup(workspace, args.path, args.force)?;
                render_backup_restore(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Fsck => {
            let db = open_db(&ctx)?;
            let report = db.fsck()?;
            render_fsck(&report, ctx.json, ctx.quiet)
        }
        Command::Index(index) => match index.command {
            IndexSubcommand::Rebuild => {
                let mut db = open_db(&ctx)?;
                let report = db.rebuild_indexes()?;
                render_index_rebuild(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Gc(args) => {
            let mut db = open_db(&ctx)?;
            let report = db.gc(args.dry_run)?;
            render_gc(&report, ctx.json, ctx.quiet)
        }
    }
}

struct RuntimeContext {
    workspace: Option<PathBuf>,
    db_dir: Option<PathBuf>,
    branch: Option<String>,
    json: bool,
    quiet: bool,
    format: OutputFormat,
}

fn open_db(ctx: &RuntimeContext) -> Result<CrabDb> {
    match (&ctx.workspace, &ctx.db_dir) {
        (Some(workspace), Some(db_dir)) => CrabDb::open_with_db_dir(workspace, db_dir),
        (Some(workspace), None) => CrabDb::open(workspace),
        (None, Some(db_dir)) => {
            let workspace = db_dir
                .parent()
                .ok_or_else(|| Error::WorkspaceNotFound(db_dir.clone()))?;
            CrabDb::open_with_db_dir(workspace, db_dir)
        }
        (None, None) => CrabDb::discover(std::env::current_dir().map_err(Error::from)?),
    }
}

fn daemon_auth(
    db: &CrabDb,
    args: &DaemonArgs,
) -> Result<(crabdb::server::ServerAuth, Option<PathBuf>)> {
    if args.no_auth {
        if args.auth_token.is_some() || args.auth_token_file.is_some() {
            return Err(Error::InvalidInput(
                "--no-auth cannot be combined with --auth-token or --auth-token-file".to_string(),
            ));
        }
        return Ok((crabdb::server::ServerAuth::disabled(), None));
    }

    if let Some(token) = &args.auth_token {
        return Ok((crabdb::server::ServerAuth::bearer(token.clone())?, None));
    }
    if let Ok(token) = std::env::var("CRABDB_DAEMON_TOKEN") {
        return Ok((crabdb::server::ServerAuth::bearer(token)?, None));
    }

    let token_path = args
        .auth_token_file
        .clone()
        .unwrap_or_else(|| db.db_dir().join("daemon.token"));
    let token = if token_path.exists() {
        fs::read_to_string(&token_path)?.trim().to_string()
    } else {
        let token = generate_daemon_token()?;
        fs::write(&token_path, format!("{token}\n"))?;
        restrict_secret_file(&token_path)?;
        token
    };
    Ok((crabdb::server::ServerAuth::bearer(token)?, Some(token_path)))
}

fn generate_daemon_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        Error::InvalidInput(format!("failed to generate daemon auth token: {err}"))
    })?;
    Ok(hex::encode(bytes))
}

fn restrict_secret_file(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn parse_optional_json(value: Option<&str>) -> Result<Option<serde_json::Value>> {
    value
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::from)
}

fn read_manual_conflict_resolution(path: &PathBuf) -> Result<ConflictManualResolution> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("files").is_some() {
        return serde_json::from_value(value).map_err(Error::from);
    }
    let files: BTreeMap<String, ConflictManualFile> =
        serde_json::from_value(value).map_err(Error::from)?;
    Ok(ConflictManualResolution { files })
}

fn parse_record_kind_arg(value: &str) -> Result<OperationKind> {
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

fn validate_merge_strategy(value: Option<&str>) -> Result<()> {
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

fn command_failure_exit_code(exit_code: Option<i32>) -> i32 {
    exit_code
        .filter(|code| *code != 0)
        .unwrap_or(1)
        .clamp(1, 255)
}
