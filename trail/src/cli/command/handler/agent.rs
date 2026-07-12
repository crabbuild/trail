use super::*;
use crate::cli::command::render::render_agent_timeline;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use trail::agent_hooks::{
    apply_agent_hook_install_plan, build_agent_hook_install_plan, inspect_agent_hook_installation,
    redact_agent_hook_payload, remove_agent_hook_installation, rollback_agent_hook_install_plan,
    AgentHookInstallRequest, AgentHookInstallScope, AgentProviderRegistry,
};
use trail::{
    AgentCaptureTransport, AgentContinueReport, AgentHookReceiptInput, AgentReviewAction,
    AgentRunReport, LaneWorkdirMode, StatusSuggestion,
};

pub(super) fn handle_agent_command(ctx: &RuntimeContext, agent: AgentCommand) -> Result<()> {
    match agent.command {
        None => handle_agent_home(ctx),
        Some(AgentSubcommand::Hook(args)) => handle_agent_hook(ctx, args),
        Some(AgentSubcommand::Hooks(args)) => handle_agent_hooks(ctx, args),
        Some(AgentSubcommand::Capture(args)) => handle_agent_capture(ctx, args),
        Some(AgentSubcommand::Artifacts(args)) => handle_agent_artifacts(ctx, args),
        Some(AgentSubcommand::Provenance(args)) => handle_agent_provenance(ctx, args),
        Some(AgentSubcommand::Attest(args)) => handle_agent_attest(ctx, args),
        Some(AgentSubcommand::Learnings(args)) => handle_agent_learnings(ctx, args),
        Some(AgentSubcommand::Export(args)) => handle_agent_trace_export(ctx, args),
        Some(AgentSubcommand::GitLink(args)) => handle_agent_git_link(ctx, args),
        Some(AgentSubcommand::Setup(args)) => handle_agent_setup(ctx, args),
        Some(AgentSubcommand::Acp(args)) => handle_agent_acp(ctx, args),
        Some(AgentSubcommand::Start(args)) => handle_agent_start(ctx, args),
        Some(AgentSubcommand::Continue(args)) => handle_agent_continue(ctx, args),
        Some(AgentSubcommand::Guide(args)) => handle_agent_guide(ctx, args),
        Some(AgentSubcommand::Ask(args)) => handle_agent_ask(ctx, args),
        Some(AgentSubcommand::Next(args)) => handle_agent_next(ctx, args),
        Some(AgentSubcommand::Status) => handle_agent_status(ctx),
        Some(AgentSubcommand::Dashboard(args)) => handle_agent_dashboard(ctx, args),
        Some(AgentSubcommand::ReviewData(args)) => handle_agent_review_data(ctx, args),
        Some(AgentSubcommand::Action(args)) => handle_agent_action(ctx, args),
        Some(AgentSubcommand::ReviewFlow(args)) => handle_agent_review_flow(ctx, args),
        Some(AgentSubcommand::Inbox(args)) => handle_agent_inbox(ctx, args),
        Some(AgentSubcommand::Board(args)) => handle_agent_board(ctx, args),
        Some(AgentSubcommand::Stack(args)) => handle_agent_stack(ctx, args),
        Some(AgentSubcommand::Brief(args)) => handle_agent_brief(ctx, args),
        Some(AgentSubcommand::Summary(args)) => handle_agent_summary(ctx, args),
        Some(AgentSubcommand::Validate(args)) => handle_agent_validate(ctx, args),
        Some(AgentSubcommand::TestPlan(args)) => handle_agent_test_plan(ctx, args),
        Some(AgentSubcommand::Report(args)) => handle_agent_report(ctx, args),
        Some(AgentSubcommand::Handoff(args)) => handle_agent_handoff(ctx, args),
        Some(AgentSubcommand::Receipt(args)) => handle_agent_receipt(ctx, args),
        Some(AgentSubcommand::Pr(args)) => handle_agent_pr(ctx, args),
        Some(AgentSubcommand::Story(args)) => handle_agent_story(ctx, args),
        Some(AgentSubcommand::Tools(args)) => handle_agent_tools(ctx, args),
        Some(AgentSubcommand::Impact(args)) => handle_agent_impact(ctx, args),
        Some(AgentSubcommand::ReviewMap(args)) => handle_agent_review_map(ctx, args),
        Some(AgentSubcommand::Risk(args)) => handle_agent_risk(ctx, args),
        Some(AgentSubcommand::Confidence(args)) => handle_agent_confidence(ctx, args),
        Some(AgentSubcommand::Ready(args)) => handle_agent_ready(ctx, args),
        Some(AgentSubcommand::Diagnose(args)) => handle_agent_diagnose(ctx, args),
        Some(AgentSubcommand::Compare(args)) => handle_agent_compare(ctx, args),
        Some(AgentSubcommand::Test(args)) => handle_agent_gate(ctx, args, AgentGateKind::Test),
        Some(AgentSubcommand::Eval(args)) => handle_agent_gate(ctx, args, AgentGateKind::Eval),
        Some(AgentSubcommand::Workdir(args)) => handle_agent_workdir(ctx, args),
        Some(AgentSubcommand::List(args)) => handle_agent_list(ctx, args),
        Some(AgentSubcommand::View(args)) => handle_agent_view(ctx, args),
        Some(AgentSubcommand::Changes(args)) => handle_agent_changes(ctx, args),
        Some(AgentSubcommand::Delta(args)) => handle_agent_delta(ctx, args),
        Some(AgentSubcommand::New(args)) => handle_agent_new(ctx, args),
        Some(AgentSubcommand::MarkReviewed(args)) => handle_agent_mark_reviewed(ctx, args),
        Some(AgentSubcommand::MarkFileReviewed(args)) => handle_agent_mark_file_reviewed(ctx, args),
        Some(AgentSubcommand::Archive(args)) => handle_agent_archive(ctx, args, true),
        Some(AgentSubcommand::Unarchive(args)) => handle_agent_archive(ctx, args, false),
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
        Some(AgentSubcommand::Open(args)) => handle_agent_open(ctx, args),
        Some(AgentSubcommand::Apply(args)) => handle_agent_apply(ctx, args),
        Some(AgentSubcommand::Finish(args)) => handle_agent_finish(ctx, args),
        Some(AgentSubcommand::Undo(args)) => handle_agent_undo(ctx, args),
        Some(AgentSubcommand::Rewind(args)) => handle_agent_rewind(ctx, args),
        Some(AgentSubcommand::Doctor(args)) => handle_agent_doctor(ctx, args),
    }
}

fn handle_agent_hook(ctx: &RuntimeContext, args: AgentHookCommand) -> Result<()> {
    match args.command {
        AgentHookSubcommand::Receive(args) => handle_agent_hook_receive(ctx, args),
    }
}

fn handle_agent_capture(ctx: &RuntimeContext, args: AgentCaptureCommand) -> Result<()> {
    let mut db = open_db(ctx)?;
    match args.command {
        AgentCaptureSubcommand::Begin(args) => {
            let workdir = args
                .workdir
                .unwrap_or(std::env::current_dir()?)
                .canonicalize()?;
            let report = db.begin_agent_capture_run(trail::AgentCaptureRunInput {
                lane: args.lane,
                workdir: workdir.to_string_lossy().into_owned(),
                owner_agent: args.owner,
                owner_session_id: args.session,
                executor_agent: args.executor,
                work_item_id: args.work_item,
                lease_ms: args.ttl_ms,
                metadata_json: Some("{\"source\":\"cli\"}".to_string()),
            })?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("capture run {} started", report.capture_run_id),
            )
        }
        AgentCaptureSubcommand::Renew(args) => {
            let report =
                db.renew_agent_capture_run(&args.run_id, &args.owner, &args.session, args.ttl_ms)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("capture run {} renewed", report.capture_run_id),
            )
        }
        AgentCaptureSubcommand::End(args) => {
            let report = db.end_agent_capture_run(&args.run_id, &args.owner, &args.session)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("capture run {} ended", report.capture_run_id),
            )
        }
        AgentCaptureSubcommand::Status(args) => {
            let reports = db.list_agent_capture_runs_page(!args.all, args.offset, args.limit)?;
            let summary = format!("{} capture run(s)", reports.len());
            render_agent_hooks_value(ctx, serde_json::to_value(reports)?, summary)
        }
        AgentCaptureSubcommand::Reconcile => {
            let report = db.reconcile_expired_agent_capture_runs()?;
            let summary = format!(
                "expired {} run(s); interrupted {} session mapping(s)",
                report.expired_run_ids.len(),
                report.interrupted_mapping_ids.len()
            );
            render_agent_hooks_value(ctx, serde_json::to_value(report)?, summary)
        }
    }
}

fn handle_agent_artifacts(ctx: &RuntimeContext, args: AgentArtifactsArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let artifacts =
        db.list_lane_artifacts_page(&args.session, args.turn.as_deref(), args.offset, args.limit)?;
    let summary = format!("{} artifact(s) for {}", artifacts.len(), args.session);
    render_agent_hooks_value(ctx, serde_json::to_value(artifacts)?, summary)
}

fn handle_agent_provenance(ctx: &RuntimeContext, args: AgentProvenanceArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (nodes, edges) = db.list_session_provenance_page(&args.session, args.offset, args.limit)?;
    let summary = format!(
        "{} provenance node(s), {} edge(s) for {}",
        nodes.len(),
        edges.len(),
        args.session
    );
    render_agent_hooks_value(
        ctx,
        serde_json::json!({"session_id":args.session,"nodes":nodes,"edges":edges}),
        summary,
    )
}

fn handle_agent_attest(ctx: &RuntimeContext, args: AgentAttestCommand) -> Result<()> {
    let mut db = open_db(ctx)?;
    match args.command {
        AgentAttestSubcommand::Create(args) => {
            let report = db.create_session_attestation(
                &args.session,
                &args.policy,
                Some(serde_json::json!({"source":"cli"})),
            )?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("attestation {} created", report.attestation_id),
            )
        }
        AgentAttestSubcommand::List(args) => {
            let reports =
                db.list_session_attestations_page(&args.session, args.offset, args.limit)?;
            let summary = format!("{} attestation(s) for {}", reports.len(), args.session);
            render_agent_hooks_value(ctx, serde_json::to_value(reports)?, summary)
        }
        AgentAttestSubcommand::Show(args) => {
            let report = db.session_attestation(&args.attestation_id)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("attestation {} ({})", report.attestation_id, report.status),
            )
        }
        AgentAttestSubcommand::Verify(args) => {
            let report = db.verify_session_attestation(&args.attestation_id)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!(
                    "attestation {}: {}",
                    report.attestation_id,
                    if report.valid { "valid" } else { "invalid" }
                ),
            )
        }
    }
}

fn handle_agent_learnings(ctx: &RuntimeContext, args: AgentLearningsCommand) -> Result<()> {
    let mut db = open_db(ctx)?;
    match args.command {
        AgentLearningsSubcommand::List(args) => {
            let reports = db.list_learnings_page(
                args.session.as_deref(),
                args.status.as_deref(),
                args.offset,
                args.limit,
            )?;
            let summary = format!("{} learning(s)", reports.len());
            render_agent_hooks_value(ctx, serde_json::to_value(reports)?, summary)
        }
        AgentLearningsSubcommand::Accept(args) => {
            let report = db.review_learning(&args.learning_id, true, &args.reviewer)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("learning {} accepted", report.learning_id),
            )
        }
        AgentLearningsSubcommand::Reject(args) => {
            let report = db.review_learning(&args.learning_id, false, &args.reviewer)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("learning {} rejected", report.learning_id),
            )
        }
    }
}

fn handle_agent_trace_export(ctx: &RuntimeContext, args: AgentTraceExportArgs) -> Result<()> {
    use std::io::Write;

    let db = open_db(ctx)?;
    let trace = db.export_agent_trace(&args.session, args.attachments)?;
    let bytes = trace.to_canonical_json()?;
    if let Some(output) = args.output {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&output)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        render_document(
            &TerminalDocument::new("Exported portable agent trace", UiTone::Success).block(
                UiBlock::Metadata(vec![("Path".to_string(), output.display().to_string())]),
            ),
            &ctx.render,
        )?;
    } else {
        std::io::stdout().write_all(&bytes)?;
    }
    Ok(())
}

