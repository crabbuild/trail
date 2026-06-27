use super::*;
use crate::cli::command::render::render_agent_timeline;
use crabdb::AgentRunReport;
use std::process::{Command as ProcessCommand, Stdio};

pub(super) fn handle_agent_command(ctx: &RuntimeContext, agent: AgentCommand) -> Result<()> {
    match agent.command {
        None => handle_agent_inbox(ctx),
        Some(AgentSubcommand::Setup(args)) => handle_agent_setup(ctx, args),
        Some(AgentSubcommand::Acp(args)) => handle_agent_acp(ctx, args),
        Some(AgentSubcommand::Start(args)) => handle_agent_start(ctx, args),
        Some(AgentSubcommand::Ask(args)) => handle_agent_ask(ctx, args),
        Some(AgentSubcommand::Next(args)) => handle_agent_next(ctx, args),
        Some(AgentSubcommand::Status) => handle_agent_status(ctx),
        Some(AgentSubcommand::Inbox) => handle_agent_inbox(ctx),
        Some(AgentSubcommand::Brief(args)) => handle_agent_brief(ctx, args),
        Some(AgentSubcommand::Summary(args)) => handle_agent_summary(ctx, args),
        Some(AgentSubcommand::Validate(args)) => handle_agent_validate(ctx, args),
        Some(AgentSubcommand::Report(args)) => handle_agent_report(ctx, args),
        Some(AgentSubcommand::Receipt(args)) => handle_agent_receipt(ctx, args),
        Some(AgentSubcommand::Pr(args)) => handle_agent_pr(ctx, args),
        Some(AgentSubcommand::Story(args)) => handle_agent_story(ctx, args),
        Some(AgentSubcommand::Risk(args)) => handle_agent_risk(ctx, args),
        Some(AgentSubcommand::Ready(args)) => handle_agent_ready(ctx, args),
        Some(AgentSubcommand::Diagnose(args)) => handle_agent_diagnose(ctx, args),
        Some(AgentSubcommand::Compare(args)) => handle_agent_compare(ctx, args),
        Some(AgentSubcommand::Test(args)) => handle_agent_gate(ctx, args, AgentGateKind::Test),
        Some(AgentSubcommand::Eval(args)) => handle_agent_gate(ctx, args, AgentGateKind::Eval),
        Some(AgentSubcommand::Workdir(args)) => handle_agent_workdir(ctx, args),
        Some(AgentSubcommand::List) => handle_agent_list(ctx),
        Some(AgentSubcommand::View(args)) => handle_agent_view(ctx, args),
        Some(AgentSubcommand::Changes(args)) => handle_agent_changes(ctx, args),
        Some(AgentSubcommand::Delta(args)) => handle_agent_delta(ctx, args),
        Some(AgentSubcommand::New(args)) => handle_agent_new(ctx, args),
        Some(AgentSubcommand::MarkReviewed(args)) => handle_agent_mark_reviewed(ctx, args),
        Some(AgentSubcommand::Change(args)) => handle_agent_change(ctx, args),
        Some(AgentSubcommand::Files(args)) => handle_agent_files(ctx, args),
        Some(AgentSubcommand::File(args)) => handle_agent_file(ctx, args),
        Some(AgentSubcommand::Checkpoints(args)) => handle_agent_checkpoints(ctx, args),
        Some(AgentSubcommand::Timeline(args)) => handle_agent_timeline(ctx, args),
        Some(AgentSubcommand::Turn(args)) => handle_agent_turn(ctx, args),
        Some(AgentSubcommand::TurnDiff(args)) => handle_agent_turn_diff(ctx, args),
        Some(AgentSubcommand::Why(args)) => handle_agent_why(ctx, args),
        Some(AgentSubcommand::Diff(args)) => handle_agent_diff(ctx, args),
        Some(AgentSubcommand::Review(args)) => handle_agent_review(ctx, args),
        Some(AgentSubcommand::Focus(args)) => handle_agent_focus(ctx, args),
        Some(AgentSubcommand::Apply(args)) => handle_agent_apply(ctx, args),
        Some(AgentSubcommand::Undo(args)) => handle_agent_undo(ctx, args),
        Some(AgentSubcommand::Rewind(args)) => handle_agent_rewind(ctx, args),
        Some(AgentSubcommand::Doctor(args)) => handle_agent_doctor(ctx, args),
    }
}

