use clap::error::ErrorKind as ClapErrorKind;

use super::*;

const CLI_PARSE_ERROR_FALLBACK: &str = r#"{"error":{"code":"INVALID_INPUT","status":400,"exit":2,"message":"invalid CLI input","scope":null,"state":null,"reason":null,"recovery":{"command":"trail --help"}}}"#;
const STRUCTURED_ERROR_FALLBACK: &str = r#"{"error":{"code":"SERIALIZATION_ERROR","status":500,"exit":1,"message":"failed to serialize structured error","scope":null,"state":null,"reason":null,"recovery":null}}"#;

pub(super) fn args_request_json_errors<I>(args: I) -> bool
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut expect_format = false;
    for arg in args {
        let arg = arg.to_string_lossy();
        if expect_format {
            if arg == "json" || arg == "ndjson" {
                return true;
            }
            expect_format = false;
            continue;
        }
        if arg == "--json" || arg == "--format=json" || arg == "--format=ndjson" {
            return true;
        }
        if arg == "--format" {
            expect_format = true;
        }
    }
    false
}

pub(super) fn env_requests_json_errors() -> bool {
    std::env::var("TRAIL_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json") || value.eq_ignore_ascii_case("ndjson"))
        .unwrap_or(false)
}

pub(super) fn handle_cli_parse_error(err: clap::Error, json: bool) -> ! {
    match err.kind() {
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => err.exit(),
        _ if json => {
            let exit_code = err.exit_code();
            render_cli_parse_error(&err, exit_code);
            std::process::exit(exit_code);
        }
        _ => {
            let mut diagnostic = UiDiagnostic::new("INVALID_INPUT", "Invalid Trail command");
            diagnostic.cause = Some(err.to_string());
            diagnostic.recovery = Some(UiNextAction {
                command: "trail --help".to_string(),
                reason: "Show the available commands and global options.".to_string(),
            });
            let document = TerminalDocument::empty().block(UiBlock::Diagnostic(diagnostic));
            let _ = render_error_document(&document, &default_error_options());
            std::process::exit(err.exit_code());
        }
    }
}

fn render_cli_parse_error(err: &clap::Error, exit_code: i32) {
    let message = err.to_string();
    let value = serde_json::json!({
        "error": {
            "code": "INVALID_INPUT",
            "status": 400,
            "exit": exit_code,
            "message": message.trim(),
            "scope": null,
            "state": null,
            "reason": null,
            "recovery": { "command": "trail --help" }
        }
    });
    let rendered =
        serde_json::to_string(&value).unwrap_or_else(|_| CLI_PARSE_ERROR_FALLBACK.to_string());
    let _ = render_structured_error(&rendered);
}

pub(super) fn render_error(err: &Error, json: bool) {
    if json {
        let value = StructuredErrorEnvelope::from_error(err);
        let rendered = serde_json::to_string(&value)
            .unwrap_or_else(|_| structured_error_fallback(err).to_string());
        let _ = render_structured_error(&rendered);
        return;
    }
    let document = TerminalDocument::empty().block(UiBlock::Diagnostic(diagnostic_for_error(err)));
    let _ = render_error_document(&document, &default_error_options());
}

fn structured_error_fallback(_err: &Error) -> &'static str {
    STRUCTURED_ERROR_FALLBACK
}

fn default_error_options() -> RenderOptions {
    RenderOptions::from_environment(
        RenderMode::Human,
        ColorPolicy::Auto,
        PagerPolicy::Never,
        false,
        false,
    )
}