fn handle_agent_git_link(ctx: &RuntimeContext, args: AgentGitLinkCommand) -> Result<()> {
    let mut db = open_db(ctx)?;
    match args.command {
        AgentGitLinkSubcommand::Link(args) => {
            let report = db.link_git_commit_to_agent(trail::GitAgentLinkInput {
                session_id: args.session,
                turn_id: args.turn,
                git_commit: args.git_commit,
                from_change: args.from_change,
                through_change: args.through_change,
                confidence: args.confidence,
                source: args.source,
                metadata: Some(serde_json::json!({"source":"cli"})),
            })?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!(
                    "linked Git commit {} to session {}",
                    report.git_commit, report.session_id
                ),
            )
        }
        AgentGitLinkSubcommand::List(args) => {
            let reports = db.list_git_agent_links_page(&args.session, args.offset, args.limit)?;
            let summary = format!("{} Git link(s) for {}", reports.len(), args.session);
            render_agent_hooks_value(ctx, serde_json::to_value(reports)?, summary)
        }
    }
}

fn handle_agent_hooks(ctx: &RuntimeContext, args: AgentHooksCommand) -> Result<()> {
    match args.command {
        AgentHooksSubcommand::Add(args) => handle_agent_hooks_add(ctx, args),
        AgentHooksSubcommand::Remove(args) => handle_agent_hooks_remove(ctx, args),
        AgentHooksSubcommand::List(args) => handle_agent_hooks_list(ctx, args),
        AgentHooksSubcommand::Status(args) => handle_agent_hooks_status(ctx, args),
        AgentHooksSubcommand::Doctor(args) => handle_agent_hooks_doctor(ctx, args),
        AgentHooksSubcommand::Events(args) => handle_agent_hooks_events(ctx, args),
        AgentHooksSubcommand::Replay(args) => handle_agent_hooks_replay(ctx, args),
        AgentHooksSubcommand::Retry(args) => {
            let mut db = open_db(ctx)?;
            let report = db.retry_agent_hook_receipt(&args.receipt)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("receipt {} queued for retry", report.receipt_id),
            )
        }
        AgentHooksSubcommand::Discard(args) => {
            let mut db = open_db(ctx)?;
            let report = db.discard_agent_hook_receipt(&args.receipt)?;
            render_agent_hooks_value(
                ctx,
                serde_json::to_value(&report)?,
                format!("receipt {} discarded", report.receipt_id),
            )
        }
    }
}

fn handle_agent_hooks_add(ctx: &RuntimeContext, args: AgentHooksAddArgs) -> Result<()> {
    let registry = AgentProviderRegistry::built_in()?;
    registry.resolve(&args.provider)?;
    let mut db = open_db(ctx)?;
    let scope = agent_hook_scope(args.scope);
    let home = agent_hooks_home_dir();
    let plan = build_agent_hook_install_plan(AgentHookInstallRequest {
        registry: &registry,
        provider: &args.provider,
        workspace_id: &db.config().workspace.id.0,
        workspace_root: db.workspace_root(),
        home_dir: home.as_deref(),
        scope,
        force: args.force,
    })?;
    let report = apply_agent_hook_install_plan(&plan, args.dry_run)?;
    if !args.dry_run {
        if let Err(error) = db.record_agent_hook_installation(&plan, args.lane.as_deref()) {
            rollback_agent_hook_install_plan(&plan).map_err(|rollback| {
                Error::Conflict(format!(
                    "failed to record hook installation ({error}); rollback also failed: {rollback}"
                ))
            })?;
            return Err(error);
        }
    }
    render_agent_hooks_value(
        ctx,
        serde_json::to_value(&report)?,
        format!(
            "{} {} hooks at {} ({:?}){}",
            if args.dry_run {
                "Would configure"
            } else {
                "Configured"
            },
            report.provider,
            report.config_path.display(),
            report.action,
            if args.dry_run { " [dry run]" } else { "" }
        ),
    )
}

fn handle_agent_hooks_remove(ctx: &RuntimeContext, args: AgentHooksRemoveArgs) -> Result<()> {
    let registry = AgentProviderRegistry::built_in()?;
    let provider = registry.resolve(&args.provider)?.provider.clone();
    let mut db = open_db(ctx)?;
    let scope = agent_hook_scope(args.scope);
    let record = db
        .list_agent_hook_installations(Some(&provider))?
        .into_iter()
        .find(|record| record.scope == scope && record.status == "installed")
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "no installed {provider} hooks were recorded for {} scope",
                scope.as_str()
            ))
        })?;
    let home = agent_hooks_home_dir();
    let plan = build_agent_hook_install_plan(AgentHookInstallRequest {
        registry: &registry,
        provider: &provider,
        workspace_id: &db.config().workspace.id.0,
        workspace_root: db.workspace_root(),
        home_dir: home.as_deref(),
        scope,
        force: true,
    })?;
    if plan.installation_id != record.installation_id || plan.config_path != record.config_path {
        return Err(Error::Conflict(
            "recorded hook ownership does not match the current provider install target"
                .to_string(),
        ));
    }
    let report = remove_agent_hook_installation(&plan, &record.config_after_digest, args.dry_run)?;
    if !args.dry_run {
        db.mark_agent_hook_installation_removed(&record.installation_id)?;
    }
    render_agent_hooks_value(
        ctx,
        serde_json::to_value(&report)?,
        format!(
            "{} {} hooks from {}{}",
            if args.dry_run {
                "Would remove"
            } else {
                "Removed"
            },
            provider,
            report.config_path.display(),
            if args.dry_run { " [dry run]" } else { "" }
        ),
    )
}

fn handle_agent_hooks_list(ctx: &RuntimeContext, args: AgentHooksListArgs) -> Result<()> {
    let registry = AgentProviderRegistry::built_in()?;
    let db = open_db(ctx)?;
    let installations = db.list_agent_hook_installations(None)?;
    let rows = registry
        .list()
        .into_iter()
        .filter_map(|manifest| {
            let matching = installations
                .iter()
                .filter(|record| {
                    record.provider == manifest.provider && record.status == "installed"
                })
                .collect::<Vec<_>>();
            if args.installed && matching.is_empty() {
                return None;
            }
            Some(serde_json::json!({
                "provider": manifest.provider,
                "display_name": manifest.display_name,
                "support": manifest.support,
                "deployment": manifest.deployment,
                "project_config_path": manifest.project_config_path,
                "installations": matching,
            }))
        })
        .collect::<Vec<_>>();
    if ctx.json {
        render_json(&rows)?;
    } else if !ctx.quiet {
        let table = UiTable::new(
            vec![
                UiColumn::left("PROVIDER", 0, 12),
                UiColumn::left("SUPPORT", 1, 10),
                UiColumn::right("INSTALLED", 0, 9),
            ],
            rows.iter()
                .map(|row| {
                    vec![
                        row["display_name"]
                            .as_str()
                            .or_else(|| row["provider"].as_str())
                            .unwrap_or_default()
                            .to_string(),
                        state_label(row["support"].as_str().unwrap_or("known")),
                        row["installations"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or_default()
                            .to_string(),
                    ]
                })
                .collect(),
        );
        render_document(
            &TerminalDocument::new("Agent hook providers", UiTone::Neutral)
                .block(UiBlock::Table(table)),
            &ctx.render,
        )?;
    }
    Ok(())
}

fn handle_agent_hooks_status(ctx: &RuntimeContext, args: AgentHooksProviderArgs) -> Result<()> {
    let registry = AgentProviderRegistry::built_in()?;
    let provider = registry.resolve(&args.provider)?.provider.clone();
    let db = open_db(ctx)?;
    let records = db.list_agent_hook_installations(Some(&provider))?;
    let home = agent_hooks_home_dir();
    let mut statuses = Vec::new();
    for record in records {
        let plan = build_agent_hook_install_plan(AgentHookInstallRequest {
            registry: &registry,
            provider: &provider,
            workspace_id: &db.config().workspace.id.0,
            workspace_root: db.workspace_root(),
            home_dir: home.as_deref(),
            scope: record.scope,
            force: true,
        })?;
        statuses.push(serde_json::json!({
            "record": record,
            "filesystem_status": inspect_agent_hook_installation(&plan)?,
        }));
    }
    let summary = if statuses.is_empty() {
        format!("{provider}: not installed")
    } else {
        format!("{provider}: {} recorded installation(s)", statuses.len())
    };
    render_agent_hooks_value(
        ctx,
        serde_json::json!({"provider": provider, "installations": statuses}),
        summary,
    )
}

fn handle_agent_hooks_doctor(ctx: &RuntimeContext, args: AgentHooksDoctorArgs) -> Result<()> {
    let _all = args.all;
    let registry = AgentProviderRegistry::built_in()?;
    let providers = if let Some(provider) = args.provider.as_deref() {
        vec![registry.resolve(provider)?]
    } else {
        registry.list()
    };
    let db = open_db(ctx)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let spool = agent_hook_spool_pressure(db.db_dir());
    let mut reports = Vec::new();
    for manifest in providers {
        let installations = db.list_agent_hook_installations(Some(&manifest.provider))?;
        let receipts = db.list_agent_hook_receipts(Some(&manifest.provider), None, 1_000)?;
        let mappings = db.list_lane_agent_sessions(Some(&manifest.provider), None, 1_000)?;
        let runs = db
            .list_agent_capture_runs(false, 1_000)?
            .into_iter()
            .filter(|run| {
                run.owner_agent == manifest.provider
                    || run.executor_agent.as_deref() == Some(manifest.provider.as_str())
            })
            .collect::<Vec<_>>();
        let probe = registry.probe(&manifest.provider, args.probe)?;
        let failed_receipts = receipts
            .iter()
            .filter(|receipt| matches!(receipt.status.as_str(), "retry" | "quarantined"))
            .count();
        let stale_finalizers = mappings
            .iter()
            .filter(|mapping| {
                mapping.status == trail::AgentCapturePhase::Finalizing
                    && mapping
                        .finalization_lease_expires_at
                        .is_none_or(|expires_at| expires_at <= now)
            })
            .count();
        let home = agent_hooks_home_dir();
        let installation_checks = installations
            .iter()
            .map(|record| {
                build_agent_hook_install_plan(AgentHookInstallRequest {
                    registry: &registry,
                    provider: &manifest.provider,
                    workspace_id: &db.config().workspace.id.0,
                    workspace_root: db.workspace_root(),
                    home_dir: home.as_deref(),
                    scope: record.scope,
                    force: true,
                })
                .and_then(|plan| inspect_agent_hook_installation(&plan))
                .map(|status| {
                    serde_json::json!({
                        "installation_id": record.installation_id,
                        "status": status,
                    })
                })
                .unwrap_or_else(|error| {
                    serde_json::json!({
                        "installation_id": record.installation_id,
                        "status": "malformed",
                        "diagnostic": error.to_string(),
                    })
                })
            })
            .collect::<Vec<_>>();
        let checks = vec![
            serde_json::json!({
                "code": "HOOK_INSTALLATION_STATUS",
                "status": if installations.is_empty() { "warning" } else { "ok" },
                "message": if installations.is_empty() { "no recorded native hook installation" } else { "native hook ownership records found" },
                "remediation": format!("trail agent hooks add {}", manifest.provider),
                "details": installation_checks,
            }),
            serde_json::json!({
                "code": "RECEIPT_REPLAY_FAILURES",
                "status": if failed_receipts == 0 { "ok" } else { "warning" },
                "message": format!("{failed_receipts} receipt(s) require retry or operator review"),
                "remediation": "trail agent hooks replay --pending",
            }),
            serde_json::json!({
                "code": "TURN_FINALIZATION_LEASE_STALE",
                "status": if stale_finalizers == 0 { "ok" } else { "warning" },
                "message": format!("{stale_finalizers} stale finalization lease(s)"),
                "remediation": "trail agent hooks replay --pending",
            }),
            serde_json::json!({
                "code": "CAPTURE_RECEIPT_SPOOLING",
                "status": if spool.files == 0 { "ok" } else { "warning" },
                "message": format!("{} spooled receipt file(s), {} bytes", spool.files, spool.bytes),
                "remediation": "trail agent hooks replay --pending",
            }),
            serde_json::json!({
                "code": "TRANSCRIPT_CAPABILITY",
                "status": if manifest.transcript_location_hints.is_empty() && manifest.canonical_export_command.is_none() { "warning" } else { "ok" },
                "message": if manifest.transcript_location_hints.is_empty() && manifest.canonical_export_command.is_none() { "provider has no stable native transcript or canonical export contract" } else { "provider transcript/export collection is capability-gated" },
                "remediation": "inspect provider capability and fidelity diagnostics before relying on transcript evidence",
            }),
        ];
        reports.push(serde_json::json!({
            "provider": manifest.provider,
            "support": manifest.support,
            "adapter_version": manifest.adapter_version,
            "provider_version_range": manifest.provider_version_range,
            "contract_source": manifest.contract_source,
            "installations": installations,
            "last_receipt": receipts.first(),
            "receipt_status_counts": receipt_status_counts(&receipts),
            "capture_mappings": mappings,
            "managed_runs": runs,
            "spool": spool,
            "checks": checks,
            "probe": probe,
            "probe_requested": args.probe,
            "probe_status": if args.probe { "version-command" } else { "static-discovery" },
        }));
    }
    let summary = format!("checked {} agent hook provider(s)", reports.len());
    render_agent_hooks_value(ctx, serde_json::Value::Array(reports), summary)
}

fn handle_agent_hooks_events(ctx: &RuntimeContext, args: AgentHooksEventsArgs) -> Result<()> {
    let registry = AgentProviderRegistry::built_in()?;
    let provider = registry.resolve(&args.provider)?.provider.clone();
    let db = open_db(ctx)?;
    let mut receipts =
        db.list_agent_hook_receipts_page(Some(&provider), None, args.offset, args.last)?;
    if args.failed {
        receipts.retain(|receipt| {
            matches!(
                receipt.status.as_str(),
                "retry" | "quarantined" | "discarded"
            )
        });
    }
    let summary = format!("{}: {} durable receipt(s)", provider, receipts.len());
    render_agent_hooks_value(ctx, serde_json::to_value(receipts)?, summary)
}

fn handle_agent_hooks_replay(ctx: &RuntimeContext, args: AgentHooksReplayArgs) -> Result<()> {
    if args.receipt.is_none() && !args.pending {
        return Err(Error::InvalidInput(
            "agent hooks replay requires --receipt <id> or --pending".to_string(),
        ));
    }
    let mut db = open_db(ctx)?;
    let spool = if args.pending {
        drain_agent_hook_spool(&mut db)?
    } else {
        AgentHookSpoolDrain::default()
    };
    let batch = if let Some(receipt_id) = args.receipt {
        match db.replay_agent_hook_receipt(&receipt_id) {
            Ok(report) => trail::AgentHookReplayBatchReport {
                recovered_stale_receipts: 0,
                replayed: vec![report],
                failures: Vec::new(),
            },
            Err(error) => trail::AgentHookReplayBatchReport {
                recovered_stale_receipts: 0,
                replayed: Vec::new(),
                failures: vec![trail::AgentHookReplayFailure {
                    receipt_id,
                    code: error.code().to_string(),
                    message: error.to_string(),
                }],
            },
        }
    } else {
        db.replay_pending_agent_hook_receipts(args.limit)?
    };
    let value = serde_json::json!({
        "spool": spool,
        "recovered_stale_receipts": batch.recovered_stale_receipts,
        "replayed": batch.replayed,
        "failures": batch.failures
    });
    let summary = format!(
        "replayed {} receipt(s); {} failed",
        value["replayed"].as_array().map(Vec::len).unwrap_or(0),
        value["failures"].as_array().map(Vec::len).unwrap_or(0)
    );
    render_agent_hooks_value(ctx, value, summary)
}

#[derive(Default, serde::Serialize)]
struct AgentHookSpoolDrain {
    imported: usize,
    duplicates: usize,
    failed: Vec<serde_json::Value>,
}

#[derive(Clone, Copy, Default, serde::Serialize)]
struct AgentHookSpoolPressure {
    files: usize,
    bytes: u64,
}

fn agent_hook_spool_pressure(db_dir: &std::path::Path) -> AgentHookSpoolPressure {
    let directory = db_dir.join("runtime/agent-hooks-spool");
    if directory
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
        .unwrap_or(true)
    {
        return AgentHookSpoolPressure::default();
    }
    let mut pressure = AgentHookSpoolPressure::default();
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten().take(10_000) {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    pressure.files += 1;
                    pressure.bytes = pressure.bytes.saturating_add(metadata.len());
                }
            }
        }
    }
    pressure
}

