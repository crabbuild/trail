use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::protocol::{CorrelationKey, Direction, EnvelopeKind, Frame, RequestId};
use super::schema::AcpV1Contract;
use super::AcpRelayOptions;
use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PathMapping {
    pub original: PathBuf,
    pub effective: PathBuf,
    pub isolated: bool,
}

pub(crate) struct WorkspaceMapper {
    workspace_root: PathBuf,
    materialized_root: PathBuf,
}

impl WorkspaceMapper {
    pub(crate) fn new(workspace_root: PathBuf, materialized_root: PathBuf) -> Result<Self> {
        let workspace_root = resolve_existing_ancestors(&workspace_root)?;
        let materialized_root = resolve_existing_ancestors(&materialized_root)?;
        Ok(Self {
            workspace_root,
            materialized_root,
        })
    }

    pub(crate) fn map(&self, path: &Path) -> Result<PathMapping> {
        let original = path.to_path_buf();
        if looks_like_foreign_absolute_path(path) {
            return Ok(PathMapping {
                effective: original.clone(),
                original,
                isolated: false,
            });
        }
        if !path.is_absolute() {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "ACP workspace paths must be absolute".to_string(),
            });
        }
        let resolved = resolve_existing_ancestors(path)?;
        let Ok(relative) = resolved.strip_prefix(&self.workspace_root) else {
            return Ok(PathMapping {
                effective: original.clone(),
                original,
                isolated: false,
            });
        };
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "ACP workspace mapping escapes its workspace root".to_string(),
            });
        }
        let effective = if relative.as_os_str().is_empty() {
            self.materialized_root.clone()
        } else {
            self.materialized_root.join(relative)
        };
        let resolved_effective = resolve_existing_ancestors(&effective)?;
        if !resolved_effective.starts_with(&self.materialized_root) {
            return Err(Error::InvalidPath {
                path: effective.to_string_lossy().to_string(),
                reason: "ACP workspace mapping escapes its materialized root".to_string(),
            });
        }
        fs::create_dir_all(&effective)?;
        Ok(PathMapping {
            original,
            effective,
            isolated: true,
        })
    }

    pub(crate) fn map_session_params(
        &self,
        params: &mut Map<String, Value>,
    ) -> Result<Vec<PathMapping>> {
        let mut mappings = Vec::new();
        let mut effective_roots = HashSet::new();
        if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
            let mapping = self.map(Path::new(cwd))?;
            params.insert(
                "cwd".to_string(),
                Value::String(mapping.effective.to_string_lossy().to_string()),
            );
            effective_roots.insert(mapping.effective.clone());
            mappings.push(mapping);
        }

        if let Some(additional) = params.get_mut("additionalDirectories") {
            let directories = additional.as_array_mut().ok_or_else(|| {
                Error::InvalidInput(
                    "ACP session additionalDirectories must be an array".to_string(),
                )
            })?;
            let mut forwarded = Vec::with_capacity(directories.len());
            for directory in directories.iter() {
                let path = directory.as_str().ok_or_else(|| {
                    Error::InvalidInput(
                        "ACP session additionalDirectories entries must be strings".to_string(),
                    )
                })?;
                let mapping = self.map(Path::new(path))?;
                if effective_roots.insert(mapping.effective.clone()) {
                    forwarded.push(Value::String(
                        mapping.effective.to_string_lossy().to_string(),
                    ));
                    mappings.push(mapping);
                }
            }
            *directories = forwarded;
        }
        Ok(mappings)
    }
}

pub(crate) fn passthrough_session_mappings(
    params: &Map<String, Value>,
) -> Result<Vec<PathMapping>> {
    let mut mappings = Vec::new();
    let mut roots = HashSet::new();
    if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
        let path = PathBuf::from(cwd);
        roots.insert(path.clone());
        mappings.push(PathMapping {
            original: path.clone(),
            effective: path,
            isolated: false,
        });
    }
    if let Some(additional) = params.get("additionalDirectories") {
        let directories = additional.as_array().ok_or_else(|| {
            Error::InvalidInput("ACP session additionalDirectories must be an array".to_string())
        })?;
        for directory in directories {
            let path = directory.as_str().ok_or_else(|| {
                Error::InvalidInput(
                    "ACP session additionalDirectories entries must be strings".to_string(),
                )
            })?;
            let path = PathBuf::from(path);
            if roots.insert(path.clone()) {
                mappings.push(PathMapping {
                    original: path.clone(),
                    effective: path,
                    isolated: false,
                });
            }
        }
    }
    Ok(mappings)
}