fn handle_agent_setup(ctx: &RuntimeContext, args: AgentSetupArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_setup_report(&args.provider, &args.editor)?;
    render_agent_setup(&report, ctx.json, ctx.quiet)
}

fn handle_agent_acp(ctx: &RuntimeContext, args: AgentAcpArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let lane = db.fresh_agent_lane_name(&args.provider, args.name.as_deref());
    let upstream_command = if args.command.is_empty() {
        default_acp_upstream_command(&args.provider, &lane)?
    } else {
        args.command
    };
    crabdb::acp::run_stdio_relay(AcpRelayOptions {
        workspace_root: db.workspace_root().to_path_buf(),
        db_dir: db.db_dir().to_path_buf(),
        lane: Some(lane),
        from_ref: args.from,
        provider: Some(args.provider),
        model: None,
        materialize: true,
        workdir: None,
        inject_mcp: !args.no_mcp,
        upstream_command,
    })
}

fn handle_agent_start(ctx: &RuntimeContext, args: AgentStartArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let lane = db.fresh_agent_lane_name(&args.provider, args.name.as_deref());
    let spawn = db.spawn_lane(&lane, None, true, Some(args.provider.clone()), None)?;
    let workdir = spawn.workdir.clone().ok_or_else(|| {
        Error::InvalidInput("agent start requires a materialized lane workdir".to_string())
    })?;
    let session = db
        .start_lane_session(
            &lane,
            Some(format!("Agent terminal {}", args.provider)),
            None,
        )?
        .session;
    db.add_lane_session_event(
        &lane,
        &session.session_id,
        "agent_task_started",
        Some(serde_json::json!({
            "provider": args.provider,
            "workdir": workdir,
            "mode": "terminal"
        })),
    )?;
    drop(db);

    let command = if args.command.is_empty() {
        default_terminal_agent_command(&args.provider)?
    } else {
        args.command
    };
    if !ctx.quiet && !ctx.json {
        println!("Agent task: {lane}");
        println!("Workdir: {workdir}");
        println!("Command: {}", command.join(" "));
    }
    let mut process = ProcessCommand::new(&command[0]);
    process
        .args(&command[1..])
        .current_dir(&workdir)
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit());
    if ctx.json {
        process.stdout(Stdio::piped());
    } else {
        process.stdout(Stdio::inherit());
    }
    let mut child = process.spawn().map_err(Error::from)?;
    let stdout_proxy = if ctx.json {
        child.stdout.take().map(|mut stdout| {
            std::thread::spawn(move || {
                let mut stderr = std::io::stderr().lock();
                let _ = std::io::copy(&mut stdout, &mut stderr);
            })
        })
    } else {
        None
    };
    let status = child.wait().map_err(Error::from)?;
    if let Some(proxy) = stdout_proxy {
        let _ = proxy.join();
    }
    let exit_code = status.code();

    let mut db = open_db(ctx)?;
    let recorded =
        db.record_lane_workdir(&lane, Some(format!("Agent task `{lane}` checkpoint")))?;
    let status = if exit_code == Some(0) {
        "completed"
    } else {
        "failed"
    };
    db.add_lane_session_event(
        &lane,
        &session.session_id,
        "agent_task_finished",
        Some(serde_json::json!({
            "provider": args.provider,
            "exit_code": exit_code,
            "status": status
        })),
    )?;
    db.end_lane_session(&session.session_id, status)?;
    let view = db.agent_task_view(&lane)?;
    let report = AgentRunReport {
        task: view.task,
        provider: args.provider,
        command,
        workdir: Some(workdir),
        exit_code,
        recorded: Some(recorded),
        status: status.to_string(),
    };
    render_agent_run(&report, ctx.json, ctx.quiet)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AgentAskIntent {
    Inbox,
    Next,
    Summary,
    Validate,
    Brief,
    Story,
    Risk,
    Ready,
    Diagnose,
    Receipt,
    Pr,
    Changes,
    TurnDiff { file: Option<String>, patch: bool },
    Delta { file: Option<String>, patch: bool },
    New { file: Option<String>, patch: bool },
    Files,
    File { path: String, patch: bool },
    Workdir,
    Checkpoints,
    Timeline,
    Turn,
    Why(String),
    Review,
    Focus,
    View,
}