fn receipt_status_counts(
    receipts: &[trail::AgentHookReceipt],
) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for receipt in receipts {
        *counts.entry(receipt.status.clone()).or_insert(0) += 1;
    }
    counts
}

fn drain_agent_hook_spool(db: &mut trail::Trail) -> Result<AgentHookSpoolDrain> {
    let directory = db.db_dir().join("runtime/agent-hooks-spool");
    if !directory.exists() {
        return Ok(AgentHookSpoolDrain::default());
    }
    if directory
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::InvalidPath {
            path: directory.display().to_string(),
            reason: "agent hook spool directory is a symlink".to_string(),
        });
    }
    let mut paths = std::fs::read_dir(&directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("receipt-") && name.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    let mut report = AgentHookSpoolDrain::default();
    for path in paths.into_iter().take(1_000) {
        let result = (|| -> Result<bool> {
            let metadata = path.symlink_metadata()?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > (trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES + 16_384) as u64
            {
                return Err(Error::InvalidPath {
                    path: path.display().to_string(),
                    reason: "invalid agent hook spool entry".to_string(),
                });
            }
            let envelope: AgentHookSpoolEnvelope = serde_json::from_slice(&std::fs::read(&path)?)?;
            if envelope.version != 1 {
                return Err(Error::InvalidInput(format!(
                    "unsupported agent hook spool version {}",
                    envelope.version
                )));
            }
            let ingested = db.persist_agent_hook_receipt(envelope.input)?;
            std::fs::remove_file(&path)?;
            Ok(ingested.duplicate)
        })();
        match result {
            Ok(true) => report.duplicates += 1,
            Ok(false) => report.imported += 1,
            Err(error) => report.failed.push(serde_json::json!({
                "path": path,
                "code": error.code(),
                "error": error.to_string(),
            })),
        }
    }
    Ok(report)
}

fn agent_hook_scope(scope: AgentHooksScopeArg) -> AgentHookInstallScope {
    match scope {
        AgentHooksScopeArg::Project => AgentHookInstallScope::Project,
        AgentHooksScopeArg::User => AgentHookInstallScope::User,
    }
}

fn agent_hooks_home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

fn render_agent_hooks_value(
    ctx: &RuntimeContext,
    value: serde_json::Value,
    human: String,
) -> Result<()> {
    if ctx.json {
        render_json(&value)?;
    } else {
        render_document(
            &TerminalDocument::new("Agent hook", UiTone::Neutral).block(UiBlock::paragraph(human)),
            &ctx.render,
        )?;
    }
    Ok(())
}

fn handle_agent_hook_receive(ctx: &RuntimeContext, args: AgentHookReceiveArgs) -> Result<()> {
    use std::io::Read;

    const MAX_STDIN_BYTES: u64 = (trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES + 1) as u64;
    let registry = AgentProviderRegistry::built_in();
    let provider = registry
        .as_ref()
        .ok()
        .and_then(|registry| registry.resolve(&args.provider).ok())
        .map(|manifest| manifest.provider.clone())
        .unwrap_or_else(|| args.provider.trim().to_ascii_lowercase());
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin()
        .lock()
        .take(MAX_STDIN_BYTES)
        .read_to_end(&mut bytes)
    {
        render_native_receipt_diagnostic(
            ctx,
            "Native agent receipt could not be read",
            error.to_string(),
        );
        return acknowledge_agent_hook(&provider, &args.native_event);
    }
    if bytes.len() > trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES {
        render_native_receipt_diagnostic(
            ctx,
            "Native agent receipt was not recorded",
            format!(
                "The receipt exceeds {} bytes.",
                trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES
            ),
        );
        return acknowledge_agent_hook(&provider, &args.native_event);
    }
    let payload: serde_json::Value =
        match if bytes.iter().all(u8::is_ascii_whitespace) && provider == "kiro" {
            Ok(serde_json::json!({}))
        } else {
            serde_json::from_slice(&bytes)
        } {
            Ok(payload) => payload,
            Err(error) => {
                render_native_receipt_diagnostic(
                    ctx,
                    "Native agent receipt was not recorded",
                    format!("The receipt is not valid JSON: {error}"),
                );
                return acknowledge_agent_hook(&provider, &args.native_event);
            }
        };
    let payload = enrich_kiro_hook_payload(payload, &provider, &args.native_event);
    if serde_json::to_vec(&payload)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
        > trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES
    {
        render_native_receipt_diagnostic(
            ctx,
            "Native agent receipt was not recorded",
            format!(
                "The enriched receipt exceeds {} bytes.",
                trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES
            ),
        );
        return acknowledge_agent_hook(&provider, &args.native_event);
    }
    if registry
        .as_ref()
        .ok()
        .and_then(|registry| registry.resolve(&provider).ok())
        .is_none()
    {
        render_native_receipt_diagnostic(
            ctx,
            "Native agent provider is not registered",
            format!("Provider `{provider}` is not registered."),
        );
        return acknowledge_agent_hook(&provider, &args.native_event);
    }
    let native_session_id = hook_payload_string(
        &payload,
        &[
            "session_id",
            "sessionId",
            "sessionID",
            "conversation_id",
            "conversationId",
            "thread_id",
            "threadId",
        ],
    );
    let native_turn_id = hook_payload_string(&payload, &["turn_id", "turnId"]);
    let occurred_at = hook_payload_i64(&payload, &["timestamp", "occurred_at", "occurredAt"]);
    let dedupe_key = args.dedupe_key.unwrap_or_else(|| {
        derive_hook_dedupe_key(
            &provider,
            &args.native_event,
            native_session_id.as_deref(),
            native_turn_id.as_deref(),
            &payload,
        )
    });
    let input = AgentHookReceiptInput {
        installation_id: args.installation,
        provider: provider.clone(),
        native_event: args.native_event.clone(),
        native_session_id,
        native_turn_id,
        transport: AgentCaptureTransport::NativeHooks,
        dedupe_key,
        payload: payload.clone(),
        occurred_at,
    };
    let capture_result = open_db(ctx).and_then(|mut db| {
        let ingested = db.persist_agent_hook_receipt(input.clone())?;
        if ingested.receipt.status != "processed" {
            if let Err(error) = db.replay_agent_hook_receipt(&ingested.receipt.receipt_id) {
                render_native_receipt_diagnostic(
                    ctx,
                    "Native receipt replay is deferred",
                    error.to_string(),
                );
            }
        }
        Ok(())
    });
    if let Err(error) = capture_result {
        let envelope = AgentHookSpoolEnvelope {
            version: 1,
            input: AgentHookReceiptInput {
                payload: redact_agent_hook_payload(payload),
                ..input
            },
        };
        if let Err(spool_error) = spool_agent_hook_receipt(ctx, &envelope) {
            render_native_receipt_diagnostic(
                ctx,
                "Native receipt was not recorded",
                format!("{error}; fallback spool also failed: {spool_error}"),
            );
        } else {
            render_native_receipt_diagnostic(
                ctx,
                "Native receipt was spooled for later replay",
                error.to_string(),
            );
        }
    }
    acknowledge_agent_hook(&provider, &args.native_event)
}