fn resolve_existing_ancestors(path: &Path) -> Result<PathBuf> {
    let normalized = normalize_absolute(path)?;
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(canonical) => {
                return Ok(suffix
                    .iter()
                    .rev()
                    .fold(canonical, |resolved, component| resolved.join(component)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = ancestor.file_name().ok_or_else(|| Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "ACP path has no existing ancestor".to_string(),
                })?;
                suffix.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or_else(|| Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "ACP path has no existing ancestor".to_string(),
                })?;
            }
            Err(error) => return Err(Error::Io(error)),
        }
    }
}

fn normalize_absolute(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(Error::InvalidPath {
                        path: path.to_string_lossy().to_string(),
                        reason: "ACP path escapes its filesystem root".to_string(),
                    });
                }
            }
        }
    }
    Ok(normalized)
}

fn looks_like_foreign_absolute_path(path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }
    let value = path.to_string_lossy().as_bytes().to_vec();
    value.starts_with(b"\\\\")
        || value.starts_with(b"//")
        || (value.len() >= 3
            && value[0].is_ascii_alphabetic()
            && value[1] == b':'
            && matches!(value[2], b'\\' | b'/'))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NegotiationState {
    AwaitingInitialize,
    InitializePending,
    V1,
    Other(u16),
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) struct TransformOptions {
    workspace: String,
    db_dir: String,
    provider: Option<String>,
    model: Option<String>,
}