fn diagnostic_for_error(err: &Error) -> UiDiagnostic {
    let mut diagnostic = match err {
        Error::WorkspaceNotFound(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Trail workspace not found");
            diagnostic.consequence = Some(
                "Trail cannot inspect or change work because this directory has no .trail workspace."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail init --from-git".to_string(),
                reason: "Initialize Trail from the Git-tracked files in this repository."
                    .to_string(),
            });
            diagnostic
        }
        Error::WorkspaceExists(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Trail workspace already exists");
            diagnostic.consequence =
                Some("Trail did not replace the existing workspace state.".to_string());
            diagnostic.recovery = Some(UiNextAction {
                command: "trail status".to_string(),
                reason: "Inspect the existing workspace before deciding whether to reuse it."
                    .to_string(),
            });
            diagnostic
        }
        Error::DirtyWorktree | Error::DirtyWorktreeWithMessage(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Worktree has unrecorded changes");
            diagnostic.consequence = Some(
                "Trail stopped the operation to protect files that could be overwritten."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail status".to_string(),
                reason: "Inspect the affected paths before recording, discarding, or moving them."
                    .to_string(),
            });
            diagnostic.alternatives.push(UiNextAction {
                command: "trail record -m \"save current work\"".to_string(),
                reason: "Record the worktree changes as a Trail operation.".to_string(),
            });
            diagnostic
        }
        Error::Conflict(_) | Error::PatchRejected(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Patch or merge conflict requires resolution");
            diagnostic.recovery = Some(UiNextAction {
                command: "trail conflicts".to_string(),
                reason: "Inspect the conflict set and its recommended safe resolution.".to_string(),
            });
            diagnostic
        }
        Error::WorkspaceLocked(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Workspace is locked by another writer");
            diagnostic.consequence = Some(
                "Trail will not risk concurrent writes to the same workspace state.".to_string(),
            );
            diagnostic
        }
        Error::IgnoredPath(path) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Path is protected by Trail ignore rules");
            diagnostic.recovery = Some(UiNextAction {
                command: format!("trail ignore check {path}"),
                reason: "Show the ignore rule that protects this path.".to_string(),
            });
            diagnostic
        }
        Error::PathIndexRequired(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail workspace upgrade is required");
            diagnostic.consequence = Some(
                "Trail blocked this mutation until every live branch and lane head has a persistent path-invariant index."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail index rebuild".to_string(),
                reason: "Upgrade legacy live roots and retry the mutation.".to_string(),
            });
            diagnostic
        }
        Error::SchemaReinitializeRequired { .. } => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail workspace schema must be reinitialized");
            diagnostic.consequence = Some(
                "Trail rejected the workspace before opening any mutable storage.".to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail init --force".to_string(),
                reason: "Back up the workspace, then create the required schema v19.".to_string(),
            });
            diagnostic
        }
        Error::ChangeLedgerReconcileRequired { command, .. } => {
            let mut diagnostic = UiDiagnostic::new(
                err.code(),
                "Changed-path ledger trust could not be established",
            );
            diagnostic.recovery = Some(UiNextAction {
                command: command.clone(),
                reason: "Retry the operation that establishes a trusted changed-path snapshot."
                    .to_string(),
            });
            diagnostic
        }
        Error::CommittedRepairRequired { .. } => {
            let mut diagnostic = UiDiagnostic::new(
                err.code(),
                "Operation committed; a derived mirror still needs repair",
            );
            diagnostic.consequence = Some(
                "Do not retry the mutation: its authoritative database transaction already committed."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail status".to_string(),
                reason: "Reopen authoritative state and idempotently repair ref, marker, and runtime mirrors."
                    .to_string(),
            });
            diagnostic
        }
        Error::InvalidInput(_) | Error::InvalidPath { .. } => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail cannot use the supplied input");
            diagnostic.recovery = Some(UiNextAction {
                command: "trail --help".to_string(),
                reason: "Review the command syntax and available options.".to_string(),
            });
            diagnostic
        }
        Error::CloneUnsupported | Error::CloneCrossDevice | Error::NativeCowSourceUnavailable => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Strict native COW is unavailable");
            diagnostic.consequence = Some(
                "Trail did not publish a partially cloned workdir or copy bytes for the strict request."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail lane spawn --workdir-mode portable-copy".to_string(),
                reason: "Use portable materialization with truthful clone/copy reporting."
                    .to_string(),
            });
            diagnostic
        }
        Error::RefNotFound(_)
        | Error::OperationNotFound(_)
        | Error::RootNotFound(_)
        | Error::ObjectNotFound { .. } => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail could not resolve the requested selector");
            diagnostic.recovery = Some(UiNextAction {
                command: "trail timeline --limit 20".to_string(),
                reason: "Inspect recent operations and copy an available selector.".to_string(),
            });
            diagnostic
        }
        Error::StaleBranch(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Branch changed before Trail could apply work");
            diagnostic.consequence = Some(
                "Trail did not apply work against a branch that may no longer match its review evidence."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail status".to_string(),
                reason: "Refresh branch state before reviewing or retrying the operation."
                    .to_string(),
            });
            diagnostic
        }
        Error::Corrupt(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail detected damaged workspace data");
            diagnostic.consequence =
                Some("Trail stopped to avoid making damaged state worse.".to_string());
            diagnostic.recovery = Some(UiNextAction {
                command: "trail fsck".to_string(),
                reason: "Inspect workspace integrity before attempting repair or restore."
                    .to_string(),
            });
            diagnostic
        }
        Error::Git(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Git interoperability command failed");
            diagnostic.recovery = Some(UiNextAction {
                command: "git status".to_string(),
                reason: "Check the Git worktree and repository state Trail needs to interoperate safely."
                    .to_string(),
            });
            diagnostic
        }
        Error::GitMappingRequired(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Git baseline mapping is required");
            diagnostic.recovery = Some(UiNextAction {
                command: "trail git import-update".to_string(),
                reason: "Import the current Git snapshot before retrying the handoff.".to_string(),
            });
            diagnostic
        }
        Error::GitHeadChanged(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Git HEAD changed during handoff");
            diagnostic.recovery = Some(UiNextAction {
                command: "git status --short".to_string(),
                reason: "Inspect the current Git branch and worktree before retrying.".to_string(),
            });
            diagnostic
        }
        Error::GitWorktreeDirty(_) => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Git tracked worktree has changes");
            diagnostic.recovery = Some(UiNextAction {
                command: "git status --short".to_string(),
                reason: "Inspect tracked Git changes before retrying the handoff.".to_string(),
            });
            diagnostic
        }
        Error::GitDeltaExportRequired(_) => {
            UiDiagnostic::new(err.code(), "Mapped Git delta export is required")
        }
        Error::DaemonUnavailable(_) | Error::DaemonError { .. } => {
            let mut diagnostic = UiDiagnostic::new(err.code(), "Trail daemon request failed");
            diagnostic.recovery = Some(UiNextAction {
                command: "trail doctor".to_string(),
                reason: "Check local workspace health before retrying the daemon-backed command."
                    .to_string(),
            });
            diagnostic
        }
        Error::Io(_)
        | Error::Sqlite(_)
        | Error::Serialization(_)
        | Error::Prolly(_)
        | Error::ProllySqlite(_)
        | Error::ProllySlateDb(_)
        | Error::Json(_)
        | Error::TomlSer(_)
        | Error::TomlDe(_) => {
            let mut diagnostic =
                UiDiagnostic::new(err.code(), "Trail could not read or write workspace data");
            diagnostic.consequence = Some(
                "No further action was taken after the failing storage or configuration operation."
                    .to_string(),
            );
            diagnostic.recovery = Some(UiNextAction {
                command: "trail doctor".to_string(),
                reason: "Check workspace storage and configuration before retrying.".to_string(),
            });
            diagnostic
        }
    };
    diagnostic.cause = Some(err.to_string());
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_structured_error_formats() {
        assert!(args_request_json_errors([std::ffi::OsString::from(
            "--format=ndjson"
        )]));
        assert!(args_request_json_errors([
            std::ffi::OsString::from("--format"),
            std::ffi::OsString::from("json"),
        ]));
    }

    #[test]
    fn cli_parse_error_fallback_is_valid_structured_json() {
        let value: serde_json::Value = serde_json::from_str(CLI_PARSE_ERROR_FALLBACK).unwrap();
        assert_eq!(value["error"]["code"], "INVALID_INPUT");
        assert_eq!(value["error"]["recovery"]["command"], "trail --help");
    }

    #[test]
    fn general_error_fallback_is_valid_json_with_hostile_error_text() {
        let hostile = Error::Serialization("quote: \" backslash: \\ newline:\n tab:\t".into());
        let rendered = structured_error_fallback(&hostile);
        let value: serde_json::Value = serde_json::from_str(rendered).unwrap();
        assert_eq!(value["error"]["code"], "SERIALIZATION_ERROR");
        assert_eq!(value["error"]["status"], 500);
        assert_eq!(value["error"]["exit"], 1);
        assert!(!rendered.contains("quote:"));
    }

    #[test]
    fn dirty_worktree_has_safe_primary_recovery() {
        let diagnostic = diagnostic_for_error(&Error::DirtyWorktree);
        assert_eq!(diagnostic.code, "DIRTY_WORKTREE");
        assert_eq!(diagnostic.recovery.unwrap().command, "trail status");
    }

    #[test]
    fn missing_git_mapping_recommends_explicit_reconciliation() {
        let diagnostic = diagnostic_for_error(&Error::GitMappingRequired("missing".into()));
        assert_eq!(diagnostic.code, "GIT_MAPPING_REQUIRED");
        assert_eq!(
            diagnostic.recovery.unwrap().command,
            "trail git import-update"
        );
    }

    #[test]
    fn path_index_required_recommends_workspace_upgrade() {
        let diagnostic = diagnostic_for_error(&Error::PathIndexRequired(
            "legacy root has no case-fold index".into(),
        ));
        assert_eq!(diagnostic.code, "PATH_INDEX_REQUIRED");
        assert_eq!(diagnostic.summary, "Trail workspace upgrade is required");
        assert!(diagnostic
            .consequence
            .as_deref()
            .is_some_and(|value| value.contains("mutation")));
        assert_eq!(diagnostic.recovery.unwrap().command, "trail index rebuild");
    }

    #[test]
    fn stable_error_categories_have_actionable_diagnostics() {
        let errors = [
            Error::WorkspaceExists(std::path::PathBuf::from("/workspace")),
            Error::PatchRejected("context changed".to_string()),
            Error::StaleBranch("main".to_string()),
            Error::Corrupt("bad object".to_string()),
            Error::Git("git failed".to_string()),
            Error::DaemonUnavailable("connection refused".to_string()),
        ];
        for error in errors {
            let diagnostic = diagnostic_for_error(&error);
            assert_eq!(diagnostic.code, error.code());
            assert!(diagnostic.cause.is_some());
            assert!(diagnostic.recovery.is_some());
        }
    }
}