fn enrich_kiro_hook_payload(
    mut payload: serde_json::Value,
    provider: &str,
    native_event: &str,
) -> serde_json::Value {
    if provider != "kiro" {
        return payload;
    }
    let Some(root) = payload.as_object_mut() else {
        return payload;
    };
    if native_event.eq_ignore_ascii_case("UserPromptSubmit") && !root.contains_key("prompt") {
        if let Ok(prompt) = std::env::var("USER_PROMPT") {
            if !prompt.is_empty() && prompt.len() <= trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES {
                root.insert("prompt".to_string(), serde_json::Value::String(prompt));
            }
        }
    }
    if !root.contains_key("cwd") {
        if let Ok(cwd) = std::env::current_dir() {
            root.insert(
                "cwd".to_string(),
                serde_json::Value::String(cwd.to_string_lossy().into_owned()),
            );
        }
    }
    payload
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct AgentHookSpoolEnvelope {
    version: u16,
    input: AgentHookReceiptInput,
}

fn acknowledge_agent_hook(provider: &str, native_event: &str) -> Result<()> {
    if provider == "codex" && matches!(native_event, "Stop" | "SubagentStop") {
        render_protocol_content("{\"continue\":true}\n")?;
    } else if provider == "gemini" {
        render_protocol_content("{}\n")?;
    }
    Ok(())
}

fn render_native_receipt_diagnostic(ctx: &RuntimeContext, summary: &str, cause: String) {
    let diagnostic = UiDiagnostic {
        code: "AGENT_HOOK_RECEIPT".to_string(),
        summary: summary.to_string(),
        cause: Some(cause),
        consequence: Some(
            "Trail acknowledged the native hook but did not apply this receipt.".to_string(),
        ),
        recovery: Some(UiNextAction {
            command: "trail agent hooks replay --pending".to_string(),
            reason: "Retry recorded native hook receipts after fixing the provider issue."
                .to_string(),
        }),
        alternatives: Vec::new(),
    };
    let _ = render_error_document(
        &TerminalDocument::empty().block(UiBlock::Diagnostic(diagnostic)),
        &ctx.render,
    );
}

fn spool_agent_hook_receipt(ctx: &RuntimeContext, envelope: &AgentHookSpoolEnvelope) -> Result<()> {
    use std::io::Write;

    let db_dir = agent_hook_spool_db_dir(ctx)
        .ok_or_else(|| Error::WorkspaceNotFound(std::env::current_dir().unwrap_or_default()))?;
    if db_dir
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(Error::InvalidPath {
            path: db_dir.display().to_string(),
            reason: "agent hook spool database directory is a symlink".to_string(),
        });
    }
    let spool_dir = db_dir.join("runtime/agent-hooks-spool");
    std::fs::create_dir_all(&spool_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&spool_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut spool_files = 0usize;
    let mut spool_bytes = 0u64;
    for entry in std::fs::read_dir(&spool_dir)? {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            spool_files = spool_files.saturating_add(1);
            spool_bytes = spool_bytes.saturating_add(metadata.len());
        }
    }
    if spool_files >= 10_000 || spool_bytes >= 64 * 1024 * 1024 {
        return Err(Error::InvalidInput(format!(
            "agent hook spool quota exceeded ({spool_files} files, {spool_bytes} bytes)"
        )));
    }
    let bytes = serde_json::to_vec(envelope)?;
    if bytes.len() > trail::AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES + 16_384 {
        return Err(Error::InvalidInput(
            "redacted agent hook spool envelope is too large".to_string(),
        ));
    }
    let digest = hook_short_hash(&bytes, 24);
    let target = spool_dir.join(format!("receipt-{digest}.json"));
    if target.exists() {
        return Ok(());
    }
    let temp = spool_dir.join(format!(".receipt-{digest}-{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    if let Err(error) = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)?;
        #[cfg(unix)]
        std::fs::File::open(&spool_dir)?.sync_all()?;
        Ok(())
    })() {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

fn agent_hook_spool_db_dir(ctx: &RuntimeContext) -> Option<std::path::PathBuf> {
    if let Some(db_dir) = ctx.db_dir.as_ref() {
        return Some(db_dir.clone());
    }
    if let Some(workspace) = ctx.workspace.as_ref() {
        return Some(workspace.join(".trail"));
    }
    let mut cursor = std::env::current_dir().ok()?;
    loop {
        let candidate = cursor.join(".trail");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !cursor.pop() {
            return None;
        }
    }
}

fn hook_payload_string(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        hook_payload_value(payload, key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn hook_payload_i64(payload: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        hook_payload_value(payload, key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        })
    })
}

fn hook_payload_value<'a>(
    payload: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    payload.get(key).or_else(|| {
        ["properties", "input", "event", "data", "session"]
            .iter()
            .find_map(|container| payload.get(*container).and_then(|value| value.get(key)))
    })
}

fn derive_hook_dedupe_key(
    provider: &str,
    native_event: &str,
    native_session_id: Option<&str>,
    native_turn_id: Option<&str>,
    payload: &serde_json::Value,
) -> String {
    let explicit_event_id = hook_payload_string(
        payload,
        &[
            "event_id",
            "eventId",
            "tool_use_id",
            "toolUseId",
            "toolCallId",
            "agent_id",
            "agentId",
        ],
    );
    let timestamp = hook_payload_i64(payload, &["timestamp", "occurred_at", "occurredAt"]);
    let payload_digest = hook_short_hash(
        serde_json::to_vec(payload).unwrap_or_default().as_slice(),
        24,
    );
    let entropy = if explicit_event_id.is_none() && native_turn_id.is_none() && timestamp.is_none()
    {
        hook_short_hash(
            format!(
                "{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|value| value.as_nanos())
                    .unwrap_or_default()
            )
            .as_bytes(),
            16,
        )
    } else {
        String::new()
    };
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        provider,
        native_event,
        native_session_id.unwrap_or("none"),
        native_turn_id.unwrap_or("none"),
        explicit_event_id.as_deref().unwrap_or("none"),
        timestamp
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        if entropy.is_empty() {
            payload_digest
        } else {
            format!("{payload_digest}:{entropy}")
        }
    )
}

fn hook_short_hash(bytes: &[u8], digest_bytes: usize) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .take(digest_bytes.min(digest.len()))
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn handle_agent_home(ctx: &RuntimeContext) -> Result<()> {
    let mut db = open_db(ctx)?;
    let tasks = db.list_agent_tasks()?.tasks;
    if tasks.len() == 1 {
        let report = db.agent_dashboard(&tasks[0].lane)?;
        render_agent_dashboard(&report, ctx.json, &ctx.render)
    } else {
        let report = db.agent_inbox()?;
        render_agent_inbox(&report, ctx.json, &ctx.render)
    }
}

fn handle_agent_setup(ctx: &RuntimeContext, args: AgentSetupArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_setup_report(&args.provider, &args.editor)?;
    render_agent_setup(&report, ctx.json, &ctx.render)
}

fn handle_agent_acp(ctx: &RuntimeContext, args: AgentAcpArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let lane = db.fresh_agent_lane_name(&args.provider, args.name.as_deref());
    let launch = if args.command.is_empty() {
        Some(trail::acp::resolve_acp_provider(
            &args.provider,
            Some(db.db_dir()),
        )?)
    } else {
        None
    };
    let provider = launch
        .as_ref()
        .map(|launch| launch.profile.agent.clone())
        .unwrap_or_else(|| args.provider.clone());
    let upstream_command = launch
        .as_ref()
        .map(|launch| launch.upstream_command.clone())
        .unwrap_or(args.command);
    let upstream_env = launch.map(|launch| launch.upstream_env).unwrap_or_default();
    trail::acp::run_stdio_relay(AcpRelayOptions {
        workspace_root: db.workspace_root().to_path_buf(),
        db_dir: db.db_dir().to_path_buf(),
        lane: Some(lane),
        from_ref: args.from,
        provider: Some(provider),
        model: None,
        materialize: true,
        workdir: None,
        inject_mcp: !args.no_mcp,
        upstream_command,
        upstream_env,
    })
}

fn handle_agent_start(ctx: &RuntimeContext, args: AgentStartArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let lane = db.fresh_agent_lane_name(&args.provider, args.name.as_deref());
    let workdir_mode = db.resolve_lane_spawn_workdir_mode(
        args.from.as_deref(),
        Some(&args.workdir_mode),
        Some(true),
        false,
        false,
        &[],
    )?;
    let report = run_terminal_agent_task(
        ctx,
        db,
        lane,
        args.provider,
        args.from,
        workdir_mode,
        args.command,
    )?;
    render_agent_run(&report, ctx.json, &ctx.render)
}

fn handle_agent_continue(ctx: &RuntimeContext, args: AgentContinueArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let source = db.agent_task_view(&args.selector)?;
    let from_change = source
        .task
        .latest_checkpoint
        .clone()
        .unwrap_or_else(|| source.review.lane.branch.head_change.clone());
    let provider = args
        .provider
        .or_else(|| source.task.provider.clone())
        .unwrap_or_else(|| "claude-code".to_string());
    let name = args
        .name
        .unwrap_or_else(|| format!("{} follow-up", source.task.title));
    let lane = db.fresh_agent_lane_name(&provider, Some(&name));
    let source_task = source.task.clone();
    let workdir_mode = db.resolve_lane_spawn_workdir_mode(
        Some(&from_change.0),
        Some(&args.workdir_mode),
        Some(true),
        false,
        false,
        &[],
    )?;
    let run = run_terminal_agent_task(
        ctx,
        db,
        lane,
        provider,
        Some(from_change.0.clone()),
        workdir_mode,
        args.command,
    )?;
    let mut suggestions = vec![StatusSuggestion {
        command: format!("trail agent view {}", run.task.lane),
        reason: "inspect the new follow-up task transcript and checkpoint".to_string(),
    }];
    push_agent_cli_suggestion(
        &mut suggestions,
        format!("trail agent changes {}", run.task.lane),
        "review changes made in the follow-up task",
    );
    push_agent_cli_suggestion(
        &mut suggestions,
        format!("trail agent land {} --dry-run", run.task.lane),
        "preview applying the follow-up task safely",
    );
    let report = AgentContinueReport {
        source_task,
        from_change,
        run,
        suggestions,
    };
    render_agent_continue(&report, ctx.json, &ctx.render)
}