impl TransformOptions {
    pub(crate) fn from_relay(options: &AcpRelayOptions) -> Self {
        Self {
            workspace: options.workspace_root.to_string_lossy().to_string(),
            db_dir: options.db_dir.to_string_lossy().to_string(),
            provider: options.provider.clone(),
            model: options.model.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformOutcome {
    capture_v1: bool,
    diagnostic: Option<String>,
}

impl TransformOutcome {
    fn passthrough(capture_v1: bool) -> Self {
        Self {
            capture_v1,
            diagnostic: None,
        }
    }

    fn diagnostic(capture_v1: bool, diagnostic: impl Into<String>) -> Self {
        Self {
            capture_v1,
            diagnostic: Some(diagnostic.into()),
        }
    }

    pub(crate) fn capture_v1(&self) -> bool {
        self.capture_v1
    }

    pub(crate) fn diagnostic_message(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

pub(crate) struct TransformPipeline {
    state: NegotiationState,
    initialize_id: Option<RequestId>,
    contract: Arc<AcpV1Contract>,
    options: TransformOptions,
    client_capabilities: Option<Value>,
    agent_capabilities: Option<Value>,
}

impl TransformPipeline {
    pub(crate) fn new(contract: Arc<AcpV1Contract>, options: TransformOptions) -> Self {
        Self {
            state: NegotiationState::AwaitingInitialize,
            initialize_id: None,
            contract,
            options,
            client_capabilities: None,
            agent_capabilities: None,
        }
    }

    pub(crate) fn apply(&mut self, frame: &mut Frame) -> Result<TransformOutcome> {
        if frame.direction() == Direction::ClientToAgent && frame.method() == Some("initialize") {
            return self.observe_initialize_request(frame);
        }

        if self.state == NegotiationState::InitializePending
            && frame.direction() == Direction::AgentToClient
            && self.matches_initialize_response(frame)
        {
            return self.observe_initialize_response(frame);
        }

        match self.state {
            NegotiationState::V1 => Ok(TransformOutcome::passthrough(true)),
            NegotiationState::AwaitingInitialize => Ok(TransformOutcome::diagnostic(
                false,
                "ACP message arrived before initialize negotiation",
            )),
            NegotiationState::InitializePending => Ok(TransformOutcome::passthrough(false)),
            NegotiationState::Other(version) => Ok(TransformOutcome::diagnostic(
                false,
                format!("ACP version {version} is outside Trail's v1 compatibility layer"),
            )),
            NegotiationState::Failed => Ok(TransformOutcome::diagnostic(
                false,
                "ACP initialize negotiation failed",
            )),
        }
    }

    pub(crate) fn commit_candidate(&self, frame: &mut Frame, candidate: Value) -> Result<()> {
        if self.state != NegotiationState::V1 || candidate == *frame.value() {
            return Ok(());
        }
        self.contract.validate(&candidate)?;
        frame.replace_value_and_commit(candidate).map_err(Error::Io)
    }

    #[allow(dead_code)]
    pub(crate) fn state(&self) -> NegotiationState {
        self.state
    }

    fn observe_initialize_request(&mut self, frame: &Frame) -> Result<TransformOutcome> {
        if self.state != NegotiationState::AwaitingInitialize {
            return Ok(TransformOutcome::diagnostic(
                false,
                "duplicate ACP initialize request was forwarded without renegotiating",
            ));
        }
        if frame.kind() != EnvelopeKind::Request {
            self.state = NegotiationState::Failed;
            return Ok(TransformOutcome::diagnostic(
                false,
                "ACP initialize must be a request with an id",
            ));
        }
        let Some(CorrelationKey {
            direction: Direction::ClientToAgent,
            id,
        }) = frame.correlation_key()
        else {
            self.state = NegotiationState::Failed;
            return Ok(TransformOutcome::diagnostic(
                false,
                "ACP initialize request could not be correlated",
            ));
        };
        self.initialize_id = Some(id);
        self.client_capabilities = frame.value().pointer("/params/clientCapabilities").cloned();
        self.state = NegotiationState::InitializePending;
        Ok(TransformOutcome::passthrough(false))
    }

    fn matches_initialize_response(&self, frame: &Frame) -> bool {
        frame.correlation_key().is_some_and(|key| {
            key.direction == Direction::ClientToAgent
                && self.initialize_id.as_ref() == Some(&key.id)
        })
    }

    fn observe_initialize_response(&mut self, frame: &mut Frame) -> Result<TransformOutcome> {
        if frame.kind() == EnvelopeKind::ErrorResponse {
            self.state = NegotiationState::Failed;
            return Ok(TransformOutcome::diagnostic(
                false,
                "ACP initialize returned an error",
            ));
        }
        let Some(version) = frame
            .value()
            .pointer("/result/protocolVersion")
            .and_then(Value::as_u64)
            .and_then(|version| u16::try_from(version).ok())
        else {
            self.state = NegotiationState::Failed;
            return Ok(TransformOutcome::diagnostic(
                false,
                "ACP initialize response omitted a valid protocol version",
            ));
        };
        self.agent_capabilities = frame.value().pointer("/result/agentCapabilities").cloned();
        if version != self.contract.wire_version() {
            self.state = NegotiationState::Other(version);
            return Ok(TransformOutcome::diagnostic(
                false,
                format!("upstream selected ACP version {version}; Trail supports v1"),
            ));
        }

        self.state = NegotiationState::V1;
        let mut candidate = frame.value().clone();
        if let Err(error) = self.add_trail_metadata(&mut candidate) {
            return Ok(TransformOutcome::diagnostic(true, error.to_string()));
        }
        if let Err(error) = self.contract.validate(&candidate) {
            return Ok(TransformOutcome::diagnostic(
                true,
                format!("ACP initialize metadata transformation rolled back: {error}"),
            ));
        }
        if let Err(error) = frame.replace_value_and_commit(candidate) {
            return Ok(TransformOutcome::diagnostic(
                true,
                format!("ACP initialize metadata transformation rolled back: {error}"),
            ));
        }
        Ok(TransformOutcome::passthrough(true))
    }

    fn add_trail_metadata(&self, message: &mut Value) -> Result<()> {
        let result = message
            .get_mut("result")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                Error::InvalidInput("ACP initialize response result must be an object".to_string())
            })?;
        let meta = match result.entry("_meta".to_string()) {
            serde_json::map::Entry::Vacant(entry) => entry
                .insert(Value::Object(Map::new()))
                .as_object_mut()
                .unwrap(),
            serde_json::map::Entry::Occupied(mut entry) if entry.get().is_null() => {
                entry.insert(Value::Object(Map::new()));
                entry.into_mut().as_object_mut().unwrap()
            }
            serde_json::map::Entry::Occupied(entry) => {
                entry.into_mut().as_object_mut().ok_or_else(|| {
                    Error::InvalidInput(
                        "ACP initialize response _meta is not an object".to_string(),
                    )
                })?
            }
        };
        meta.insert(
            "trail".to_string(),
            serde_json::json!({
                "relay": true,
                "capture": true,
                "workspace": self.options.workspace,
                "dbDir": self.options.db_dir,
                "provider": self.options.provider,
                "model": self.options.model
            }),
        );
        Ok(())
    }
}
