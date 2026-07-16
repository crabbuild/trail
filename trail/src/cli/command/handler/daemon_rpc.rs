use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use trail::model::*;

use super::*;

const PERFORMANCE_METRICS_FILE_ENV: &str = "TRAIL_PERFORMANCE_METRICS_FILE";
const OPERATION_METRICS_HEADER: &str = "x-trail-operation-metrics";
const DAEMON_READ_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const DAEMON_MUTATING_REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(super) fn try_handle_auto_daemon_command(
    ctx: &RuntimeContext,
    daemon_token: Option<String>,
    command: &Command,
) -> Result<bool> {
    if !daemon_supports_command(command) {
        return Ok(false);
    }
    if matches!(
        command,
        Command::Status(_)
            | Command::Diff(_)
            | Command::Record(_)
            | Command::Index(IndexCommand {
                command: IndexSubcommand::Reconcile(_),
            })
    ) || matches!(
        command,
        Command::Lane(LaneCommand {
            command: LaneSubcommand::Status(_)
                | LaneSubcommand::Record(_)
                | LaneSubcommand::ApplyPatch(_)
                | LaneSubcommand::Diff(_),
        })
    ) {
        let workspace = daemon_start::workspace_from_context(ctx)?;
        if !workspace.join(".trail").is_dir() {
            return Err(Error::WorkspaceNotFound(workspace));
        }
        let ready =
            daemon_start::ensure_workspace_daemon_ready(&workspace, daemon_token.as_deref())?;
        let result = try_handle_daemon_command(
            ctx,
            Some(ready.url.clone()),
            Some(ready.auth_token.clone()),
            command,
        );
        match result {
            Ok(handled) => return Ok(handled),
            Err(error) => {
                if daemon_command_requires_one_recovery_retry(&error) {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    loop {
                        match daemon_start::ensure_workspace_daemon_ready(
                            &workspace,
                            daemon_token.as_deref(),
                        ) {
                            Ok(recovered) => {
                                return try_handle_daemon_command(
                                    ctx,
                                    Some(recovered.url),
                                    Some(recovered.auth_token),
                                    command,
                                );
                            }
                            Err(_) if std::time::Instant::now() < deadline => {
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(_) => return Err(error),
                        }
                    }
                }
                return Err(error);
            }
        }
    }
    let workspace = daemon_start::workspace_from_context(ctx)?;
    if workspace.join(".trail").is_dir() {
        if let Some(ready) =
            daemon_start::existing_workspace_daemon_ready(&workspace, daemon_token.as_deref())?
        {
            return try_handle_daemon_command(
                ctx,
                Some(ready.url),
                Some(ready.auth_token),
                command,
            );
        }
    }
    let Some(daemon_url) = discover_daemon_url(ctx)? else {
        return Ok(false);
    };
    match try_handle_daemon_command(ctx, Some(daemon_url), daemon_token, command) {
        Ok(handled) => Ok(handled),
        Err(err) if auto_daemon_should_fallback(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn daemon_command_requires_one_recovery_retry(error: &Error) -> bool {
    matches!(
        error,
        Error::ChangeLedgerReconcileRequired { .. } | Error::DaemonError { exit_code: 16, .. }
    )
}

pub(super) fn try_handle_daemon_command(
    ctx: &RuntimeContext,
    daemon_url: Option<String>,
    daemon_token: Option<String>,
    command: &Command,
) -> Result<bool> {
    let Some(daemon_url) = daemon_url else {
        return Ok(false);
    };
    if !daemon_supports_command(command) {
        return Ok(false);
    }

    let token = resolve_daemon_token(ctx, daemon_token)?;
    let client = DaemonClient::new(&daemon_url, token)?;
    match command {
        Command::Status(args) => {
            if args.branch.is_some() {
                return Ok(false);
            }
            let report: StatusReport = client.get_json("/v1/status")?;
            render_status(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        Command::Diff(args) => {
            let path = diff_path(args)?;
            let summary: DiffSummary = client.get_json(&path)?;
            render_diff(
                &summary,
                ctx.json,
                &ctx.render,
                args.patch,
                args.stat,
                args.name_only,
                args.name_status,
            )?;
            Ok(true)
        }
        Command::Record(args) => {
            let body = serde_json::json!({
                "ref_name": ctx.branch,
                "message": args.message,
                "paths": args.paths,
                "kind": args.kind,
                "session_id": args.session,
                "allow_ignored": args.allow_ignored,
            });
            let report: RecordReport = client.post_json("/v1/record", &body)?;
            render_record(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        Command::Index(IndexCommand {
            command: IndexSubcommand::Reconcile(args),
        }) => {
            let body = serde_json::json!({ "lane": args.lane });
            let report: ChangeLedgerReconcileReport =
                client.post_json("/v1/index/reconcile", &body)?;
            if matches!(ctx.format, OutputFormat::Ndjson) {
                render_ndjson(&report)?;
            } else {
                render_change_ledger_reconcile(&report, ctx.json, &ctx.render)?;
            }
            Ok(true)
        }
        Command::Timeline(args) => handle_timeline_command(ctx, &client, args),
        Command::Why(args) => handle_why_command(ctx, &client, args),
        Command::History(args) => handle_history_command(ctx, &client, args),
        Command::CodeFrom(args) => handle_code_from_command(ctx, &client, args),
        Command::Lane(lane) => handle_lane_command(ctx, &client, lane),
        Command::Session(session) => handle_session_command(ctx, &client, session),
        Command::Approvals(approvals) => handle_approvals_command(ctx, &client, approvals),
        Command::Lease(lease) => handle_lease_command(ctx, &client, lease),
        Command::Doctor => {
            let report: DoctorReport = client.get_json("/v1/doctor")?;
            render_doctor(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn daemon_supports_command(command: &Command) -> bool {
    match command {
        Command::Status(args) => args.branch.is_none(),
        Command::Record(_)
        | Command::Index(IndexCommand {
            command: IndexSubcommand::Reconcile(_),
        })
        | Command::Diff(_)
        | Command::Timeline(_)
        | Command::Why(_)
        | Command::History(_)
        | Command::CodeFrom(_)
        | Command::Session(_)
        | Command::Approvals(_)
        | Command::Lease(_)
        | Command::Doctor => true,
        Command::Lane(lane) => match &lane.command {
            LaneSubcommand::List
            | LaneSubcommand::Show(_)
            | LaneSubcommand::Status(_)
            | LaneSubcommand::Review(_)
            | LaneSubcommand::Contribution(_)
            | LaneSubcommand::Gates(_)
            | LaneSubcommand::Readiness(_)
            | LaneSubcommand::Merge(_)
            | LaneSubcommand::MergeQueue(_)
            | LaneSubcommand::RefreshPreview(_)
            | LaneSubcommand::Handoff(_)
            | LaneSubcommand::Claim(_)
            | LaneSubcommand::Record(_)
            | LaneSubcommand::Rewind(_)
            | LaneSubcommand::Events(_)
            | LaneSubcommand::Read(_)
            | LaneSubcommand::Workdir(_)
            | LaneSubcommand::SyncWorkdir(_)
            | LaneSubcommand::ApplyPatch(_)
            | LaneSubcommand::Diff(_)
            | LaneSubcommand::Timeline(_) => true,
            LaneSubcommand::Turn(_) => true,
            LaneSubcommand::Trace(_) => true,
            _ => false,
        },
        _ => false,
    }
}

fn auto_daemon_should_fallback(err: &Error) -> bool {
    matches!(err, Error::DaemonUnavailable(_))
}

fn handle_lane_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    lane: &LaneCommand,
) -> Result<bool> {
    match &lane.command {
        LaneSubcommand::Spawn(args) => {
            let mut body = Map::new();
            body.insert("name".to_string(), Value::String(args.name.clone()));
            if let Some(from) = &args.from {
                body.insert("from_ref".to_string(), Value::String(from.clone()));
            }
            if args.no_materialize {
                body.insert("materialize".to_string(), Value::Bool(false));
            } else if let Some(materialize) = args.materialize {
                body.insert("materialize".to_string(), Value::Bool(materialize));
            }
            if let Some(workdir_mode) = &args.workdir_mode {
                body.insert(
                    "workdir_mode".to_string(),
                    Value::String(workdir_mode.clone()),
                );
            }
            if let Some(workdir) = &args.workdir {
                body.insert(
                    "workdir".to_string(),
                    Value::String(workdir.to_string_lossy().to_string()),
                );
            }
            if !args.paths.is_empty() {
                body.insert(
                    "paths".to_string(),
                    Value::Array(args.paths.iter().cloned().map(Value::String).collect()),
                );
            }
            if args.include_neighbors {
                body.insert("include_neighbors".to_string(), Value::Bool(true));
            }
            if let Some(provider) = &args.provider {
                body.insert("provider".to_string(), Value::String(provider.clone()));
            }
            if let Some(model) = &args.model {
                body.insert("model".to_string(), Value::String(model.clone()));
            }
            let report: LaneSpawnReport = client.post_json("/v1/lanes", &Value::Object(body))?;
            render_lane_spawn(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::List => {
            let lanes: Vec<LaneDetails> = client.get_json("/v1/lanes")?;
            render_lane_list(&lanes, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Show(args) => {
            let details: LaneDetails = client.get_json(&format!("/v1/lanes/{}", args.name))?;
            render_lane_details(&details, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Status(args) => {
            let report: LaneStatusReport =
                client.get_json(&format!("/v1/lanes/{}/status", args.name))?;
            render_lane_status(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Review(args) => {
            let report: LaneReviewPacketReport = client.get_json(&format!(
                "/v1/lanes/{}/review?limit={}",
                args.name, args.limit
            ))?;
            render_lane_review_packet(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Contribution(args) => {
            let report: LaneContributionReport = client.get_json(&format!(
                "/v1/lanes/{}/contribution?limit={}",
                args.name, args.limit
            ))?;
            render_lane_contribution(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Gates(args) => {
            let mut params = vec![format!("limit={}", args.limit)];
            if let Some(kind) = &args.kind {
                params.push(format!("kind={kind}"));
            }
            let path = append_query(&format!("/v1/lanes/{}/gates", args.name), params);
            let report: LaneGateHistoryReport = client.get_json(&path)?;
            render_lane_gate_history(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Readiness(args) => {
            let report: LaneReadinessReport =
                client.get_json(&format!("/v1/lanes/{}/readiness", args.name))?;
            render_lane_readiness(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Merge(args) => {
            validate_merge_strategy(args.strategy.as_deref())?;
            let body = serde_json::json!({
                "into": args.into,
                "strategy": args.strategy,
                "dry_run": args.dry_run,
                "direct": args.direct,
            });
            let report: MergeReport =
                client.post_json(&format!("/v1/lanes/{}/merge", args.name), &body)?;
            render_merge(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::MergeQueue(queue) => handle_lane_merge_queue_command(ctx, client, queue),
        LaneSubcommand::RefreshPreview(args) => {
            let path = append_query(
                &format!("/v1/lanes/{}/refresh-preview", args.name),
                vec![format!("target={}", args.target)],
            );
            let report: LaneRefreshPreviewReport = client.get_json(&path)?;
            render_lane_refresh_preview(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Handoff(args) => {
            let report: LaneHandoffReport = client.get_json(&format!(
                "/v1/lanes/{}/handoff?limit={}",
                args.name, args.limit
            ))?;
            render_lane_handoff(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Claim(args) => {
            let body = serde_json::json!({
                "path": args.path,
                "ttl_secs": args.ttl_secs,
            });
            let report: LaneClaimReport =
                client.post_json(&format!("/v1/lanes/{}/claims", args.name), &body)?;
            render_lane_claim(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Record(args) => {
            let body = serde_json::json!({
                "message": args.message,
                "preview": args.preview,
            });
            if args.preview {
                let report: LaneRecordPreviewReport =
                    client.post_json(&format!("/v1/lanes/{}/record", args.name), &body)?;
                render_lane_record_preview(&report, ctx.json, &ctx.render)?;
            } else {
                let report: LaneRecordReport =
                    client.post_json(&format!("/v1/lanes/{}/record", args.name), &body)?;
                render_lane_record(&report, ctx.json, &ctx.render)?;
            }
            Ok(true)
        }
        LaneSubcommand::Rewind(args) => {
            let body = serde_json::json!({
                "to": args.target,
                "record_current": args.record_current,
                "sync_workdir": args.sync_workdir,
            });
            let report: LaneRewindReport =
                client.post_json(&format!("/v1/lanes/{}/rewind", args.name), &body)?;
            render_lane_rewind(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Events(args) => {
            let mut params = vec![format!("limit={}", args.limit)];
            if let Some(lane) = &args.lane {
                params.push(format!("lane={lane}"));
            }
            if let Some(session) = &args.session {
                params.push(format!("session={session}"));
            }
            if let Some(turn) = &args.turn {
                params.push(format!("turn={turn}"));
            }
            if let Some(event_type) = &args.event_type {
                params.push(format!("type={event_type}"));
            }
            let path = append_query("/v1/lane/events", params);
            let events: Vec<LaneEventRecord> = client.get_json(&path)?;
            render_lane_events(&events, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::SyncWorkdir(args) => {
            let body = serde_json::json!({
                "force": args.force,
                "paths": args.paths,
                "include_neighbors": args.include_neighbors,
            });
            let report: LaneWorkdirSyncReport =
                client.post_json(&format!("/v1/lanes/{}/sync-workdir", args.name), &body)?;
            render_lane_workdir_sync(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Read(args) => {
            let mut body = Map::new();
            body.insert("path".to_string(), Value::String(args.path.clone()));
            if args.hydrate {
                body.insert("hydrate".to_string(), Value::Bool(true));
            } else if args.no_hydrate {
                body.insert("hydrate".to_string(), Value::Bool(false));
            }
            body.insert("force".to_string(), Value::Bool(args.force));
            body.insert(
                "include_neighbors".to_string(),
                Value::Bool(args.include_neighbors),
            );
            let report: LaneFileReadReport = client.post_json(
                &format!("/v1/lanes/{}/read-file", args.name),
                &Value::Object(body),
            )?;
            render_lane_file_read(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Workdir(args) => {
            let report: LaneWorkdirReport =
                client.get_json(&format!("/v1/lanes/{}/workdir", args.name))?;
            render_lane_workdir(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::ApplyPatch(args) => {
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            if args.allow_stale {
                patch.allow_stale = true;
            }
            let body = serde_json::to_value(&patch)?;
            let report: LanePatchReport =
                client.post_json(&format!("/v1/lanes/{}/patches", args.name), &body)?;
            render_lane_patch(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Diff(args) => {
            validate_diff_view(
                args.patch,
                args.stat,
                args.show_line_ids,
                args.name_only,
                args.name_status,
            )?;
            let mut params = Vec::new();
            if args.patch {
                params.push("patch=1".to_string());
            }
            if args.show_line_ids {
                params.push("show_line_ids=1".to_string());
            }
            let path = append_query(&format!("/v1/lanes/{}/diff", args.name), params);
            let summary: DiffSummary = client.get_json(&path)?;
            let title = format!("Lane diff: {}", args.name);
            render_diff_with_title(
                &summary,
                ctx.json,
                &ctx.render,
                args.patch,
                args.stat,
                args.name_only,
                args.name_status,
                Some(&title),
            )?;
            Ok(true)
        }
        LaneSubcommand::Timeline(args) => {
            let path = append_query(
                "/v1/timeline",
                vec![
                    format!("lane={}", args.name),
                    format!("limit={}", args.limit),
                ],
            );
            let entries: Vec<TimelineEntry> = client.get_json(&path)?;
            render_timeline(&entries, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneSubcommand::Turn(turn) => handle_lane_turn_command(ctx, client, turn),
        LaneSubcommand::Trace(trace) => handle_lane_trace_command(ctx, client, trace),
        _ => Ok(false),
    }
}

fn handle_timeline_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    args: &TimelineArgs,
) -> Result<bool> {
    let mut params = vec![format!("limit={}", args.limit)];
    if let Some(branch) = &args.branch {
        params.push(format!("branch={branch}"));
    }
    if let Some(session) = &args.session {
        params.push(format!("session={session}"));
    }
    if let Some(lane) = &args.lane {
        params.push(format!("lane={lane}"));
    }
    let entries: Vec<TimelineEntry> = client.get_json(&append_query("/v1/timeline", params))?;
    render_timeline(&entries, ctx.json, &ctx.render)?;
    Ok(true)
}

fn handle_why_command(ctx: &RuntimeContext, client: &DaemonClient, args: &WhyArgs) -> Result<bool> {
    let mut params = Vec::new();
    match (&args.path_line, &args.line_id) {
        (Some(path_line), None) => params.push(format!("path_line={path_line}")),
        (None, Some(line_id)) => params.push(format!("line_id={line_id}")),
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
    }
    if let Some(at) = args.at.as_ref().or(ctx.branch.as_ref()) {
        params.push(format!("at={at}"));
    }
    let result: WhyResult = client.get_json(&append_query("/v1/why", params))?;
    render_why(&result, ctx.json, &ctx.render)?;
    Ok(true)
}

fn handle_history_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    args: &HistoryArgs,
) -> Result<bool> {
    let params = match (
        args.selector.as_deref(),
        args.file_id.as_deref(),
        args.line_id.as_deref(),
    ) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            return Err(Error::InvalidInput(
                "history accepts one path, --file-id, or --line-id selector".to_string(),
            ));
        }
        (_, Some(file_id), None) => vec![format!("file_id={file_id}")],
        (_, None, Some(line_id)) => vec![format!("line_id={line_id}")],
        (Some(path), None, None) => vec![format!("selector={path}")],
        (None, None, None) => {
            return Err(Error::InvalidInput(
                "history requires a path, --file-id, or --line-id".to_string(),
            ));
        }
    };
    let result: HistoryResult = client.get_json(&append_query("/v1/history", params))?;
    render_history(&result, ctx.json, &ctx.render)?;
    Ok(true)
}

fn handle_code_from_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    args: &CodeFromArgs,
) -> Result<bool> {
    let result: CodeFromResult = client.get_json(&append_query(
        "/v1/code-from",
        vec![format!("selector={}", args.selector)],
    ))?;
    render_code_from(&result, ctx.json, &ctx.render)?;
    Ok(true)
}

fn handle_session_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    session: &SessionCommand,
) -> Result<bool> {
    match &session.command {
        SessionSubcommand::Start(args) => {
            let body = serde_json::json!({
                "lane": args.lane,
                "title": args.title,
                "id": args.id,
            });
            let report: LaneSessionStartReport = client.post_json("/v1/sessions", &body)?;
            render_session_start(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        SessionSubcommand::Current(args) => {
            let path = match &args.lane {
                Some(lane) => append_query("/v1/sessions/current", vec![format!("lane={lane}")]),
                None => "/v1/sessions/current".to_string(),
            };
            let reports: Vec<LaneSessionCurrentReport> = client.get_json(&path)?;
            render_session_current(&reports, ctx.json, &ctx.render)?;
            Ok(true)
        }
        SessionSubcommand::List(args) => {
            let path = match &args.lane {
                Some(lane) => append_query("/v1/sessions", vec![format!("lane={lane}")]),
                None => "/v1/sessions".to_string(),
            };
            let sessions: Vec<LaneSession> = client.get_json(&path)?;
            render_session_list(&sessions, ctx.json, &ctx.render)?;
            Ok(true)
        }
        SessionSubcommand::Show(args) => {
            let details: LaneSessionDetails =
                client.get_json(&format!("/v1/sessions/{}", args.session_id))?;
            render_session_details(&details, ctx.json, &ctx.render)?;
            Ok(true)
        }
        SessionSubcommand::Context(args) => {
            let path = append_query(
                &format!("/v1/sessions/{}/context", args.session_id),
                vec![format!("limit={}", args.limit)],
            );
            let report: LaneSessionContextReport = client.get_json(&path)?;
            render_session_context(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        SessionSubcommand::End(args) => {
            let body = serde_json::json!({
                "status": args.status,
            });
            let report: LaneSessionEndReport =
                client.post_json(&format!("/v1/sessions/{}/end", args.session_id), &body)?;
            render_session_end(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn handle_approvals_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    approvals: &ApprovalsCommand,
) -> Result<bool> {
    match &approvals.command {
        ApprovalsSubcommand::Request(args) => {
            let mut body = Map::new();
            body.insert("lane".to_string(), Value::String(args.lane.clone()));
            body.insert("action".to_string(), Value::String(args.action.clone()));
            body.insert("summary".to_string(), Value::String(args.summary.clone()));
            if let Some(payload) = parse_optional_json(args.payload_json.as_deref())? {
                body.insert("payload".to_string(), payload);
            }
            if let Some(session) = &args.session {
                body.insert("session_id".to_string(), Value::String(session.clone()));
            }
            if let Some(turn) = &args.turn {
                body.insert("turn_id".to_string(), Value::String(turn.clone()));
            }
            let report: LaneApprovalRequestReport =
                client.post_json("/v1/approvals", &Value::Object(body))?;
            render_approval_request(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        ApprovalsSubcommand::List(args) => {
            let mut params = Vec::new();
            if let Some(lane) = &args.lane {
                params.push(format!("lane={lane}"));
            }
            if let Some(status) = &args.status {
                params.push(format!("status={status}"));
            }
            let approvals: Vec<LaneApproval> =
                client.get_json(&append_query("/v1/approvals", params))?;
            render_approval_list(&approvals, ctx.json, &ctx.render)?;
            Ok(true)
        }
        ApprovalsSubcommand::Show(args) => {
            let approval: LaneApproval =
                client.get_json(&format!("/v1/approvals/{}", args.approval_id))?;
            render_approval(&approval, ctx.json, &ctx.render)?;
            Ok(true)
        }
        ApprovalsSubcommand::Decide(args) => {
            let body = serde_json::json!({
                "decision": args.decision.as_str(),
                "reviewer": args.reviewer,
                "note": args.note,
            });
            let report: LaneApprovalDecisionReport = client.post_json(
                &format!("/v1/approvals/{}/decision", args.approval_id),
                &body,
            )?;
            render_approval_decision(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn handle_lane_turn_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    turn: &LaneTurnCommand,
) -> Result<bool> {
    match &turn.command {
        LaneTurnSubcommand::Start(args) => {
            let mut body = Map::new();
            body.insert("lane".to_string(), Value::String(args.name.clone()));
            if let Some(from) = &args.from {
                body.insert("branch".to_string(), Value::String(from.clone()));
            }
            if let Some(title) = &args.title {
                body.insert("session_title".to_string(), Value::String(title.clone()));
            }
            if let Some(base_change) = &args.base_change {
                body.insert(
                    "base_change".to_string(),
                    Value::String(base_change.clone()),
                );
            }
            let report: LaneTurnStartReport =
                client.post_json("/v1/lane/turns", &Value::Object(body))?;
            render_lane_turn_start(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTurnSubcommand::Show(args) => {
            let details: LaneTurnDetails =
                client.get_json(&format!("/v1/lane/turns/{}", args.turn_id))?;
            render_lane_turn_details(&details, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTurnSubcommand::Message(args) => {
            let body = serde_json::json!({
                "role": args.role,
                "text": args.text,
            });
            let report: LaneMessageReport =
                client.post_json(&format!("/v1/lane/turns/{}/messages", args.turn_id), &body)?;
            render_lane_message(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTurnSubcommand::Event(args) => {
            let mut body = Map::new();
            body.insert(
                "event_type".to_string(),
                Value::String(args.event_type.clone()),
            );
            if let Some(payload) = parse_optional_json(args.payload_json.as_deref())? {
                body.insert("payload".to_string(), payload);
            }
            if let Some(change) = &args.change {
                body.insert("change_id".to_string(), Value::String(change.clone()));
            }
            if let Some(message) = &args.message {
                body.insert("message_id".to_string(), Value::String(message.clone()));
            }
            let report: LaneTurnEventReport = client.post_json(
                &format!("/v1/lane/turns/{}/events", args.turn_id),
                &Value::Object(body),
            )?;
            render_lane_turn_event(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTurnSubcommand::ApplyPatch(args) => {
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            if args.allow_stale {
                patch.allow_stale = true;
            }
            let body = serde_json::to_value(&patch)?;
            let report: LanePatchReport =
                client.post_json(&format!("/v1/lane/turns/{}/patches", args.turn_id), &body)?;
            render_lane_patch(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTurnSubcommand::End(args) => {
            let body = serde_json::json!({
                "status": args.status,
            });
            let report: LaneTurnEndReport =
                client.post_json(&format!("/v1/lane/turns/{}/end", args.turn_id), &body)?;
            render_lane_turn_end(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn handle_lane_trace_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    trace: &LaneTraceCommand,
) -> Result<bool> {
    match &trace.command {
        LaneTraceSubcommand::Start(args) => {
            let mut body = Map::new();
            body.insert(
                "span_type".to_string(),
                Value::String(args.span_type.clone()),
            );
            body.insert("name".to_string(), Value::String(args.name.clone()));
            if let Some(parent) = &args.parent {
                body.insert("parent".to_string(), Value::String(parent.clone()));
            }
            if let Some(trace_id) = &args.trace_id {
                body.insert("trace".to_string(), Value::String(trace_id.clone()));
            }
            if let Some(attributes) = parse_optional_json(args.attributes_json.as_deref())? {
                body.insert("attributes".to_string(), attributes);
            }
            let report: LaneTraceSpanStartReport = client.post_json(
                &format!("/v1/lane/turns/{}/spans", args.turn_id),
                &Value::Object(body),
            )?;
            render_lane_trace_span_start(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTraceSubcommand::End(args) => {
            let mut body = Map::new();
            body.insert("status".to_string(), Value::String(args.status.clone()));
            if let Some(result) = parse_optional_json(args.result_json.as_deref())? {
                body.insert("result".to_string(), result);
            }
            let report: LaneTraceSpanEndReport = client.post_json(
                &format!("/v1/lane/spans/{}/end", args.span_id),
                &Value::Object(body),
            )?;
            render_lane_trace_span_end(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTraceSubcommand::List(args) => {
            let mut params = trace_filter_params(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.trace_id.as_deref(),
            );
            params.push(format!("limit={}", args.limit));
            let path = append_query("/v1/lane/spans", params);
            let spans: Vec<LaneTraceSpan> = client.get_json(&path)?;
            render_lane_trace_spans(&spans, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTraceSubcommand::Summary(args) => {
            let mut params = trace_filter_params(
                args.lane.as_deref(),
                args.session.as_deref(),
                args.turn.as_deref(),
                args.trace_id.as_deref(),
            );
            params.push(format!("slowest={}", args.slowest_limit));
            let path = append_query("/v1/lane/spans/summary", params);
            let report: LaneTraceSummaryReport = client.get_json(&path)?;
            render_lane_trace_summary(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneTraceSubcommand::Show(args) => {
            let span: LaneTraceSpan =
                client.get_json(&format!("/v1/lane/spans/{}", args.span_id))?;
            render_lane_trace_span(&span, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn trace_filter_params(
    lane: Option<&str>,
    session: Option<&str>,
    turn: Option<&str>,
    trace_id: Option<&str>,
) -> Vec<String> {
    let mut params = Vec::new();
    if let Some(lane) = lane {
        params.push(format!("lane={lane}"));
    }
    if let Some(session) = session {
        params.push(format!("session={session}"));
    }
    if let Some(turn) = turn {
        params.push(format!("turn={turn}"));
    }
    if let Some(trace_id) = trace_id {
        params.push(format!("trace={trace_id}"));
    }
    params
}

fn handle_lane_merge_queue_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    queue: &LaneMergeQueueCommand,
) -> Result<bool> {
    match &queue.command {
        LaneMergeQueueSubcommand::Add(args) => {
            let body = serde_json::json!({
                "lane": args.lane,
                "into": args.into,
                "priority": args.priority,
            });
            let report: LaneMergeQueueAddReport =
                client.post_json("/v1/lanes/merges/queue", &body)?;
            render_lane_merge_queue_add(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneMergeQueueSubcommand::List => {
            let entries: Vec<LaneMergeQueueEntry> = client.get_json("/v1/lanes/merges/queue")?;
            render_lane_merge_queue_list(&entries, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneMergeQueueSubcommand::Explain(args) => {
            let report: LaneMergeQueueExplainReport =
                client.get_json(&format!("/v1/lanes/merges/queue/{}/explain", args.selector))?;
            render_lane_merge_queue_explain(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneMergeQueueSubcommand::Run(args) => {
            let body = match args.limit {
                Some(limit) => serde_json::json!({ "limit": limit }),
                None => serde_json::json!({}),
            };
            let report: LaneMergeQueueRunReport =
                client.post_json("/v1/lanes/merges/queue/run", &body)?;
            render_lane_merge_queue_run(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LaneMergeQueueSubcommand::Remove(args) => {
            let report: LaneMergeQueueRemoveReport =
                client.delete_json(&format!("/v1/lanes/merges/queue/{}", args.selector))?;
            render_lane_merge_queue_remove(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn handle_lease_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    lease: &LeaseCommand,
) -> Result<bool> {
    match &lease.command {
        LeaseSubcommand::Acquire(args) => {
            let body = serde_json::json!({
                "lane": args.lane,
                "path": args.path,
                "mode": args.mode.as_str(),
                "ttl_secs": args.ttl_secs,
            });
            let report: LeaseAcquireReport = client.post_json("/v1/leases", &body)?;
            render_lease_acquire(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LeaseSubcommand::List(args) => {
            let path = if args.all {
                append_query("/v1/leases", vec!["all=1".to_string()])
            } else {
                "/v1/leases".to_string()
            };
            let leases: Vec<LeaseRecord> = client.get_json(&path)?;
            render_lease_list(&leases, ctx.json, &ctx.render)?;
            Ok(true)
        }
        LeaseSubcommand::Release(args) => {
            let report: LeaseReleaseReport =
                client.delete_json(&format!("/v1/leases/{}", args.lease_id))?;
            render_lease_release(&report, ctx.json, &ctx.render)?;
            Ok(true)
        }
    }
}

fn diff_path(args: &DiffArgs) -> Result<String> {
    let forms = usize::from(args.range.is_some())
        + usize::from(args.root.is_some())
        + usize::from(args.dirty);
    if forms != 1 {
        return Err(Error::InvalidInput(
            "diff requires exactly one of RANGE, --root ROOT..ROOT, or --dirty".to_string(),
        ));
    }

    let mut params = Vec::new();
    if args.patch {
        params.push("patch=1".to_string());
    }
    if args.show_line_ids {
        params.push("show_line_ids=1".to_string());
    }
    if args.dirty {
        params.push("dirty=1".to_string());
    } else if let Some(root) = &args.root {
        params.push(format!("root={root}"));
    } else if let Some(range) = &args.range {
        params.push(format!("range={range}"));
    }
    Ok(append_query("/v1/diff", params))
}

fn append_query(path: &str, params: Vec<String>) -> String {
    if params.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", params.join("&"))
    }
}

pub(super) struct DaemonClient {
    endpoint: DaemonTransport,
    token: Option<String>,
}

impl DaemonClient {
    pub(super) fn new(url: &str, token: Option<String>) -> Result<Self> {
        Ok(Self {
            endpoint: DaemonTransport::parse(url)?,
            token,
        })
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request_json("GET", path, None)
    }

    fn post_json<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T> {
        self.request_json("POST", path, Some(body))
    }

    fn delete_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request_json("DELETE", path, None)
    }

    pub(super) fn request_json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let body_bytes = match body {
            Some(value) => serde_json::to_vec(value)?,
            None => Vec::new(),
        };
        let request_path = self.endpoint.request_path(path);
        let mut request = format!(
            "{method} {request_path} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.endpoint.authority(),
            body_bytes.len()
        );
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        if let Some(token) = &self.token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        let metrics_file =
            std::env::var_os(PERFORMANCE_METRICS_FILE_ENV).filter(|path| !path.is_empty());
        if metrics_file.is_some() {
            request.push_str("X-Trail-Operation-Metrics: 1\r\n");
        }
        request.push_str("\r\n");

        let response = self.endpoint.exchange(
            request.as_bytes(),
            &body_bytes,
            daemon_request_timeout(method, path),
        )?;
        let (status, operation_metrics, response_body) = parse_http_response(&response)?;
        if let (Some(path), Some(report)) = (metrics_file.as_deref(), operation_metrics) {
            emit_daemon_operation_metrics_report(Path::new(path), report)?;
        }
        if !(200..300).contains(&status) {
            return Err(error_from_daemon_response(status, response_body));
        }
        serde_json::from_slice(response_body).map_err(Error::from)
    }
}

fn daemon_request_timeout(method: &str, path: &str) -> Duration {
    if !matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
        || path == "/v1/record"
        || path == "/v1/ledger/challenge"
    {
        DAEMON_READ_REQUEST_TIMEOUT
    } else {
        DAEMON_MUTATING_REQUEST_TIMEOUT
    }
}

#[derive(Serialize)]
struct LedgerFenceRequest<'a> {
    protocol_version: u16,
    owner_nonce: &'a str,
    workspace_identity: &'a str,
    executable_identity: &'a str,
    scope_id: &'a str,
    expected_epoch: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct LedgerFenceProof {
    pub(super) protocol_version: u16,
    pub(super) pid: u32,
    pub(super) process_start_identity: String,
    pub(super) executable_identity: String,
    pub(super) owner_nonce: String,
    pub(super) workspace_identity: String,
    pub(super) live_fence_sequence: u64,
    pub(super) scope_id: String,
    pub(super) epoch: u64,
    pub(super) daemon_launch_nonce: String,
    pub(super) durable_offset: u64,
    pub(super) folded_offset: u64,
}

pub(super) fn authenticated_ledger_fence(
    endpoint: &daemon_start::WorkspaceDaemonEndpoint,
) -> Result<LedgerFenceProof> {
    let client = DaemonClient::new(&endpoint.url, Some(endpoint.auth_token.clone()))?;
    let body = serde_json::to_value(LedgerFenceRequest {
        protocol_version: endpoint.protocol_version,
        owner_nonce: &endpoint.owner_nonce,
        workspace_identity: &endpoint.workspace_identity,
        executable_identity: &endpoint.executable_identity,
        scope_id: &endpoint.scope_id,
        expected_epoch: endpoint.epoch,
    })?;
    client.request_json("POST", "/v1/ledger/challenge", Some(&body))
}

enum DaemonTransport {
    Tcp(DaemonEndpoint),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl DaemonTransport {
    fn parse(url: &str) -> Result<Self> {
        #[cfg(unix)]
        if let Some(path) = url.strip_prefix("unix://") {
            if path.is_empty() {
                return Err(Error::InvalidInput(
                    "daemon Unix socket path is empty".into(),
                ));
            }
            return Ok(Self::Unix(PathBuf::from(path)));
        }
        Ok(Self::Tcp(DaemonEndpoint::parse(url)?))
    }

    fn authority(&self) -> &str {
        match self {
            Self::Tcp(endpoint) => &endpoint.authority,
            #[cfg(unix)]
            Self::Unix(_) => "localhost",
        }
    }

    fn request_path(&self, path: &str) -> String {
        match self {
            Self::Tcp(endpoint) => endpoint.request_path(path),
            #[cfg(unix)]
            Self::Unix(_) => path.to_string(),
        }
    }

    fn exchange(&self, request: &[u8], body: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        match self {
            Self::Tcp(endpoint) => {
                let mut stream =
                    TcpStream::connect((&*endpoint.host, endpoint.port)).map_err(|error| {
                        Error::DaemonUnavailable(format!(
                            "could not connect to {}: {error}",
                            endpoint.authority
                        ))
                    })?;
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                exchange_stream(&mut stream, request, body)
                    .map_err(|error| map_daemon_exchange_error(error, timeout))
            }
            #[cfg(unix)]
            Self::Unix(path) => {
                let mut stream = UnixStream::connect(path).map_err(|error| {
                    Error::DaemonUnavailable(format!(
                        "could not connect to workspace daemon socket {}: {error}",
                        path.display()
                    ))
                })?;
                stream.set_read_timeout(Some(timeout))?;
                stream.set_write_timeout(Some(timeout))?;
                exchange_stream(&mut stream, request, body)
                    .map_err(|error| map_daemon_exchange_error(error, timeout))
            }
        }
    }
}

fn map_daemon_exchange_error(error: Error, timeout: Duration) -> Error {
    match error {
        Error::Io(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
            Error::DaemonUnavailable(format!(
                "workspace daemon response timed out after {} seconds",
                timeout.as_secs()
            ))
        }
        error => error,
    }
}

fn exchange_stream<S: Read + Write>(
    stream: &mut S,
    request: &[u8],
    body: &[u8],
) -> Result<Vec<u8>> {
    stream.write_all(request)?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    Ok(response)
}

struct DaemonEndpoint {
    host: String,
    port: u16,
    authority: String,
    base_path: String,
}

impl DaemonEndpoint {
    fn parse(url: &str) -> Result<Self> {
        let trimmed = url.trim().trim_end_matches('/');
        let rest = trimmed.strip_prefix("http://").ok_or_else(|| {
            Error::InvalidInput(
                "--daemon-url currently supports local http:// URLs only".to_string(),
            )
        })?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            return Err(Error::InvalidInput(
                "--daemon-url must include a host".to_string(),
            ));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => {
                let port = port.parse::<u16>().map_err(|_| {
                    Error::InvalidInput(format!("invalid daemon URL port `{port}`"))
                })?;
                (host.trim_matches(['[', ']']).to_string(), port)
            }
            None => (authority.to_string(), 80),
            Some(_) => {
                return Err(Error::InvalidInput(
                    "--daemon-url must include a non-empty host".to_string(),
                ));
            }
        };
        let base_path = if path.is_empty() {
            String::new()
        } else {
            format!("/{}", path.trim_end_matches('/'))
        };
        Ok(Self {
            host,
            port,
            authority: authority.to_string(),
            base_path,
        })
    }

    fn request_path(&self, path: &str) -> String {
        if self.base_path.is_empty() {
            path.to_string()
        } else {
            format!("{}{}", self.base_path, path)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct DaemonEndpointFile {
    pub(super) version: u32,
    pub(super) url: String,
    pub(super) pid: u32,
    pub(super) auth: bool,
}

pub(super) fn daemon_endpoint_path(db_dir: &Path) -> PathBuf {
    db_dir.join("daemon.json")
}

pub(super) fn daemon_url_for_listener(local_addr: SocketAddr) -> String {
    let host = match local_addr.ip() {
        IpAddr::V4(addr) if addr.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V4(addr) => addr.to_string(),
        IpAddr::V6(addr) if addr.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V6(addr) => format!("[{addr}]"),
    };
    format!("http://{host}:{}", local_addr.port())
}

fn discover_daemon_url(ctx: &RuntimeContext) -> Result<Option<String>> {
    let Some(db_dir) = discover_db_dir(ctx) else {
        return Ok(None);
    };
    let endpoint_path = daemon_endpoint_path(&db_dir);
    let bytes = match std::fs::read(endpoint_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::from(err)),
    };
    let endpoint = match serde_json::from_slice::<DaemonEndpointFile>(&bytes) {
        Ok(endpoint) if endpoint.version == 1 => endpoint,
        _ => return Ok(None),
    };
    if DaemonEndpoint::parse(&endpoint.url).is_err() {
        return Ok(None);
    }
    Ok(Some(endpoint.url))
}

fn parse_http_response(response: &[u8]) -> Result<(u16, Option<&str>, &[u8])> {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err(Error::DaemonUnavailable(
            "daemon returned a malformed HTTP response".to_string(),
        ));
    };
    let header = std::str::from_utf8(&response[..header_end]).map_err(|err| {
        Error::DaemonUnavailable(format!("daemon returned non-UTF-8 HTTP headers: {err}"))
    })?;
    let status_line = header.lines().next().ok_or_else(|| {
        Error::DaemonUnavailable("daemon returned an empty HTTP response".to_string())
    })?;
    let mut parts = status_line.split_whitespace();
    let _http = parts.next();
    let status = parts
        .next()
        .ok_or_else(|| Error::DaemonUnavailable("daemon response missing HTTP status".to_string()))?
        .parse::<u16>()
        .map_err(|_| {
            Error::DaemonUnavailable(format!(
                "daemon response has invalid status `{status_line}`"
            ))
        })?;
    let mut operation_metrics = None;
    for line in header.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(Error::DaemonUnavailable(
                "daemon response has a malformed HTTP header".into(),
            ));
        };
        if name.eq_ignore_ascii_case(OPERATION_METRICS_HEADER) {
            if operation_metrics.is_some() {
                return Err(Error::DaemonUnavailable(
                    "daemon response repeated its operation metrics report".into(),
                ));
            }
            operation_metrics = Some(value.trim());
        }
    }
    Ok((status, operation_metrics, &response[header_end + 4..]))
}

fn emit_daemon_operation_metrics_report(path: &Path, report: &str) -> Result<()> {
    let report = serde_json::from_str::<Value>(report).map_err(|error| {
        Error::DaemonUnavailable(format!(
            "daemon returned malformed operation metrics JSON: {error}"
        ))
    })?;
    if !report.is_object() {
        return Err(Error::DaemonUnavailable(
            "daemon returned a non-object operation metrics report".into(),
        ));
    }
    let mut line = serde_json::to_vec(&report)?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let written = file.write(&line)?;
    if written != line.len() {
        return Err(Error::Io(std::io::Error::new(
            ErrorKind::WriteZero,
            format!(
                "short performance metrics append: wrote {written} of {} bytes",
                line.len()
            ),
        )));
    }
    file.flush()?;
    Ok(())
}

fn error_from_daemon_response(status: u16, body: &[u8]) -> Error {
    if let Ok(error) = serde_json::from_slice::<DaemonErrorBody>(body) {
        if status == 401 {
            return Error::DaemonUnavailable(error.error.message);
        }
        if error.error.code.as_ref().and_then(DaemonErrorCode::as_text)
            == Some("CHANGE_LEDGER_RECONCILE_REQUIRED")
        {
            return Error::ChangeLedgerReconcileRequired {
                scope: error.error.scope.unwrap_or_default(),
                state: error.error.state.unwrap_or_default(),
                reason: error
                    .error
                    .reason
                    .unwrap_or_else(|| error.error.message.clone()),
                command: error
                    .error
                    .recovery
                    .map(|recovery| recovery.command)
                    .unwrap_or_else(|| "trail index reconcile".to_string()),
            };
        }
        let numeric_code = error
            .error
            .code
            .as_ref()
            .and_then(DaemonErrorCode::as_numeric);
        return Error::DaemonError {
            message: error.error.message,
            exit_code: error
                .error
                .exit
                .or(error.error.exit_code)
                .or(numeric_code)
                .unwrap_or(1),
        };
    }
    Error::DaemonError {
        message: format!("daemon returned HTTP {status}"),
        exit_code: if status == 401 { 11 } else { 1 },
    }
}

#[derive(Deserialize)]
struct DaemonErrorBody {
    error: DaemonErrorDetails,
}

#[derive(Deserialize)]
struct DaemonErrorDetails {
    message: String,
    #[serde(default)]
    code: Option<DaemonErrorCode>,
    #[serde(default)]
    exit: Option<i32>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    recovery: Option<DaemonErrorRecovery>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DaemonErrorCode {
    Text(String),
    Numeric(i32),
}

impl DaemonErrorCode {
    fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Numeric(_) => None,
        }
    }

    fn as_numeric(&self) -> Option<i32> {
        match self {
            Self::Text(_) => None,
            Self::Numeric(value) => Some(*value),
        }
    }
}

#[derive(Deserialize)]
struct DaemonErrorRecovery {
    command: String,
}

fn resolve_daemon_token(ctx: &RuntimeContext, explicit: Option<String>) -> Result<Option<String>> {
    if let Some(token) = explicit {
        return Ok(Some(token));
    }
    let Some(db_dir) = discover_db_dir(ctx) else {
        return Ok(None);
    };
    let token_path = db_dir.join("daemon.token");
    if !token_path.exists() {
        return Ok(None);
    }
    let token = std::fs::read_to_string(&token_path)?.trim().to_string();
    if token.is_empty() {
        return Ok(None);
    }
    Ok(Some(token))
}

fn discover_db_dir(ctx: &RuntimeContext) -> Option<PathBuf> {
    if let Some(db_dir) = &ctx.db_dir {
        return Some(db_dir.clone());
    }
    let start = ctx
        .workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())?;
    let mut dir = start;
    loop {
        let candidate = dir.join(".trail");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_response_carries_one_request_scoped_metrics_report() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Trail-Operation-Metrics: {\"generation\":7,\"operation\":\"status\"}\r\nContent-Length: 2\r\n\r\n{}";
        let (status, metrics, body) = parse_http_response(response).unwrap();
        assert_eq!(status, 200);
        assert_eq!(metrics, Some("{\"generation\":7,\"operation\":\"status\"}"));
        assert_eq!(body, b"{}");

        let duplicate = b"HTTP/1.1 200 OK\r\nX-Trail-Operation-Metrics: {}\r\nx-trail-operation-metrics: {}\r\n\r\n{}";
        assert!(matches!(
            parse_http_response(duplicate),
            Err(Error::DaemonUnavailable(message))
                if message.contains("repeated")
        ));
    }

    #[test]
    fn daemon_timeout_allows_large_mutations_without_extending_read_requests_indefinitely() {
        assert_eq!(
            daemon_request_timeout("GET", "/v1/status"),
            Duration::from_secs(120)
        );
        assert_eq!(
            daemon_request_timeout("GET", "/v1/diff?dirty=1"),
            Duration::from_secs(120)
        );
        assert_eq!(
            daemon_request_timeout("POST", "/v1/record"),
            Duration::from_secs(120)
        );
        assert_eq!(
            daemon_request_timeout("POST", "/v1/lanes"),
            Duration::from_secs(15 * 60)
        );
        assert_eq!(
            daemon_request_timeout("POST", "/v1/index/reconcile"),
            Duration::from_secs(15 * 60)
        );
    }

    #[test]
    fn daemon_metrics_emission_appends_exactly_one_json_line() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("metrics.jsonl");
        emit_daemon_operation_metrics_report(
            &path,
            "{\"generation\":9,\"operation\":\"structured_patch\"}",
        )
        .unwrap();
        let lines = std::fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 1);
        let value: Value = serde_json::from_str(lines.trim()).unwrap();
        assert_eq!(value["generation"], 9);
        assert_eq!(value["operation"], "structured_patch");
    }

    #[test]
    fn daemon_error_parser_accepts_legacy_numeric_transport_codes() {
        let error = error_from_daemon_response(
            429,
            br#"{"error":{"message":"rate limit exceeded","code":2}}"#,
        );
        match error {
            Error::DaemonError { message, exit_code } => {
                assert_eq!(message, "rate limit exceeded");
                assert_eq!(exit_code, 2);
            }
            error => panic!("unexpected daemon error: {error}"),
        }
    }

    #[test]
    fn daemon_error_parser_preserves_structured_lane_recovery() {
        let error = error_from_daemon_response(
            409,
            br#"{"error":{"code":"CHANGE_LEDGER_RECONCILE_REQUIRED","status":409,"exit":16,"message":"reconcile","scope":"lane-scope","state":"untrusted_gap","reason":"overflow","recovery":{"command":"trail index reconcile --lane reconcile-bot"}}}"#,
        );
        match error {
            Error::ChangeLedgerReconcileRequired { command, .. } => {
                assert_eq!(command, "trail index reconcile --lane reconcile-bot");
            }
            error => panic!("unexpected daemon error: {error}"),
        }
    }
}