fn run_terminal_agent_task(
    ctx: &RuntimeContext,
    mut db: trail::Trail,
    lane: String,
    provider: String,
    from: Option<String>,
    workdir_mode: LaneWorkdirMode,
    command: Vec<String>,
) -> Result<AgentRunReport> {
    const TERMINAL_CAPTURE_LEASE_MS: u64 = 86_400_000;

    let spawn = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        &lane,
        from.as_deref(),
        workdir_mode.clone(),
        Some(provider.clone()),
        None,
        None,
        &[],
        false,
    )?;
    let workdir = spawn.workdir.clone().ok_or_else(|| {
        Error::InvalidInput("agent start requires a filesystem lane workdir".to_string())
    })?;
    let fuse_mount = if workdir_mode == LaneWorkdirMode::FuseCow {
        Some(db.mount_fuse_cow_workdir_for_lane(&lane)?)
    } else {
        None
    };
    let nfs_mount = if workdir_mode == LaneWorkdirMode::NfsCow {
        Some(db.mount_nfs_cow_workdir_for_lane(&lane)?)
    } else {
        None
    };
    #[cfg(target_os = "windows")]
    let dokan_mount = if workdir_mode == LaneWorkdirMode::DokanCow {
        Some(db.mount_dokan_cow_workdir_for_lane(&lane)?)
    } else {
        None
    };
    let session = db
        .start_lane_session(&lane, Some(format!("Agent terminal {}", provider)), None)?
        .session;
    let capture_run = db.begin_agent_capture_run(trail::AgentCaptureRunInput {
        lane: Some(lane.clone()),
        workdir: workdir.clone(),
        owner_agent: provider.clone(),
        owner_session_id: session.session_id.clone(),
        executor_agent: Some(provider.clone()),
        work_item_id: Some(lane.clone()),
        lease_ms: TERMINAL_CAPTURE_LEASE_MS,
        metadata_json: Some("{\"source\":\"agent-start\",\"mode\":\"terminal\"}".to_string()),
    })?;
    db.add_lane_session_event(
        &lane,
        &session.session_id,
        "agent_task_started",
        Some(serde_json::json!({
            "provider": provider,
            "workdir": workdir,
            "mode": "terminal",
            "workdir_mode": workdir_mode.as_str(),
            "from": from
        })),
    )?;
    let mut workspace_environment = db.lane_workspace_environment(&lane)?;
    workspace_environment.extend([
        ("TRAIL_CAPTURE_MODE".to_string(), "terminal".to_string()),
        (
            "TRAIL_CAPTURE_RUN_ID".to_string(),
            capture_run.capture_run_id.clone(),
        ),
        (
            "TRAIL_CAPTURE_WORKSPACE".to_string(),
            db.workspace_root().to_string_lossy().into_owned(),
        ),
        (
            "TRAIL_CAPTURE_LANE".to_string(),
            capture_run.lane_id.clone().unwrap_or_else(|| lane.clone()),
        ),
    ]);
    let project_hook_settings = if provider == "claude-code" {
        db.list_agent_hook_installations(Some(&provider))?
            .into_iter()
            .find(|installation| {
                installation.status == "installed"
                    && installation.scope == AgentHookInstallScope::Project
                    && installation.config_path.is_file()
            })
            .map(|installation| installation.config_path)
    } else {
        None
    };
    let workspace_root = db.workspace_root().to_path_buf();
    drop(db);

    let command_is_default = command.is_empty();
    let mut command = if command_is_default {
        default_terminal_agent_command(&provider)?
    } else {
        command
    };
    if command_is_default {
        if let Some(settings) = project_hook_settings {
            command.push("--settings".to_string());
            command.push(settings.to_string_lossy().into_owned());
        }
    }
    if !ctx.json {
        render_document(
            &TerminalDocument::new(format!("Launching agent task {lane}"), UiTone::Success).block(
                UiBlock::Metadata(vec![
                    ("Workdir".to_string(), workdir.clone()),
                    ("Command".to_string(), command.join(" ")),
                ]),
            ),
            &ctx.render,
        )?;
    }
    let (launch_program, launch_args) =
        confined_terminal_agent_command(&command, &workspace_root, Path::new(&workdir))?;
    let mut process = ProcessCommand::new(launch_program);
    let mut git_ceiling_directories = std::env::var_os("GIT_CEILING_DIRECTORIES")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    if !git_ceiling_directories
        .iter()
        .any(|path| path == &workspace_root)
    {
        git_ceiling_directories.push(workspace_root.clone());
    }
    let git_ceiling_directories =
        std::env::join_paths(git_ceiling_directories).map_err(|error| {
            Error::InvalidInput(format!("cannot construct Git discovery ceiling: {error}"))
        })?;
    process
        .args(launch_args)
        .current_dir(&workdir)
        .envs(workspace_environment)
        .env("PWD", &workdir)
        .env("OLDPWD", &workdir)
        .env("GIT_CEILING_DIRECTORIES", git_ceiling_directories)
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
            "provider": provider,
            "exit_code": exit_code,
            "status": status
        })),
    )?;
    if db.show_lane_session(&session.session_id)?.session.status == "active" {
        db.end_lane_session(&session.session_id, status)?;
    }
    db.end_agent_capture_run(&capture_run.capture_run_id, &provider, &session.session_id)?;
    let view = db.agent_task_view(&lane)?;
    let report = AgentRunReport {
        task: view.task,
        provider,
        command,
        workdir: Some(workdir),
        exit_code,
        recorded: Some(recorded),
        status: status.to_string(),
    };
    drop(fuse_mount);
    drop(nfs_mount);
    #[cfg(target_os = "windows")]
    drop(dokan_mount);
    Ok(report)
}

fn push_agent_cli_suggestion(
    suggestions: &mut Vec<StatusSuggestion>,
    command: String,
    reason: &str,
) {
    if !suggestions
        .iter()
        .any(|suggestion| suggestion.command == command)
    {
        suggestions.push(StatusSuggestion {
            command,
            reason: reason.to_string(),
        });
    }
}