fn handle_agent_ask(ctx: &RuntimeContext, args: AgentAskArgs) -> Result<()> {
    let selector = args.selector;
    let question = args.question.join(" ");
    match resolve_agent_ask_intent(&question)? {
        AgentAskIntent::Inbox => handle_agent_inbox(ctx),
        AgentAskIntent::Next => handle_agent_next(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Summary => handle_agent_summary(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Validate => handle_agent_validate(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Brief => handle_agent_brief(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Story => handle_agent_story(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Risk => handle_agent_risk(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Ready => handle_agent_ready(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Diagnose => handle_agent_diagnose(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Receipt => handle_agent_receipt(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Pr => handle_agent_pr(
            ctx,
            AgentPrArgs {
                selector,
                title_only: false,
                body_only: false,
            },
        ),
        AgentAskIntent::Changes => handle_agent_changes(
            ctx,
            AgentChangesArgs {
                selector,
                by_turn: false,
                by_operation: false,
            },
        ),
        AgentAskIntent::TurnDiff { file, patch } => handle_agent_turn_diff(
            ctx,
            AgentTurnDiffArgs {
                selector,
                turn: None,
                file,
                stat: false,
                patch,
            },
        ),
        AgentAskIntent::Delta { file, patch } => handle_agent_delta(
            ctx,
            AgentDeltaArgs {
                selector,
                by_turn: false,
                by_operation: false,
                file,
                patch,
            },
        ),
        AgentAskIntent::New { file, patch } => handle_agent_new(
            ctx,
            AgentNewArgs {
                selector,
                file,
                patch,
            },
        ),
        AgentAskIntent::Files => handle_agent_files(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::File { path, patch } => handle_agent_file(
            ctx,
            AgentFileArgs {
                selector_or_path: selector,
                path: Some(path),
                patch,
            },
        ),
        AgentAskIntent::Workdir => handle_agent_workdir(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Checkpoints => {
            handle_agent_checkpoints(ctx, AgentSelectorArgs { selector })
        }
        AgentAskIntent::Timeline => handle_agent_timeline(
            ctx,
            AgentChangesArgs {
                selector,
                by_turn: false,
                by_operation: false,
            },
        ),
        AgentAskIntent::Turn => handle_agent_turn(
            ctx,
            AgentTurnArgs {
                selector_or_turn: Some(selector),
                turn: None,
                file: None,
                patch: false,
            },
        ),
        AgentAskIntent::Why(path) => handle_agent_why(
            ctx,
            AgentWhyArgs {
                selector_or_path: selector,
                path: Some(path),
            },
        ),
        AgentAskIntent::Review => handle_agent_review(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Focus => handle_agent_focus(
            ctx,
            AgentFocusArgs {
                selector,
                file: None,
                patch: false,
            },
        ),
        AgentAskIntent::View => handle_agent_view(ctx, AgentSelectorArgs { selector }),
    }
}

fn resolve_agent_ask_intent(question: &str) -> Result<AgentAskIntent> {
    let lowered = question.to_ascii_lowercase();
    let tokens = agent_ask_tokens(question);
    let lowered_tokens = tokens
        .iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let path = agent_ask_path(&tokens, &lowered_tokens);
    let wants_patch = lowered.contains("patch")
        || lowered.contains("diff")
        || lowered.contains("hunk")
        || lowered.contains("unified");
    let wants_turn_diff = wants_patch
        && (lowered.contains("turn")
            || lowered.contains("prompt")
            || lowered.contains("last response")
            || lowered.contains("latest response"));
    let mentions_turn_or_prompt = lowered.contains("turn")
        || lowered.contains("prompt")
        || lowered.contains("last response")
        || lowered.contains("latest response");
    let wants_prompt_change = mentions_turn_or_prompt
        && (lowered.contains("what changed")
            || lowered.contains("changed in")
            || lowered.contains("changed from")
            || lowered.contains("changes in")
            || lowered.contains("changes from")
            || lowered.contains("code changed")
            || agent_ask_has_any(&lowered_tokens, &["changed", "changes", "delta"]));

    if agent_ask_has_any(&lowered_tokens, &["why", "explain", "reason"]) {
        return path.map(AgentAskIntent::Why).ok_or_else(|| {
            Error::InvalidInput(
                "agent ask needs a file path for why/explain questions, for example `crabdb agent ask explain README.md`"
                    .to_string(),
            )
        });
    }
    if wants_turn_diff {
        return Ok(AgentAskIntent::TurnDiff {
            file: path,
            patch: true,
        });
    }
    if let Some(path) = path.clone() {
        if lowered.contains("which prompt")
            || lowered.contains("what prompt")
            || lowered.contains("which turn")
            || lowered.contains("what turn")
            || lowered.contains("which operation")
            || lowered.contains("what operation")
            || lowered.contains("which checkpoint")
            || lowered.contains("what checkpoint")
            || lowered.contains("which tool")
            || lowered.contains("what tool")
            || lowered.contains("who changed")
            || lowered.contains("who touched")
            || lowered.contains("what touched")
            || lowered.contains("what caused")
            || lowered.contains("where did")
            || lowered.contains("came from")
            || agent_ask_has_any(
                &lowered_tokens,
                &["touched", "caused", "introduced", "origin"],
            )
        {
            return Ok(AgentAskIntent::Why(path));
        }
    }
    if wants_prompt_change {
        return Ok(AgentAskIntent::Delta {
            file: path.clone(),
            patch: wants_patch,
        });
    }
    if agent_ask_has_any(&lowered_tokens, &["inspect", "file", "path"]) {
        if let Some(path) = path.clone() {
            return Ok(AgentAskIntent::File {
                path,
                patch: wants_patch,
            });
        }
    }
    if lowered.contains("changed files")
        || lowered.contains("which files")
        || lowered.contains("what files")
        || lowered.contains("edited files")
        || lowered.contains("touched files")
        || lowered.contains("files touched")
        || lowered.contains("where did it edit")
        || lowered.contains("where did the agent edit")
        || lowered.contains("what did it edit")
        || lowered.contains("what did the agent edit")
        || agent_ask_has_any(&lowered_tokens, &["files"])
    {
        return Ok(AgentAskIntent::Files);
    }
    if let Some(path) = path.clone() {
        if lowered.contains("what changed")
            || lowered.contains("changed")
            || lowered.contains("diff")
            || lowered.contains("patch")
        {
            return Ok(AgentAskIntent::File {
                path,
                patch: wants_patch,
            });
        }
    }
    if lowered.contains("what needs attention")
        || lowered.contains("needs my attention")
        || lowered.contains("need my attention")
        || lowered.contains("what is waiting")
        || lowered.contains("what's waiting")
        || lowered.contains("waiting on me")
        || lowered.contains("waiting for me")
        || lowered.contains("show inbox")
        || lowered.contains("agent inbox")
        || lowered.contains("task inbox")
        || lowered.contains("work queue")
        || lowered.contains("task queue")
        || lowered.contains("all agent tasks")
        || lowered.contains("all tasks")
        || matches!(lowered.trim(), "inbox" | "home" | "queue")
    {
        return Ok(AgentAskIntent::Inbox);
    }
    if lowered.contains("workdir")
        || lowered.contains("work dir")
        || lowered.contains("working directory")
        || lowered.contains("task directory")
        || lowered.contains("agent directory")
        || lowered.contains("materialized directory")
        || lowered.contains("materialized checkout")
        || lowered.contains("checkout path")
        || lowered.contains("local checkout")
        || lowered.contains("cd command")
        || lowered.contains("where is the code")
        || lowered.contains("where are the files")
        || lowered.contains("open folder")
        || lowered.contains("open directory")
    {
        return Ok(AgentAskIntent::Workdir);
    }
    if lowered.contains("transcript")
        || lowered.contains("conversation")
        || lowered.contains("message history")
        || lowered.contains("prompt history")
        || agent_ask_has_any(&lowered_tokens, &["chat", "messages"])
    {
        return Ok(AgentAskIntent::View);
    }
    if lowered.contains("turn") || lowered.contains("prompt") {
        return Ok(AgentAskIntent::Turn);
    }
    if lowered.contains("review first")
        || lowered.contains("inspect first")
        || lowered.contains("look at first")
        || lowered.contains("where should i start")
    {
        return Ok(AgentAskIntent::Focus);
    }
    if lowered.contains("review plan")
        || lowered.contains("review dashboard")
        || lowered.contains("review priority")
        || lowered.contains("review priorities")
        || lowered.contains("open review")
        || lowered.contains("start review")
        || lowered.contains("review this task")
        || lowered.contains("review this agent task")
        || lowered.contains("review task")
        || lowered.contains("task review")
        || lowered.contains("what should i review")
        || lowered.contains("what to review")
        || lowered.contains("show review")
    {
        return Ok(AgentAskIntent::Review);
    }
    if lowered.contains("what should")
        || lowered.contains("what now")
        || lowered.contains("next")
        || agent_ask_has_any(&lowered_tokens, &["todo", "help"])
    {
        return Ok(AgentAskIntent::Next);
    }
    if lowered.contains("can land")
        || lowered.contains("can apply")
        || lowered.contains("can merge")
        || lowered.contains("can ship")
        || lowered.contains("safe")
        || lowered.contains("ready")
        || lowered.contains("preflight")
        || agent_ask_has_any(&lowered_tokens, &["land", "apply", "merge", "ship"])
    {
        return Ok(AgentAskIntent::Ready);
    }
    if lowered.contains("recover")
        || lowered.contains("stuck")
        || lowered.contains("sideways")
        || lowered.contains("blocked")
        || lowered.contains("failed")
        || lowered.contains("failure")
    {
        return Ok(AgentAskIntent::Diagnose);
    }
    if lowered.contains("rewind")
        || lowered.contains("checkpoint")
        || lowered.contains("undo")
        || lowered.contains("roll back")
        || lowered.contains("rollback")
    {
        return Ok(AgentAskIntent::Checkpoints);
    }
    if lowered.contains("just changed")
        || lowered.contains("last change")
        || lowered.contains("latest change")
        || lowered.contains("recent change")
        || agent_ask_has_any(&lowered_tokens, &["last"])
    {
        return Ok(AgentAskIntent::Delta {
            file: None,
            patch: wants_patch,
        });
    }
    if lowered.contains("since i looked")
        || lowered.contains("since reviewed")
        || lowered.contains("new changes")
        || lowered.contains("what changed")
    {
        return Ok(AgentAskIntent::New {
            file: None,
            patch: wants_patch,
        });
    }
    if lowered.contains("all changes")
        || lowered.contains("change cards")
        || agent_ask_has_any(&lowered_tokens, &["changes"])
    {
        return Ok(AgentAskIntent::Changes);
    }
    if wants_patch {
        return Ok(AgentAskIntent::Delta {
            file: path,
            patch: true,
        });
    }
    if lowered.contains("timeline") || lowered.contains("chronological") || lowered.contains("when")
    {
        return Ok(AgentAskIntent::Timeline);
    }
    if lowered.contains("turn") || lowered.contains("prompt") {
        return Ok(AgentAskIntent::Turn);
    }
    if lowered.contains("what did")
        || lowered.contains("what happened")
        || lowered.contains("what was done")
        || lowered.contains("what got done")
    {
        return Ok(AgentAskIntent::Story);
    }
    if agent_ask_has_any(&lowered_tokens, &["tools", "tool", "commands", "command"]) {
        return Ok(AgentAskIntent::View);
    }
    if lowered.contains("test status")
        || lowered.contains("validation")
        || lowered.contains("what tests")
        || lowered.contains("which tests")
        || lowered.contains("do i need tests")
        || lowered.contains("need tests")
    {
        return Ok(AgentAskIntent::Validate);
    }
    if lowered.contains("summary") || lowered.contains("overview") || lowered.contains("cockpit") {
        return Ok(AgentAskIntent::Summary);
    }
    if lowered.contains("brief") {
        return Ok(AgentAskIntent::Brief);
    }
    if lowered.contains("story") || lowered.contains("happened") {
        return Ok(AgentAskIntent::Story);
    }
    if lowered.contains("risk") {
        return Ok(AgentAskIntent::Risk);
    }
    if lowered.contains("receipt") {
        return Ok(AgentAskIntent::Receipt);
    }
    if lowered.contains("pr") || lowered.contains("pull request") {
        return Ok(AgentAskIntent::Pr);
    }
    if lowered.contains("review") {
        return Ok(AgentAskIntent::Review);
    }
    if lowered.contains("focus") || lowered.contains("first file") {
        return Ok(AgentAskIntent::Focus);
    }
    if lowered.contains("view") || lowered.contains("transcript") {
        return Ok(AgentAskIntent::View);
    }

    Err(Error::InvalidInput(format!(
        "could not route agent question `{question}`; try `what should I do next`, `what changed`, `what just changed`, `is it safe to land`, `recover`, `changed files`, or `explain README.md`"
    )))
}

fn agent_ask_tokens(question: &str) -> Vec<String> {
    question
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\''
                            | '`'
                            | ','
                            | ';'
                            | ':'
                            | '?'
                            | '!'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                    )
                })
                .to_string()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

fn agent_ask_has_any(tokens: &[String], words: &[&str]) -> bool {
    tokens
        .iter()
        .any(|token| words.iter().any(|word| token == word))
}

fn agent_ask_path(tokens: &[String], lowered_tokens: &[String]) -> Option<String> {
    for (idx, token) in lowered_tokens.iter().enumerate() {
        if matches!(
            token.as_str(),
            "why" | "explain" | "inspect" | "file" | "path"
        ) {
            if let Some(path) = tokens
                .get(idx + 1)
                .and_then(|value| agent_ask_clean_path(value))
            {
                return Some(path);
            }
        }
    }
    tokens.iter().find_map(|token| agent_ask_clean_path(token))
}

fn agent_ask_clean_path(token: &str) -> Option<String> {
    let value = token.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | ',' | ';' | ':' | '?' | '!' | '(' | ')' | '[' | ']' | '{' | '}'
        )
    });
    if value.is_empty() || value.starts_with("--") {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "what"
            | "changed"
            | "change"
            | "changes"
            | "why"
            | "explain"
            | "inspect"
            | "file"
            | "path"
            | "in"
            | "for"
            | "the"
            | "latest"
            | "task"
    ) {
        return None;
    }
    if value.contains('/') || value.contains('\\') || value.contains('.') {
        return Some(value.to_string());
    }
    None
}

fn handle_agent_status(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_status()?;
    render_agent_status(&report, ctx.json, ctx.quiet)
}

fn handle_agent_inbox(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_inbox()?;
    render_agent_inbox(&report, ctx.json, ctx.quiet)
}

fn handle_agent_next(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_next(&args.selector)?;
    render_agent_next(&report, ctx.json, ctx.quiet)
}

fn handle_agent_list(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.list_agent_tasks()?;
    render_agent_list(&report, ctx.json, ctx.quiet)
}

fn handle_agent_brief(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_brief(&args.selector)?;
    render_agent_brief(&report, ctx.json, ctx.quiet)
}

fn handle_agent_summary(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_summary(&args.selector)?;
    render_agent_summary(&report, ctx.json, ctx.quiet)
}

fn handle_agent_validate(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_validate(&args.selector)?;
    render_agent_validate(&report, ctx.json, ctx.quiet)
}

fn handle_agent_report(ctx: &RuntimeContext, args: AgentReportArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_report(&args.selector)?;
    render_agent_report(&report, ctx.json, ctx.quiet, args.markdown)
}

fn handle_agent_receipt(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_receipt(&args.selector)?;
    render_agent_receipt(&report, ctx.json, ctx.quiet)
}

fn handle_agent_pr(ctx: &RuntimeContext, args: AgentPrArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_pr_draft(&args.selector)?;
    render_agent_pr(
        &report,
        ctx.json,
        ctx.quiet,
        args.title_only,
        args.body_only,
    )
}

fn handle_agent_story(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_story(&args.selector)?;
    render_agent_story(&report, ctx.json, ctx.quiet)
}

fn handle_agent_risk(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_risk(&args.selector)?;
    render_agent_risk(&report, ctx.json, ctx.quiet)
}

fn handle_agent_ready(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_ready(&args.selector)?;
    render_agent_ready(&report, ctx.json, ctx.quiet)
}

fn handle_agent_diagnose(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_diagnose(&args.selector)?;
    render_agent_diagnose(&report, ctx.json, ctx.quiet)
}

fn handle_agent_compare(ctx: &RuntimeContext, args: AgentCompareArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_compare(&args.left, &args.right)?;
    render_agent_compare(&report, ctx.json, ctx.quiet)
}

enum AgentGateKind {
    Test,
    Eval,
}

fn handle_agent_gate(ctx: &RuntimeContext, args: AgentGateArgs, kind: AgentGateKind) -> Result<()> {
    let mut db = open_db(ctx)?;
    let options = LaneGateOptions {
        suite: args.suite,
        score: args.score,
        threshold: args.threshold,
    };
    let report = match kind {
        AgentGateKind::Test => db.run_agent_test_with_options(
            &args.selector,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
        AgentGateKind::Eval => db.run_agent_eval_with_options(
            &args.selector,
            args.command,
            args.turn.as_deref(),
            args.timeout_secs,
            options,
        )?,
    };
    let render_result = render_lane_test(&report, ctx.json, ctx.quiet);
    if render_result.is_ok() && !report.success {
        std::process::exit(command_failure_exit_code(report.exit_code));
    }
    render_result
}

fn handle_agent_workdir(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_workdir(&args.selector)?;
    render_agent_workdir(&report, ctx.json, ctx.quiet)
}

fn handle_agent_view(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_task_view(&args.selector)?;
    render_agent_view(&report, ctx.json, ctx.quiet)
}

fn handle_agent_changes(ctx: &RuntimeContext, args: AgentChangesArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_changes(&args.selector, args.by_operation)?;
    render_agent_changes(&report, ctx.json, ctx.quiet)
}

fn handle_agent_delta(ctx: &RuntimeContext, args: AgentDeltaArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let _ = args.by_turn;
    let report = db.agent_delta(
        &args.selector,
        args.by_operation,
        args.file.as_deref(),
        args.patch,
    )?;
    render_agent_delta(&report, ctx.json, ctx.quiet)
}

fn handle_agent_new(ctx: &RuntimeContext, args: AgentNewArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_new(&args.selector, args.file.as_deref(), args.patch)?;
    render_agent_new(&report, ctx.json, ctx.quiet)
}

fn handle_agent_mark_reviewed(ctx: &RuntimeContext, args: AgentMarkReviewedArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_mark_reviewed(&args.selector, args.note)?;
    render_agent_mark_reviewed(&report, ctx.json, ctx.quiet)
}

fn handle_agent_change(ctx: &RuntimeContext, args: AgentChangeArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, change_selector) = agent_change_selector_args(&args);
    let report = db.agent_change_set(&selector, &change_selector, args.patch)?;
    render_agent_change_set(&report, ctx.json, ctx.quiet)
}

fn handle_agent_timeline(ctx: &RuntimeContext, args: AgentChangesArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_timeline(&args.selector, args.by_operation)?;
    render_agent_timeline(&report, ctx.json, ctx.quiet)
}

fn handle_agent_files(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_files(&args.selector)?;
    render_agent_files(&report, ctx.json, ctx.quiet)
}

fn handle_agent_file(ctx: &RuntimeContext, args: AgentFileArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, path) = agent_file_selector_args(&args);
    let report = db.agent_file(&selector, &path, args.patch)?;
    render_agent_file(&report, ctx.json, ctx.quiet)
}

fn handle_agent_checkpoints(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_checkpoints(&args.selector)?;
    render_agent_checkpoints(&report, ctx.json, ctx.quiet)
}

fn handle_agent_why(ctx: &RuntimeContext, args: AgentWhyArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, path) = match args.path {
        Some(path) => (args.selector_or_path, path),
        None => ("latest".to_string(), args.selector_or_path),
    };
    let report = db.agent_why(&selector, &path)?;
    render_agent_why(&report, ctx.json, ctx.quiet)
}

fn agent_change_selector_args(args: &AgentChangeArgs) -> (String, String) {
    match (&args.selector_or_change, &args.change) {
        (Some(selector), Some(change)) => (selector.clone(), change.clone()),
        (Some(change), None) => ("latest".to_string(), change.clone()),
        (None, None) => ("latest".to_string(), "1".to_string()),
        (None, Some(change)) => ("latest".to_string(), change.clone()),
    }
}

fn agent_file_selector_args(args: &AgentFileArgs) -> (String, String) {
    match &args.path {
        Some(path) => (args.selector_or_path.clone(), path.clone()),
        None => ("latest".to_string(), args.selector_or_path.clone()),
    }
}

fn handle_agent_turn(ctx: &RuntimeContext, args: AgentTurnArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, turn) = resolve_agent_turn_cli_args(&db, &args);
    let report = db.agent_turn(&selector, &turn, args.file.as_deref(), args.patch)?;
    render_agent_turn(&report, ctx.json, ctx.quiet)
}

fn handle_agent_turn_diff(ctx: &RuntimeContext, args: AgentTurnDiffArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_diff(
        &args.selector,
        args.turn.as_deref(),
        None,
        None,
        args.turn.is_none(),
        args.file.as_deref(),
        args.patch,
    )?;
    render_agent_diff(&report, ctx.json, ctx.quiet, args.stat)
}

fn resolve_agent_turn_cli_args(db: &crabdb::CrabDb, args: &AgentTurnArgs) -> (String, String) {
    match (&args.selector_or_turn, &args.turn) {
        (None, None) => ("latest".to_string(), "last".to_string()),
        (Some(value), None) => {
            if value == "latest" || db.agent_task_view(value).is_ok() {
                (value.clone(), "last".to_string())
            } else {
                ("latest".to_string(), value.clone())
            }
        }
        (Some(selector), Some(turn)) => (selector.clone(), turn.clone()),
        (None, Some(turn)) => ("latest".to_string(), turn.clone()),
    }
}

fn handle_agent_diff(ctx: &RuntimeContext, args: AgentDiffArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_diff(
        &args.selector,
        args.turn.as_deref(),
        args.operation.as_deref(),
        args.checkpoint.as_deref(),
        args.last_turn,
        args.file.as_deref(),
        args.patch,
    )?;
    render_agent_diff(&report, ctx.json, ctx.quiet, args.stat)
}

fn handle_agent_review(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_review(&args.selector)?;
    render_agent_review(&report, ctx.json, ctx.quiet)
}

fn handle_agent_focus(ctx: &RuntimeContext, args: AgentFocusArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_focus(&args.selector, args.file.as_deref(), args.patch)?;
    render_agent_focus(&report, ctx.json, ctx.quiet)
}

fn handle_agent_apply(ctx: &RuntimeContext, args: AgentApplyArgs) -> Result<()> {
    let _ = args.into_current_git_branch;
    let mut db = open_db(ctx)?;
    let report = db.agent_apply(&args.selector, args.dry_run, args.message)?;
    render_agent_apply(&report, ctx.json, ctx.quiet)
}

fn handle_agent_rewind(ctx: &RuntimeContext, args: AgentRewindArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_rewind(&args.selector, &args.target)?;
    render_lane_rewind(&report, ctx.json, ctx.quiet)
}

fn handle_agent_undo(ctx: &RuntimeContext, args: AgentUndoArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_undo(
        &args.selector,
        args.last_turn,
        args.turn.as_deref(),
        args.prompt.as_deref(),
        args.last_operation,
    )?;
    render_lane_rewind(&report, ctx.json, ctx.quiet)
}

fn handle_agent_doctor(ctx: &RuntimeContext, args: AgentDoctorArgs) -> Result<()> {
    let mut checks = Vec::new();
    let mut status = "ok";
    match open_db(ctx) {
        Ok(_) => checks.push(serde_json::json!({
            "name": "workspace",
            "status": "ok",
            "message": "CrabDB workspace opened"
        })),
        Err(err) => {
            status = "failed";
            checks.push(serde_json::json!({
                "name": "workspace",
                "status": "failed",
                "message": err.to_string()
            }));
        }
    }
    let profile = crabdb::acp::acp_provider_profile(&args.provider)?;
    if profile.available {
        checks.push(serde_json::json!({
            "name": "provider",
            "status": "ok",
            "message": format!("{} profile available", profile.agent)
        }));
    } else {
        status = "failed";
        checks.push(serde_json::json!({
            "name": "provider",
            "status": "failed",
            "message": profile.notes.join("; ")
        }));
    }
    let report = serde_json::json!({
        "status": status,
        "provider": profile.agent,
        "checks": checks,
        "suggestions": [
            {
                "command": format!("crabdb agent setup --provider {} --editor vscode", args.provider),
                "reason": "configure an ACP editor with fresh agent lanes"
            }
        ]
    });
    if ctx.json {
        return render_json(&report);
    }
    if !ctx.quiet {
        println!("Agent doctor: {status}");
        for check in checks {
            println!(
                "[{}] {}: {}",
                check["status"].as_str().unwrap_or("-"),
                check["name"].as_str().unwrap_or("-"),
                check["message"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

fn default_acp_upstream_command(provider: &str, _lane: &str) -> Result<Vec<String>> {
    let profile = crabdb::acp::acp_provider_profile(provider)?;
    let separator = profile
        .relay_command
        .iter()
        .position(|part| part == "--")
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "provider `{provider}` does not define an ACP upstream command"
            ))
        })?;
    Ok(profile.relay_command[separator + 1..].to_vec())
}

fn default_terminal_agent_command(provider: &str) -> Result<Vec<String>> {
    match provider {
        "claude-code" | "claude" => Ok(vec!["claude".to_string()]),
        other => Err(Error::InvalidInput(format!(
            "unsupported terminal agent provider `{other}`; supported providers: claude-code"
        ))),
    }
}
