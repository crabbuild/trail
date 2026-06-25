use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crabdb::model::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::*;

pub(super) fn try_handle_auto_daemon_command(
    ctx: &RuntimeContext,
    daemon_token: Option<String>,
    command: &Command,
) -> Result<bool> {
    if !daemon_supports_command(command) {
        return Ok(false);
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
            render_status(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        Command::Diff(args) => {
            let path = diff_path(args)?;
            let summary: DiffSummary = client.get_json(&path)?;
            render_diff(&summary, ctx.json, ctx.quiet, args.stat)?;
            Ok(true)
        }
        Command::Agent(agent) => handle_agent_command(ctx, &client, agent),
        Command::MergeAgent(args) => {
            validate_merge_strategy(args.strategy.as_deref())?;
            let body = serde_json::json!({
                "agent_id": args.name,
                "strategy": args.strategy,
                "dry_run": args.dry_run,
            });
            let report: MergeReport =
                client.post_json(&format!("/v1/branches/{}/merge-agent", args.into), &body)?;
            render_merge(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        Command::MergeQueue(queue) => handle_merge_queue_command(ctx, &client, queue),
        _ => Ok(false),
    }
}

fn daemon_supports_command(command: &Command) -> bool {
    match command {
        Command::Status(args) => args.branch.is_none(),
        Command::Diff(_) | Command::MergeAgent(_) | Command::MergeQueue(_) => true,
        Command::Agent(agent) => matches!(
            agent.command,
            AgentSubcommand::Spawn(_)
                | AgentSubcommand::Status(_)
                | AgentSubcommand::Readiness(_)
                | AgentSubcommand::Read(_)
                | AgentSubcommand::SyncWorkdir(_)
                | AgentSubcommand::ApplyPatch(_)
                | AgentSubcommand::Diff(_)
        ),
        _ => false,
    }
}

fn auto_daemon_should_fallback(err: &Error) -> bool {
    matches!(err, Error::DaemonUnavailable(_))
}

fn handle_agent_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    agent: &AgentCommand,
) -> Result<bool> {
    match &agent.command {
        AgentSubcommand::Spawn(args) => {
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
            let report: AgentSpawnReport = client.post_json("/v1/agents", &Value::Object(body))?;
            render_agent_spawn(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::Status(args) => {
            let report: AgentStatusReport =
                client.get_json(&format!("/v1/agents/{}/status", args.name))?;
            render_agent_status(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::Readiness(args) => {
            let report: AgentReadinessReport =
                client.get_json(&format!("/v1/agents/{}/readiness", args.name))?;
            render_agent_readiness(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::SyncWorkdir(args) => {
            let body = serde_json::json!({
                "force": args.force,
                "paths": args.paths,
                "include_neighbors": args.include_neighbors,
            });
            let report: AgentWorkdirSyncReport =
                client.post_json(&format!("/v1/agents/{}/sync-workdir", args.name), &body)?;
            render_agent_workdir_sync(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::Read(args) => {
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
            let report: AgentFileReadReport = client.post_json(
                &format!("/v1/agents/{}/read-file", args.name),
                &Value::Object(body),
            )?;
            render_agent_file_read(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::ApplyPatch(args) => {
            let mut patch: PatchDocument =
                serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
            if args.allow_ignored {
                patch.allow_ignored = true;
            }
            let body = serde_json::to_value(&patch)?;
            let report: AgentPatchReport =
                client.post_json(&format!("/v1/agents/{}/patches", args.name), &body)?;
            render_agent_patch(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        AgentSubcommand::Diff(args) => {
            let mut params = Vec::new();
            if args.patch {
                params.push("patch=1".to_string());
            }
            if args.show_line_ids {
                params.push("show_line_ids=1".to_string());
            }
            let path = append_query(&format!("/v1/agents/{}/diff", args.name), params);
            let summary: DiffSummary = client.get_json(&path)?;
            render_diff(&summary, ctx.json, ctx.quiet, false)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_merge_queue_command(
    ctx: &RuntimeContext,
    client: &DaemonClient,
    queue: &MergeQueueCommand,
) -> Result<bool> {
    match &queue.command {
        MergeQueueSubcommand::Add(args) => {
            let body = serde_json::json!({
                "source": args.source,
                "target": args.into,
                "priority": args.priority,
            });
            let report: MergeQueueAddReport = client.post_json("/v1/merge-queue", &body)?;
            render_merge_queue_add(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        MergeQueueSubcommand::List => {
            let entries: Vec<MergeQueueEntry> = client.get_json("/v1/merge-queue")?;
            render_merge_queue_list(&entries, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        MergeQueueSubcommand::Run(args) => {
            let body = match args.limit {
                Some(limit) => serde_json::json!({ "limit": limit }),
                None => serde_json::json!({}),
            };
            let report: MergeQueueRunReport = client.post_json("/v1/merge-queue/run", &body)?;
            render_merge_queue_run(&report, ctx.json, ctx.quiet)?;
            Ok(true)
        }
        MergeQueueSubcommand::Remove(args) => {
            let report: MergeQueueRemoveReport =
                client.delete_json(&format!("/v1/merge-queue/{}", args.selector))?;
            render_merge_queue_remove(&report, ctx.json, ctx.quiet)?;
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

struct DaemonClient {
    endpoint: DaemonEndpoint,
    token: Option<String>,
}

impl DaemonClient {
    fn new(url: &str, token: Option<String>) -> Result<Self> {
        Ok(Self {
            endpoint: DaemonEndpoint::parse(url)?,
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

    fn request_json<T: DeserializeOwned>(
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
            self.endpoint.authority,
            body_bytes.len()
        );
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        if let Some(token) = &self.token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        request.push_str("\r\n");

        let mut stream =
            TcpStream::connect((&*self.endpoint.host, self.endpoint.port)).map_err(|err| {
                Error::DaemonUnavailable(format!(
                    "could not connect to {}: {err}",
                    self.endpoint.authority
                ))
            })?;
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(Error::from)?;
        stream.write_all(request.as_bytes())?;
        if !body_bytes.is_empty() {
            stream.write_all(&body_bytes)?;
        }
        stream.flush()?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let (status, response_body) = parse_http_response(&response)?;
        if !(200..300).contains(&status) {
            return Err(error_from_daemon_response(status, response_body));
        }
        serde_json::from_slice(response_body).map_err(Error::from)
    }
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
                ))
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

fn parse_http_response(response: &[u8]) -> Result<(u16, &[u8])> {
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
    Ok((status, &response[header_end + 4..]))
}

fn error_from_daemon_response(status: u16, body: &[u8]) -> Error {
    if let Ok(error) = serde_json::from_slice::<DaemonErrorBody>(body) {
        if status == 401 {
            return Error::DaemonUnavailable(error.error.message);
        }
        return Error::DaemonError {
            message: error.error.message,
            exit_code: error.error.code.unwrap_or(1),
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
    #[serde(default, alias = "exit_code")]
    code: Option<i32>,
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
        let candidate = dir.join(".crabdb");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