fn handle_agent_guide(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_guide(&args.selector)?;
    render_agent_guide(&report, ctx.json, &ctx.render)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AgentAskIntent {
    Inbox,
    Board,
    Stack,
    Next,
    Guide,
    Dashboard,
    ReviewData,
    Actions,
    Summary,
    Validate,
    TestPlan,
    Brief,
    Story,
    Risk,
    Impact,
    ReviewMap,
    Confidence,
    Ready,
    Diagnose,
    Receipt,
    Handoff,
    Pr,
    Changes,
    ChangesByFile,
    Tools,
    TaskDiff { file: Option<String>, patch: bool },
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
    ReviewFlow,
    Review,
    Focus,
    View,
}

fn handle_agent_ask(ctx: &RuntimeContext, args: AgentAskArgs) -> Result<()> {
    let selector = args.selector;
    let question = args.question.join(" ");
    match resolve_agent_ask_intent(&question)? {
        AgentAskIntent::Inbox => handle_agent_inbox(ctx, AgentInboxArgs { all: false }),
        AgentAskIntent::Board => handle_agent_board(ctx, AgentInboxArgs { all: false }),
        AgentAskIntent::Stack => handle_agent_stack(ctx, AgentInboxArgs { all: false }),
        AgentAskIntent::Next => handle_agent_next(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Guide => handle_agent_guide(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Dashboard => handle_agent_dashboard(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::ReviewData => handle_agent_review_data(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Actions => handle_agent_action(
            ctx,
            AgentActionArgs {
                selector_or_action: Some(selector),
                action: None,
                print: false,
                confirm: false,
                message: None,
                note: None,
            },
        ),
        AgentAskIntent::Summary => handle_agent_summary(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Validate => handle_agent_validate(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::TestPlan => handle_agent_test_plan(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Brief => handle_agent_brief(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Story => handle_agent_story(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Risk => handle_agent_risk(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Impact => handle_agent_impact(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::ReviewMap => handle_agent_review_map(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Confidence => handle_agent_confidence(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Ready => handle_agent_ready(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Diagnose => handle_agent_diagnose(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Receipt => handle_agent_receipt(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::Handoff => handle_agent_handoff(ctx, AgentSelectorArgs { selector }),
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
                by_file: false,
            },
        ),
        AgentAskIntent::ChangesByFile => handle_agent_changes(
            ctx,
            AgentChangesArgs {
                selector,
                by_turn: false,
                by_operation: false,
                by_file: true,
            },
        ),
        AgentAskIntent::Tools => handle_agent_tools(ctx, AgentSelectorArgs { selector }),
        AgentAskIntent::TaskDiff { file, patch } => handle_agent_diff(
            ctx,
            AgentDiffArgs {
                selector,
                turn: None,
                operation: None,
                checkpoint: None,
                last_turn: false,
                file,
                stat: false,
                patch,
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
            AgentTimelineArgs {
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
        AgentAskIntent::ReviewFlow => handle_agent_review_flow(ctx, AgentSelectorArgs { selector }),
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
    let mentions_apply_flow = lowered.contains("pull request")
        || lowered.contains("apply")
        || lowered.contains("merge")
        || lowered.contains("ready")
        || agent_ask_has_any(&lowered_tokens, &["land", "ship", "pr"]);
    if lowered.contains("review data")
        || lowered.contains("review json")
        || lowered.contains("review packet json")
        || lowered.contains("editor panel")
        || lowered.contains("side panel")
        || lowered.contains("panel data")
        || lowered.contains("ui data")
        || lowered.contains("one json")
        || lowered.contains("single json")
        || lowered.contains("single packet")
    {
        return Ok(AgentAskIntent::ReviewData);
    }
    let asks_actions = lowered.contains("action palette")
        || lowered.contains("actions palette")
        || lowered.contains("command palette")
        || lowered.contains("show actions")
        || lowered.contains("show action")
        || lowered.contains("list actions")
        || lowered.contains("available actions")
        || lowered.contains("what actions")
        || lowered.contains("which actions")
        || lowered.contains("buttons")
        || lowered.contains("show buttons")
        || lowered.contains("what buttons")
        || lowered.contains("what can i do")
        || lowered.contains("what can we do")
        || lowered.contains("what are my options")
        || lowered.contains("what options")
        || lowered.contains("available commands")
        || lowered.contains("what commands can i run")
        || lowered.contains("which commands can i run");
    if path.is_none() && asks_actions {
        return Ok(AgentAskIntent::Actions);
    }
    let asks_blocker = lowered.contains("what blocks")
        || lowered.contains("what is blocking")
        || lowered.contains("what's blocking")
        || lowered.contains("what is blocked")
        || lowered.contains("why blocked")
        || lowered.contains("why is this blocked")
        || lowered.contains("why is it blocked")
        || lowered.contains("blocking this")
        || lowered.contains("blocking the")
        || agent_ask_has_any(&lowered_tokens, &["blockers", "blocking"]);
    let asks_problem = lowered.contains("what went wrong")
        || lowered.contains("what's wrong")
        || lowered.contains("what is wrong")
        || lowered.contains("anything wrong")
        || lowered.contains("any problems")
        || lowered.contains("any issues")
        || lowered.contains("did it fail")
        || lowered.contains("did this fail")
        || lowered.contains("why did it fail")
        || lowered.contains("why did this fail")
        || lowered.contains("why failed")
        || lowered.contains("why is it failing")
        || lowered.contains("why is this failing")
        || agent_ask_has_any(
            &lowered_tokens,
            &["failed", "failing", "failure", "problem", "problems"],
        );
    let asks_file_risk = (lowered.contains("risk")
        || lowered.contains("risky")
        || lowered.contains("red flag")
        || lowered.contains("worry")
        || lowered.contains("danger"))
        && (lowered.contains("file")
            || lowered.contains("files")
            || lowered.contains("path")
            || lowered.contains("paths"));
    let asks_file_to_open = lowered.contains("what file should i open")
        || lowered.contains("which file should i open")
        || lowered.contains("what file do i open")
        || lowered.contains("which file do i open")
        || lowered.contains("file should i open")
        || lowered.contains("open first file")
        || lowered.contains("open the first file")
        || lowered.contains("open next file")
        || lowered.contains("open the next file")
        || lowered.contains("open in editor")
        || lowered.contains("open in my editor");
    let asks_impact = lowered.contains("impact")
        || lowered.contains("blast radius")
        || lowered.contains("surface area")
        || lowered.contains("scope of change")
        || lowered.contains("change scope")
        || lowered.contains("what areas")
        || lowered.contains("which areas")
        || lowered.contains("areas did")
        || lowered.contains("areas changed")
        || lowered.contains("what parts")
        || lowered.contains("which parts")
        || lowered.contains("what surfaces")
        || lowered.contains("which surfaces")
        || lowered.contains("what should i test because")
        || lowered.contains("what should we test because");
    let asks_review_map = lowered.contains("review map")
        || lowered.contains("review-map")
        || lowered.contains("file checklist")
        || lowered.contains("files checklist")
        || lowered.contains("review files")
        || lowered.contains("review all files")
        || lowered.contains("review by file")
        || lowered.contains("review by area")
        || lowered.contains("map of changes")
        || lowered.contains("map the changes")
        || lowered.contains("change map")
        || lowered.contains("changes map")
        || lowered.contains("review every file");
    let asks_confidence = lowered.contains("confidence")
        || lowered.contains("go/no-go")
        || lowered.contains("go no go")
        || lowered.contains("go-no-go")
        || lowered.contains("final check")
        || lowered.contains("ship check")
        || lowered.contains("apply check")
        || lowered.contains("green light")
        || lowered.contains("am i good")
        || lowered.contains("are we good")
        || lowered.contains("good to go")
        || lowered.contains("should i ship")
        || lowered.contains("should we ship")
        || lowered.contains("should i land")
        || lowered.contains("should we land")
        || lowered.contains("should i apply")
        || lowered.contains("should we apply");
    let asks_test_plan = lowered.contains("test plan")
        || lowered.contains("validation plan")
        || lowered.contains("what tests")
        || lowered.contains("which tests")
        || lowered.contains("what should i test")
        || lowered.contains("what should we test")
        || lowered.contains("how do i test")
        || lowered.contains("how should i test")
        || lowered.contains("how should we test")
        || lowered.contains("how to test")
        || lowered.contains("test this")
        || lowered.contains("test it")
        || lowered.contains("run tests")
        || lowered.contains("run the tests")
        || lowered.contains("what validation should i run")
        || lowered.contains("which validation should i run");

    if path.is_none() && asks_confidence {
        return Ok(AgentAskIntent::Confidence);
    }
    if path.is_none() && asks_impact {
        return Ok(AgentAskIntent::Impact);
    }
    if path.is_none() && asks_review_map {
        return Ok(AgentAskIntent::ReviewMap);
    }
    if path.is_none() && asks_test_plan {
        return Ok(AgentAskIntent::TestPlan);
    }

    if path.is_none()
        && mentions_apply_flow
        && (agent_ask_has_any(&lowered_tokens, &["why", "reason"])
            || lowered.contains("why can't")
            || lowered.contains("why cant")
            || lowered.contains("why cannot")
            || asks_blocker)
    {
        return Ok(AgentAskIntent::Ready);
    }
    if path.is_none() && asks_blocker {
        return Ok(AgentAskIntent::Diagnose);
    }
    if path.is_none() && asks_problem {
        return Ok(AgentAskIntent::Diagnose);
    }
    if agent_ask_has_any(&lowered_tokens, &["why", "explain", "reason"]) {
        return path.map(AgentAskIntent::Why).ok_or_else(|| {
            Error::InvalidInput(
                "agent ask needs a file path for why/explain questions, for example `trail agent ask explain README.md`"
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
    if asks_file_risk {
        return Ok(AgentAskIntent::ChangesByFile);
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
        || lowered.contains("what did it change")
        || lowered.contains("what did the agent change")
        || lowered.contains("what did it touch")
        || lowered.contains("what did the agent touch")
        || lowered.contains("what files did it change")
        || lowered.contains("what files did the agent change")
        || lowered.contains("which files did it change")
        || lowered.contains("which files did the agent change")
        || lowered.contains("what files did it touch")
        || lowered.contains("what files did the agent touch")
        || lowered.contains("which files did it touch")
        || lowered.contains("which files did the agent touch")
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
    if lowered.contains("apply order")
        || lowered.contains("apply first")
        || lowered.contains("which agent first")
        || lowered.contains("which task first")
        || lowered.contains("what should i apply first")
        || lowered.contains("what should i land first")
        || lowered.contains("what should i finish first")
        || lowered.contains("show stack")
        || lowered.contains("agent stack")
        || lowered.contains("task stack")
        || matches!(lowered.trim(), "stack" | "order" | "apply order")
    {
        return Ok(AgentAskIntent::Stack);
    }
    if lowered.contains("agent board")
        || lowered.contains("task board")
        || lowered.contains("multi agent")
        || lowered.contains("multi-agent")
        || lowered.contains("show board")
        || matches!(lowered.trim(), "board" | "tasks")
    {
        return Ok(AgentAskIntent::Board);
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
    if lowered.contains("walk me through")
        || lowered.contains("walk through")
        || lowered.contains("walkthrough")
        || lowered.contains("step by step")
        || lowered.contains("step-by-step")
        || lowered.contains("review flow")
        || lowered.contains("review loop")
        || lowered.contains("review checklist")
        || lowered.contains("finish checklist")
        || lowered.contains("ship checklist")
        || lowered.contains("guide me through review")
        || lowered.contains("guide me through the review")
        || lowered.contains("how do i review")
        || lowered.contains("how should i review")
        || lowered.contains("review steps")
        || lowered.contains("review workflow")
    {
        return Ok(AgentAskIntent::ReviewFlow);
    }
    if lowered.contains("review first")
        || lowered.contains("inspect first")
        || asks_file_to_open
        || lowered.contains("what file should i review first")
        || lowered.contains("which file should i review first")
        || lowered.contains("first file to review")
        || lowered.contains("first file should i review")
        || lowered.contains("look at first")
        || lowered.contains("look first")
        || lowered.contains("where should i look first")
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
    if lowered.contains("commit message")
        || lowered.contains("git message")
        || lowered.contains("message for commit")
        || lowered.contains("message should i use")
        || lowered.contains("what message should i use")
        || lowered.contains("commit title")
    {
        return Ok(AgentAskIntent::Ready);
    }
    if lowered.contains("pr title")
        || lowered.contains("pr body")
        || lowered.contains("pr description")
        || lowered.contains("pull request title")
        || lowered.contains("pull request body")
        || lowered.contains("pull request description")
        || lowered.contains("draft pr")
        || lowered.contains("draft a pr")
        || lowered.contains("draft the pr")
        || lowered.contains("draft pull request")
        || lowered.contains("draft a pull request")
        || lowered.contains("draft the pull request")
        || lowered.contains("put in the pr")
        || lowered.contains("put in pr")
        || lowered.contains("put in the pull request")
        || lowered.contains("put in pull request")
        || lowered.contains("write the pr")
        || lowered.contains("write a pr")
        || lowered.contains("write the pull request")
        || lowered.contains("write a pull request")
    {
        return Ok(AgentAskIntent::Pr);
    }
    if lowered.contains("handoff")
        || lowered.contains("hand off")
        || lowered.contains("share with another agent")
        || lowered.contains("share with an agent")
        || lowered.contains("give to another agent")
        || lowered.contains("give this to another agent")
        || lowered.contains("send to another agent")
        || lowered.contains("handoff packet")
    {
        return Ok(AgentAskIntent::Handoff);
    }
    if lowered.contains("receipt")
        || lowered.contains("copyable")
        || lowered.contains("share summary")
        || lowered.contains("summary to share")
        || lowered.contains("what should i share")
        || lowered.contains("note to share")
        || lowered.contains("review note")
        || lowered.contains("after action")
        || lowered.contains("after-action")
        || lowered.contains("post run")
        || lowered.contains("post-run")
    {
        return Ok(AgentAskIntent::Receipt);
    }
    if lowered.contains("red flag")
        || lowered.contains("what should i worry")
        || lowered.contains("what should we worry")
        || lowered.contains("worry about")
        || lowered.contains("worried about")
        || lowered.contains("anything risky")
        || lowered.contains("what is risky")
        || lowered.contains("what's risky")
        || lowered.contains("risky")
        || lowered.contains("dangerous")
        || lowered.contains("danger")
        || lowered.contains("unsafe")
        || lowered.contains("blast radius")
        || lowered.contains("high risk")
        || lowered.contains("risk review")
    {
        return Ok(AgentAskIntent::Risk);
    }
    if lowered.contains("help me")
        || lowered.contains("show guide")
        || lowered.contains("agent guide")
        || lowered.contains("getting started")
        || lowered.contains("how do i use trail")
        || lowered.contains("how should i use trail")
        || lowered.contains("how to use trail")
        || lowered.contains("how do i use this")
        || lowered.contains("how should i use this")
        || lowered.contains("what commands should i use")
        || matches!(lowered.trim(), "help" | "guide")
    {
        return Ok(AgentAskIntent::Guide);
    }
    if lowered.contains("what should")
        || lowered.contains("what now")
        || lowered.contains("next")
        || agent_ask_has_any(&lowered_tokens, &["todo"])
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
        if lowered.contains("by file")
            || lowered.contains("by changed file")
            || lowered.contains("per file")
            || lowered.contains("file by file")
            || lowered.contains("file-by-file")
        {
            return Ok(AgentAskIntent::ChangesByFile);
        }
        return Ok(AgentAskIntent::Changes);
    }
    if wants_patch {
        return Ok(AgentAskIntent::TaskDiff {
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
    if lowered.contains("tool call")
        || lowered.contains("tool use")
        || lowered.contains("tools used")
        || lowered.contains("used tools")
        || lowered.contains("available command")
        || lowered.contains("available commands")
        || agent_ask_has_any(&lowered_tokens, &["tools", "tool"])
    {
        return Ok(AgentAskIntent::Tools);
    }
    if agent_ask_has_any(&lowered_tokens, &["commands", "command"]) {
        return Ok(AgentAskIntent::Tools);
    }
    if lowered.contains("test status")
        || lowered.contains("validation")
        || lowered.contains("tests passing")
        || lowered.contains("tests pass")
        || lowered.contains("test pass")
        || lowered.contains("did tests pass")
        || lowered.contains("did the tests pass")
        || lowered.contains("did it pass tests")
        || lowered.contains("is it tested")
        || lowered.contains("is this tested")
        || lowered.contains("has it been tested")
        || lowered.contains("was it tested")
        || lowered.contains("test results")
        || lowered.contains("validation status")
        || lowered.contains("validation guidance")
        || lowered.contains("missing validation")
        || lowered.contains("validation missing")
        || lowered.contains("what validation")
        || lowered.contains("which validation")
        || lowered.contains("need validation")
        || lowered.contains("needs validation")
        || lowered.contains("do i need tests")
        || lowered.contains("need tests")
    {
        return Ok(AgentAskIntent::Validate);
    }
    if lowered.contains("dashboard")
        || lowered.contains("overview")
        || lowered.contains("cockpit")
        || lowered.contains("status board")
        || lowered.contains("one screen")
    {
        return Ok(AgentAskIntent::Dashboard);
    }
    if lowered.contains("summary") {
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
    if lowered.contains("handoff") || lowered.contains("hand off") {
        return Ok(AgentAskIntent::Handoff);
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
    render_agent_status(&report, ctx.json, &ctx.render)
}

fn handle_agent_dashboard(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_dashboard(&args.selector)?;
    render_agent_dashboard(&report, ctx.json, &ctx.render)
}

fn handle_agent_review_data(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_review_data(&args.selector)?;
    render_agent_review_data(&report, ctx.json, &ctx.render)
}

fn handle_agent_action(ctx: &RuntimeContext, args: AgentActionArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let (selector, action_id) = agent_action_selector_args(&mut db, &args)?;
    let review_data = match db.agent_review_data(&selector) {
        Ok(report) => report,
        Err(Error::InvalidInput(message)) if message.contains("no agent tasks") => {
            if let Some(action_id) = action_id {
                return handle_agent_empty_action(ctx, &action_id, &args);
            }
            return render_agent_empty_action_palette(ctx.json, &ctx.render);
        }
        Err(err) => return Err(err),
    };
    let Some(action_id) = action_id else {
        return render_agent_action_palette(&review_data, ctx.json, &ctx.render);
    };
    let action = review_data
        .actions
        .iter()
        .find(|action| agent_action_matches(action, &action_id))
        .cloned()
        .ok_or_else(|| {
            let known = review_data
                .actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Error::InvalidInput(format!(
                "agent action `{action_id}` was not found for `{selector}`; available actions: {known}"
            ))
        })?;

    if args.print {
        if ctx.json {
            return render_json(&serde_json::json!({
                "task": review_data.task,
                "action": action
            }));
        }
        render_raw_content(&format!("{}\n", action.command), &ctx.render)?;
        return Ok(());
    }
    if !action.enabled {
        return Err(Error::InvalidInput(format!(
            "agent action `{}` is disabled: {}",
            action.id,
            action
                .disabled_reason
                .as_deref()
                .unwrap_or("the current task state does not allow it")
        )));
    }
    if action.requires_confirmation && !args.confirm {
        return Err(Error::InvalidInput(format!(
            "agent action `{}` requires --confirm because it is `{}`; inspect first with `trail agent action {} {} --print`",
            action.id, action.safety, review_data.task.lane, action.id
        )));
    }

    let lane = review_data.task.lane.clone();
    match action.id.as_str() {
        "open_focus_file" => run_agent_shell_action(ctx, &action.command),
        "inspect_focus_file" => {
            let report = db.agent_focus(&lane, action.path.as_deref(), false)?;
            render_agent_focus(&report, ctx.json, &ctx.render)
        }
        "show_focus_patch" => {
            let report = db.agent_focus(&lane, action.path.as_deref(), true)?;
            render_agent_focus(&report, ctx.json, &ctx.render)
        }
        "mark_focus_file_reviewed" => {
            let path = action.path.as_deref().ok_or_else(|| {
                Error::InvalidInput(
                    "mark_focus_file_reviewed action did not include a file path".to_string(),
                )
            })?;
            let report = db.agent_mark_file_reviewed(&lane, path, args.note)?;
            render_agent_mark_file_reviewed(&report, ctx.json, &ctx.render)
        }
        "show_review_map" => {
            let report = db.agent_review_map(&lane)?;
            render_agent_review_map(&report, ctx.json, &ctx.render)
        }
        "show_test_plan" => {
            let report = db.agent_test_plan(&lane)?;
            render_agent_test_plan(&report, ctx.json, &ctx.render)
        }
        "validation_next" => {
            if ctx.json {
                return Err(Error::InvalidInput(
                    "validation_next runs an open-world shell command and cannot produce one JSON report; use --print or run the printed command directly".to_string(),
                ));
            }
            run_agent_shell_action(ctx, &action.command)
        }
        "apply_dry_run" => {
            let report = db.agent_apply(&lane, true, args.message)?;
            render_agent_apply(&report, ctx.json, &ctx.render)
        }
        "apply_task" => {
            let report = db.agent_finish(&lane, false, args.message, args.note)?;
            render_agent_finish(&report, ctx.json, &ctx.render)
        }
        "mark_task_reviewed" => {
            let report = db.agent_mark_reviewed(&lane, args.note)?;
            render_agent_mark_reviewed(&report, ctx.json, &ctx.render)
        }
        _ => Err(Error::InvalidInput(format!(
            "agent action `{}` is not executable by this Trail version; run `{}` directly",
            action.id, action.command
        ))),
    }
}

fn handle_agent_empty_action(
    ctx: &RuntimeContext,
    action_id: &str,
    args: &AgentActionArgs,
) -> Result<()> {
    let action = agent_empty_action_palette_actions()
        .into_iter()
        .find(|action| agent_action_matches(action, action_id))
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent action `{action_id}` was not found; run `trail agent action` to see first-run actions"
            ))
        })?;

    if args.print {
        if ctx.json {
            return render_json(&serde_json::json!({
                "status": "empty",
                "task": null,
                "action": action
            }));
        }
        render_raw_content(&format!("{}\n", action.command), &ctx.render)?;
        return Ok(());
    }
    if action.requires_confirmation && !args.confirm {
        return Err(Error::InvalidInput(format!(
            "agent action `{}` requires --confirm because it is `{}`; inspect first with `trail agent action {} --print`",
            action.id, action.safety, action.id
        )));
    }

    match action.id.as_str() {
        "setup_vscode" => handle_agent_setup(
            ctx,
            AgentSetupArgs {
                provider: "claude-code".to_string(),
                editor: "vscode".to_string(),
            },
        ),
        "setup_codex_vscode" => handle_agent_setup(
            ctx,
            AgentSetupArgs {
                provider: "codex".to_string(),
                editor: "vscode".to_string(),
            },
        ),
        "setup_cursor_vscode" => handle_agent_setup(
            ctx,
            AgentSetupArgs {
                provider: "cursor".to_string(),
                editor: "vscode".to_string(),
            },
        ),
        "doctor_claude_code" => handle_agent_doctor(
            ctx,
            AgentDoctorArgs {
                provider: "claude-code".to_string(),
            },
        ),
        "doctor_codex" => handle_agent_doctor(
            ctx,
            AgentDoctorArgs {
                provider: "codex".to_string(),
            },
        ),
        "doctor_cursor" => handle_agent_doctor(
            ctx,
            AgentDoctorArgs {
                provider: "cursor".to_string(),
            },
        ),
        "start_terminal_task" => handle_agent_start(
            ctx,
            AgentStartArgs {
                provider: "claude-code".to_string(),
                name: None,
                from: None,
                workdir_mode: "full-cow".to_string(),
                command: Vec::new(),
            },
        ),
        "start_gemini_task" => handle_agent_start(
            ctx,
            AgentStartArgs {
                provider: "gemini".to_string(),
                name: None,
                from: None,
                workdir_mode: "full-cow".to_string(),
                command: Vec::new(),
            },
        ),
        _ => Err(Error::InvalidInput(format!(
            "agent action `{}` is not executable by this Trail version; run `{}` directly",
            action.id, action.command
        ))),
    }
}

fn agent_action_selector_args(
    db: &mut trail::Trail,
    args: &AgentActionArgs,
) -> Result<(String, Option<String>)> {
    match (&args.selector_or_action, &args.action) {
        (None, None) => Ok(("latest".to_string(), None)),
        (Some(selector), Some(action)) => Ok((selector.clone(), Some(action.clone()))),
        (Some(value), None) => match db.agent_review_data("latest") {
            Ok(latest)
                if latest
                    .actions
                    .iter()
                    .any(|action| agent_action_matches(action, value)) =>
            {
                Ok(("latest".to_string(), Some(value.clone())))
            }
            Err(Error::InvalidInput(message))
                if message.contains("no agent tasks")
                    && agent_empty_action_palette_actions()
                        .iter()
                        .any(|action| agent_action_matches(action, value)) =>
            {
                Ok(("latest".to_string(), Some(value.clone())))
            }
            _ => Ok((value.clone(), None)),
        },
        (None, Some(action)) => Ok(("latest".to_string(), Some(action.clone()))),
    }
}

fn agent_action_matches(action: &AgentReviewAction, requested: &str) -> bool {
    action.id == requested
        || action.kind == requested
        || agent_action_key(&action.label) == agent_action_key(requested)
}

fn agent_action_key(value: &str) -> String {
    value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch == '_' || ch == '-' || ch.is_ascii_whitespace() {
                Some('_')
            } else {
                None
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn run_agent_shell_action(ctx: &RuntimeContext, command: &str) -> Result<()> {
    if ctx.json {
        return Err(Error::InvalidInput(
            "shell-backed agent actions cannot produce one JSON report; use --print to inspect the command".to_string(),
        ));
    }
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "sh".into());
    let status = ProcessCommand::new(shell)
        .arg("-c")
        .arg(command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(Error::from)?;
    if !status.success() {
        return Err(Error::InvalidInput(format!(
            "agent action command failed with status {}: {command}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string())
        )));
    }
    render_document(
        &TerminalDocument::new("Agent action completed", UiTone::Success).block(UiBlock::Metadata(
            vec![("Command".to_string(), command.to_string())],
        )),
        &ctx.render,
    )?;
    Ok(())
}

fn handle_agent_review_flow(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_review_flow(&args.selector)?;
    render_agent_review_flow(&report, ctx.json, &ctx.render)
}

fn handle_agent_inbox(ctx: &RuntimeContext, args: AgentInboxArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_inbox_with_options(args.all)?;
    render_agent_inbox(&report, ctx.json, &ctx.render)
}

fn handle_agent_board(ctx: &RuntimeContext, args: AgentInboxArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_board_with_options(args.all)?;
    render_agent_board(&report, ctx.json, &ctx.render)
}

fn handle_agent_stack(ctx: &RuntimeContext, args: AgentInboxArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_stack_with_options(args.all)?;
    render_agent_stack(&report, ctx.json, &ctx.render)
}

fn handle_agent_next(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_next(&args.selector)?;
    render_agent_next(&report, ctx.json, &ctx.render)
}

fn handle_agent_list(ctx: &RuntimeContext, args: AgentListArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.list_agent_tasks_with_options(args.all)?;
    render_agent_list(&report, ctx.json, &ctx.render)
}

fn handle_agent_brief(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_brief(&args.selector)?;
    render_agent_brief(&report, ctx.json, &ctx.render)
}

fn handle_agent_summary(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_summary(&args.selector)?;
    render_agent_summary(&report, ctx.json, &ctx.render)
}

fn handle_agent_validate(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_validate(&args.selector)?;
    render_agent_validate(&report, ctx.json, &ctx.render)
}

fn handle_agent_test_plan(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_test_plan(&args.selector)?;
    render_agent_test_plan(&report, ctx.json, &ctx.render)
}

fn handle_agent_report(ctx: &RuntimeContext, args: AgentReportArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_report(&args.selector)?;
    render_agent_report(&report, ctx.json, &ctx.render, args.markdown)
}

fn handle_agent_handoff(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_handoff(&args.selector)?;
    render_agent_handoff(&report, ctx.json, &ctx.render)
}

fn handle_agent_receipt(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_receipt(&args.selector)?;
    render_agent_receipt(&report, ctx.json, &ctx.render)
}

fn handle_agent_pr(ctx: &RuntimeContext, args: AgentPrArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_pr_draft(&args.selector)?;
    render_agent_pr(
        &report,
        ctx.json,
        &ctx.render,
        args.title_only,
        args.body_only,
    )
}

fn handle_agent_story(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_story(&args.selector)?;
    render_agent_story(&report, ctx.json, &ctx.render)
}

fn handle_agent_tools(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_tools(&args.selector)?;
    render_agent_tools(&report, ctx.json, &ctx.render)
}

fn handle_agent_impact(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_impact(&args.selector)?;
    render_agent_impact(&report, ctx.json, &ctx.render)
}

fn handle_agent_review_map(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_review_map(&args.selector)?;
    render_agent_review_map(&report, ctx.json, &ctx.render)
}

fn handle_agent_risk(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_risk(&args.selector)?;
    render_agent_risk(&report, ctx.json, &ctx.render)
}

fn handle_agent_confidence(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_confidence(&args.selector)?;
    render_agent_confidence(&report, ctx.json, &ctx.render)
}

fn handle_agent_ready(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_ready(&args.selector)?;
    render_agent_ready(&report, ctx.json, &ctx.render)
}

fn handle_agent_diagnose(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_diagnose(&args.selector)?;
    render_agent_diagnose(&report, ctx.json, &ctx.render)
}

fn handle_agent_compare(ctx: &RuntimeContext, args: AgentCompareArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_compare(&args.left, &args.right)?;
    render_agent_compare(&report, ctx.json, &ctx.render)
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
    let render_result = render_lane_test(&report, ctx.json, &ctx.render);
    if render_result.is_ok() && !report.success {
        std::process::exit(command_failure_exit_code(report.exit_code));
    }
    render_result
}

fn handle_agent_workdir(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_workdir(&args.selector)?;
    render_agent_workdir(&report, ctx.json, &ctx.render)
}

fn handle_agent_view(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    match db.agent_task_view(&args.selector) {
        Ok(report) => render_agent_view(&report, ctx.json, &ctx.render),
        Err(Error::InvalidInput(message)) if message.contains("no agent tasks") => {
            render_agent_empty_task_hint("view", ctx.json, &ctx.render)
        }
        Err(err) => Err(err),
    }
}

fn handle_agent_changes(ctx: &RuntimeContext, args: AgentChangesArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let _ = args.by_turn;
    match db.agent_changes_with_options(&args.selector, args.by_operation, args.by_file) {
        Ok(report) => render_agent_changes(&report, ctx.json, &ctx.render),
        Err(Error::InvalidInput(message)) if message.contains("no agent tasks") => {
            render_agent_empty_task_hint("changes", ctx.json, &ctx.render)
        }
        Err(err) => Err(err),
    }
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
    render_agent_delta(&report, ctx.json, &ctx.render)
}

fn handle_agent_new(ctx: &RuntimeContext, args: AgentNewArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_new(&args.selector, args.file.as_deref(), args.patch)?;
    render_agent_new(&report, ctx.json, &ctx.render)
}

fn handle_agent_mark_reviewed(ctx: &RuntimeContext, args: AgentMarkReviewedArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_mark_reviewed(&args.selector, args.note)?;
    render_agent_mark_reviewed(&report, ctx.json, &ctx.render)
}

fn handle_agent_mark_file_reviewed(
    ctx: &RuntimeContext,
    args: AgentMarkFileReviewedArgs,
) -> Result<()> {
    let mut db = open_db(ctx)?;
    let (selector, path) = agent_mark_file_reviewed_selector_args(&args);
    let report = db.agent_mark_file_reviewed(&selector, &path, args.note)?;
    render_agent_mark_file_reviewed(&report, ctx.json, &ctx.render)
}

fn handle_agent_archive(
    ctx: &RuntimeContext,
    args: AgentArchiveArgs,
    archived: bool,
) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_archive(&args.selector, archived, args.note)?;
    render_agent_archive(&report, ctx.json, &ctx.render)
}

fn handle_agent_change(ctx: &RuntimeContext, args: AgentChangeArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, change_selector) = agent_change_selector_args(&args);
    let report = db.agent_change_set(&selector, &change_selector, args.patch)?;
    render_agent_change_set(&report, ctx.json, &ctx.render)
}

fn handle_agent_timeline(ctx: &RuntimeContext, args: AgentTimelineArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let _ = args.by_turn;
    let report = db.agent_timeline(&args.selector, args.by_operation)?;
    render_agent_timeline(&report, ctx.json, &ctx.render)
}

fn handle_agent_files(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_files(&args.selector)?;
    render_agent_files(&report, ctx.json, &ctx.render)
}

fn handle_agent_file(ctx: &RuntimeContext, args: AgentFileArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, path) = agent_file_selector_args(&args);
    let report = db.agent_file(&selector, &path, args.patch)?;
    render_agent_file(&report, ctx.json, &ctx.render)
}

fn handle_agent_checkpoints(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_checkpoints(&args.selector)?;
    render_agent_checkpoints(&report, ctx.json, &ctx.render)
}

fn handle_agent_why(ctx: &RuntimeContext, args: AgentWhyArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, path) = match args.path {
        Some(path) => (args.selector_or_path, path),
        None => ("latest".to_string(), args.selector_or_path),
    };
    let report = db.agent_why(&selector, &path)?;
    render_agent_why(&report, ctx.json, &ctx.render)
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

fn agent_mark_file_reviewed_selector_args(args: &AgentMarkFileReviewedArgs) -> (String, String) {
    match &args.path {
        Some(path) => (args.selector_or_path.clone(), path.clone()),
        None => ("latest".to_string(), args.selector_or_path.clone()),
    }
}

fn handle_agent_turn(ctx: &RuntimeContext, args: AgentTurnArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let (selector, turn) = resolve_agent_turn_cli_args(&db, &args);
    let report = db.agent_turn(&selector, &turn, args.file.as_deref(), args.patch)?;
    render_agent_turn(&report, ctx.json, &ctx.render)
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
    render_agent_diff(&report, ctx.json, &ctx.render, args.stat)
}

fn resolve_agent_turn_cli_args(db: &trail::Trail, args: &AgentTurnArgs) -> (String, String) {
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
    render_agent_diff(&report, ctx.json, &ctx.render, args.stat)
}

fn handle_agent_review(ctx: &RuntimeContext, args: AgentSelectorArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_review(&args.selector)?;
    render_agent_review(&report, ctx.json, &ctx.render)
}

fn handle_agent_focus(ctx: &RuntimeContext, args: AgentFocusArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_focus(&args.selector, args.file.as_deref(), args.patch)?;
    render_agent_focus(&report, ctx.json, &ctx.render)
}

fn handle_agent_open(ctx: &RuntimeContext, args: AgentOpenArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.agent_focus(&args.selector, args.file.as_deref(), false)?;
    let open_path = report.open_path.clone().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` has no materialized workdir to open; run `trail agent focus {}` to inspect it without opening an editor",
            report.task.name, report.task.name
        ))
    })?;
    let open_command = report.open_command.clone().ok_or_else(|| {
        Error::InvalidInput(format!(
            "agent task `{}` did not produce an editor command",
            report.task.name
        ))
    })?;
    if ctx.json {
        return render_agent_focus(&report, true, &ctx.render);
    }
    if args.print {
        render_raw_content(&format!("{open_command}\n"), &ctx.render)?;
        return Ok(());
    }
    let shell = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "sh".into());
    let status = ProcessCommand::new(shell)
        .arg("-c")
        .arg(&open_command)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(Error::from)?;
    if !status.success() {
        return Err(Error::InvalidInput(format!(
            "editor command failed with status {}: {open_command}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string())
        )));
    }
    render_document(
        &TerminalDocument::new("Opened agent focus", UiTone::Success)
            .block(UiBlock::Metadata(vec![("Path".to_string(), open_path)])),
        &ctx.render,
    )?;
    Ok(())
}

fn handle_agent_apply(ctx: &RuntimeContext, args: AgentApplyArgs) -> Result<()> {
    let _ = args.into_current_git_branch;
    let mut db = open_db(ctx)?;
    match db.agent_apply(&args.selector, args.dry_run, args.message) {
        Ok(report) => render_agent_apply(&report, ctx.json, &ctx.render),
        Err(Error::InvalidInput(message)) if message.contains("no agent tasks") => {
            render_agent_empty_task_hint("apply", ctx.json, &ctx.render)
        }
        Err(err) => Err(err),
    }
}

fn handle_agent_finish(ctx: &RuntimeContext, args: AgentFinishArgs) -> Result<()> {
    let _ = args.into_current_git_branch;
    let mut db = open_db(ctx)?;
    let report = db.agent_finish(&args.selector, args.dry_run, args.message, args.note)?;
    render_agent_finish(&report, ctx.json, &ctx.render)
}

fn handle_agent_rewind(ctx: &RuntimeContext, args: AgentRewindArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.agent_rewind(&args.selector, &args.target)?;
    render_lane_rewind(&report, ctx.json, &ctx.render)
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
    render_lane_rewind(&report, ctx.json, &ctx.render)
}

fn handle_agent_doctor(ctx: &RuntimeContext, args: AgentDoctorArgs) -> Result<()> {
    let mut checks = Vec::new();
    let mut status = "ok";
    let mut workspace_ok = true;
    match open_db(ctx) {
        Ok(_) => checks.push(serde_json::json!({
            "name": "workspace",
            "status": "ok",
            "message": "Trail workspace opened"
        })),
        Err(err) => {
            status = "failed";
            workspace_ok = false;
            checks.push(serde_json::json!({
                "name": "workspace",
                "status": "failed",
                "message": err.to_string()
            }));
        }
    }
    let profile = trail::acp::agent_provider_profile(&args.provider)?;
    checks.push(serde_json::json!({
        "name": "provider",
        "status": "ok",
        "message": format!("{} profile loaded", profile.agent)
    }));

    let mut launch_ok = false;
    if profile.supports_acp {
        if profile.available {
            launch_ok = true;
            checks.push(serde_json::json!({
                "name": "acp",
                "status": "ok",
                "message": profile.notes.join("; ")
            }));
        } else {
            checks.push(serde_json::json!({
                "name": "acp",
                "status": "warning",
                "message": profile.notes.join("; ")
            }));
        }
    } else {
        checks.push(serde_json::json!({
            "name": "acp",
            "status": "skipped",
            "message": "provider does not advertise an ACP entrypoint; use terminal mode"
        }));
    }

    if let Some(command) = &profile.default_terminal_command {
        let launcher = command
            .first()
            .map(String::as_str)
            .unwrap_or(&profile.agent);
        if command_available(launcher) {
            launch_ok = true;
            checks.push(serde_json::json!({
                "name": "terminal",
                "status": "ok",
                "message": format!("default terminal command `{}` is available", command.join(" "))
            }));
        } else {
            checks.push(serde_json::json!({
                "name": "terminal",
                "status": "warning",
                "message": format!("default terminal command `{}` was not found on PATH", command.join(" "))
            }));
        }
    } else {
        checks.push(serde_json::json!({
            "name": "terminal",
            "status": "skipped",
            "message": "provider does not define a default terminal command"
        }));
    }

    if profile.supports_mcp {
        checks.push(serde_json::json!({
            "name": "mcp",
            "status": "ok",
            "message": "Trail exposes `trail mcp` for provider-side context tools"
        }));
    } else {
        checks.push(serde_json::json!({
            "name": "mcp",
            "status": "skipped",
            "message": "no built-in MCP setup note for this provider"
        }));
    }

    if workspace_ok && !launch_ok {
        status = "failed";
    }
    let setup_command = if profile.agent == "claude-code" {
        "trail agent setup".to_string()
    } else {
        format!(
            "trail agent setup --provider {} --editor vscode",
            profile.agent
        )
    };
    let mut suggestions = vec![serde_json::json!({
        "command": setup_command,
        "reason": "print the recommended Trail setup for this provider"
    })];
    if profile.supports_terminal {
        suggestions.push(serde_json::json!({
            "command": format!("trail agent start --provider {}", profile.agent),
            "reason": "launch a fresh materialized task lane from the terminal"
        }));
    }
    if profile.supports_acp {
        suggestions.push(serde_json::json!({
            "command": format!("trail acp doctor --agent {}", profile.agent),
            "reason": "check the lower-level ACP relay command"
        }));
    }
    if profile.supports_mcp {
        suggestions.push(serde_json::json!({
            "command": "trail mcp",
            "reason": "stdio MCP server command to register in the native agent"
        }));
    }
    let report = serde_json::json!({
        "status": status,
        "provider": profile.agent,
        "display_name": profile.display_name,
        "capabilities": {
            "acp": profile.supports_acp,
            "mcp": profile.supports_mcp,
            "terminal": profile.supports_terminal
        },
        "relay_command": profile.relay_command,
        "default_terminal_command": profile.default_terminal_command,
        "checks": checks,
        "suggestions": suggestions
    });
    if ctx.json {
        return render_json(&report);
    }
    let check_rows = checks
        .iter()
        .map(|check| {
            UiCheck::new(
                match check["status"].as_str().unwrap_or_default() {
                    "ok" | "pass" => UiCheckState::Pass,
                    "warn" | "warning" => UiCheckState::Warn,
                    "pending" => UiCheckState::Pending,
                    _ => UiCheckState::Fail,
                },
                check["name"].as_str().unwrap_or("check"),
                check["message"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    render_document(
        &TerminalDocument::new(
            format!("Agent diagnostics: {status}"),
            if status == "ok" {
                UiTone::Success
            } else {
                UiTone::Attention
            },
        )
        .block(UiBlock::Checklist(check_rows)),
        &ctx.render,
    )
}

fn default_terminal_agent_command(provider: &str) -> Result<Vec<String>> {
    trail::acp::terminal_agent_command(provider)
}

fn confined_terminal_agent_command(
    command: &[String],
    workspace_root: &Path,
    workdir: &Path,
) -> Result<(std::ffi::OsString, Vec<std::ffi::OsString>)> {
    #[cfg(target_os = "macos")]
    {
        let sandbox = PathBuf::from("/usr/bin/sandbox-exec");
        if !sandbox.is_file() {
            return Err(Error::InvalidInput(
                "isolated terminal agents require `/usr/bin/sandbox-exec` on macOS".to_string(),
            ));
        }
        let workspace_root = workspace_root.canonicalize()?;
        let workdir = workdir.canonicalize()?;
        let trail_internal = workspace_root.join(".trail").canonicalize()?;
        let escape = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let profile = format!(
            "(version 1)\n\
             (allow default)\n\
             (deny file-write* (subpath \"{}\"))\n\
             (allow file-write* (subpath \"{}\") (subpath \"{}\"))",
            escape(&workspace_root),
            escape(&trail_internal),
            escape(&workdir),
        );
        let mut args = vec![
            std::ffi::OsString::from("-p"),
            std::ffi::OsString::from(profile),
            std::ffi::OsString::from(&command[0]),
        ];
        args.extend(command[1..].iter().map(std::ffi::OsString::from));
        Ok((sandbox.into_os_string(), args))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (workspace_root, workdir);
        Ok((
            std::ffi::OsString::from(&command[0]),
            command[1..].iter().map(std::ffi::OsString::from).collect(),
        ))
    }
}

fn command_available(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return std::path::Path::new(command).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}
