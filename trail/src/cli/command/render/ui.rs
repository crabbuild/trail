use std::fmt::Write as _;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::{Command, Stdio};

use anstyle::{AnsiColor, Color, Style};
use terminal_size::{terminal_size, Height, Width};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use trail::model::{FileChangeKind, LineChangeKind, OperationKind, WorktreeState};
use trail::{Error, Result};
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenderMode {
    Human,
    Plain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ColorPolicy {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PagerPolicy {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GlyphSet {
    Unicode,
    Ascii,
}

#[derive(Clone, Debug)]
pub(crate) struct RenderOptions {
    pub(crate) mode: RenderMode,
    pub(crate) color: bool,
    pub(crate) glyphs: GlyphSet,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) stdout_is_terminal: bool,
    pub(crate) stderr_is_terminal: bool,
    pub(crate) verbose: bool,
    pub(crate) quiet: bool,
    pub(crate) pager: PagerPolicy,
}

impl RenderOptions {
    pub(crate) fn from_environment(
        mode: RenderMode,
        policy: ColorPolicy,
        pager: PagerPolicy,
        verbose: bool,
        quiet: bool,
    ) -> Self {
        let stdout_is_terminal = io::stdout().is_terminal();
        let stderr_is_terminal = io::stderr().is_terminal();
        let term_is_dumb = std::env::var("TERM")
            .map(|term| term.eq_ignore_ascii_case("dumb"))
            .unwrap_or(false);
        let color = resolve_color(
            mode,
            policy,
            stdout_is_terminal,
            term_is_dumb,
            std::env::var_os("NO_COLOR").is_some(),
        );
        let glyphs = if mode == RenderMode::Human && stdout_is_terminal && !term_is_dumb {
            GlyphSet::Unicode
        } else {
            GlyphSet::Ascii
        };
        let (width, height) = terminal_size()
            .map(|(Width(width), Height(height))| (usize::from(width), usize::from(height)))
            .unwrap_or((80, 24));
        Self {
            mode,
            color,
            glyphs,
            width: width.max(24),
            height: height.max(8),
            stdout_is_terminal,
            stderr_is_terminal,
            verbose,
            quiet,
            pager,
        }
    }

    #[cfg(test)]
    pub(crate) fn test(mode: RenderMode, width: usize) -> Self {
        Self {
            mode,
            color: false,
            glyphs: if mode == RenderMode::Human {
                GlyphSet::Unicode
            } else {
                GlyphSet::Ascii
            },
            width,
            height: 24,
            stdout_is_terminal: mode == RenderMode::Human,
            stderr_is_terminal: mode == RenderMode::Human,
            verbose: false,
            quiet: false,
            pager: PagerPolicy::Never,
        }
    }

    pub(crate) fn unicode(&self, unicode: &'static str, ascii: &'static str) -> &'static str {
        match self.glyphs {
            GlyphSet::Unicode => unicode,
            GlyphSet::Ascii => ascii,
        }
    }

    pub(crate) fn progress_allowed(&self) -> bool {
        self.mode == RenderMode::Human && self.stderr_is_terminal && !self.quiet
    }
}

fn resolve_color(
    mode: RenderMode,
    policy: ColorPolicy,
    stdout_is_terminal: bool,
    term_is_dumb: bool,
    no_color: bool,
) -> bool {
    match policy {
        ColorPolicy::Always => mode == RenderMode::Human,
        ColorPolicy::Never => false,
        ColorPolicy::Auto => {
            mode == RenderMode::Human && stdout_is_terminal && !term_is_dumb && !no_color
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiTone {
    Success,
    Attention,
    Blocked,
    Failure,
    Neutral,
    Info,
    Muted,
}

#[derive(Clone, Debug)]
pub(crate) struct TerminalDocument {
    pub(crate) lead: Option<UiLead>,
    pub(crate) context: Option<String>,
    pub(crate) blocks: Vec<UiBlock>,
    pub(crate) next: Option<UiNextAction>,
    pub(crate) more: Vec<UiNextAction>,
    pub(crate) pager_eligible: bool,
}

impl TerminalDocument {
    pub(crate) fn new(lead: impl Into<String>, tone: UiTone) -> Self {
        Self {
            lead: Some(UiLead {
                text: lead.into(),
                tone,
            }),
            context: None,
            blocks: Vec::new(),
            next: None,
            more: Vec::new(),
            pager_eligible: false,
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            lead: None,
            context: None,
            blocks: Vec::new(),
            next: None,
            more: Vec::new(),
            pager_eligible: false,
        }
    }

    pub(crate) fn context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub(crate) fn block(mut self, block: UiBlock) -> Self {
        self.blocks.push(block);
        self
    }

    pub(crate) fn next(mut self, command: impl Into<String>, reason: impl Into<String>) -> Self {
        self.next = Some(UiNextAction {
            command: command.into(),
            reason: reason.into(),
        });
        self
    }

    pub(crate) fn more(mut self, command: impl Into<String>, reason: impl Into<String>) -> Self {
        self.more.push(UiNextAction {
            command: command.into(),
            reason: reason.into(),
        });
        self
    }

    pub(crate) fn pager_eligible(mut self) -> Self {
        self.pager_eligible = true;
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiLead {
    pub(crate) text: String,
    pub(crate) tone: UiTone,
}

#[derive(Clone, Debug)]
pub(crate) struct UiNextAction {
    pub(crate) command: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug)]
pub(crate) enum UiBlock {
    Paragraph { text: String, tone: UiTone },
    Metadata(Vec<(String, String)>),
    Section { title: String, blocks: Vec<UiBlock> },
    Table(UiTable),
    Changes(UiChangeList),
    Checklist(Vec<UiCheck>),
    Diagnostic(UiDiagnostic),
    Notice(String),
    Lines(Vec<(String, UiTone)>),
    Patch { title: String, text: String },
}

impl UiBlock {
    pub(crate) fn paragraph(text: impl Into<String>) -> Self {
        Self::Paragraph {
            text: text.into(),
            tone: UiTone::Neutral,
        }
    }

    pub(crate) fn section(title: impl Into<String>, blocks: Vec<UiBlock>) -> Self {
        Self::Section {
            title: title.into(),
            blocks,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiTable {
    pub(crate) columns: Vec<UiColumn>,
    pub(crate) rows: Vec<Vec<String>>,
}

impl UiTable {
    pub(crate) fn new(columns: Vec<UiColumn>, rows: Vec<Vec<String>>) -> Self {
        Self { columns, rows }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiColumn {
    pub(crate) header: String,
    pub(crate) priority: u8,
    pub(crate) min_width: usize,
    pub(crate) align: UiAlign,
}

impl UiColumn {
    pub(crate) fn left(header: impl Into<String>, priority: u8, min_width: usize) -> Self {
        Self {
            header: header.into(),
            priority,
            min_width,
            align: UiAlign::Left,
        }
    }

    pub(crate) fn right(header: impl Into<String>, priority: u8, min_width: usize) -> Self {
        Self {
            header: header.into(),
            priority,
            min_width,
            align: UiAlign::Right,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiAlign {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub(crate) struct UiChangeList {
    pub(crate) changes: Vec<UiChange>,
    pub(crate) additions: u64,
    pub(crate) deletions: u64,
    pub(crate) omitted: Option<UiOmitted>,
}

impl UiChangeList {
    pub(crate) fn new(changes: Vec<UiChange>) -> Self {
        let additions = changes.iter().map(|change| change.additions).sum();
        let deletions = changes.iter().map(|change| change.deletions).sum();
        Self {
            changes,
            additions,
            deletions,
            omitted: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiChange {
    pub(crate) marker: char,
    pub(crate) path: String,
    pub(crate) old_path: Option<String>,
    pub(crate) additions: u64,
    pub(crate) deletions: u64,
    pub(crate) detail: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct UiOmitted {
    pub(crate) count: usize,
    pub(crate) command: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiCheckState {
    Pass,
    Warn,
    Fail,
    Blocked,
    Pending,
    Skip,
}

#[derive(Clone, Debug)]
pub(crate) struct UiCheck {
    pub(crate) state: UiCheckState,
    pub(crate) label: String,
    pub(crate) detail: String,
}

impl UiCheck {
    pub(crate) fn new(
        state: UiCheckState,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            state,
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// Creates a checklist entry from a stable domain status while keeping the
/// renderer's blocker-before-warning ordering shared across command families.
pub(crate) fn check_for_status(
    status: &str,
    label: impl Into<String>,
    detail: impl Into<String>,
) -> UiCheck {
    UiCheck::new(check_state_from_status(status), label, detail)
}

pub(crate) fn check_state_from_status(status: &str) -> UiCheckState {
    match status.to_ascii_lowercase().as_str() {
        "ok" | "allow" | "allowed" | "pass" | "passed" | "healthy" | "ready" | "info" => {
            UiCheckState::Pass
        }
        "warning" | "warn" | "stale" => UiCheckState::Warn,
        "pending" | "approval_required" | "waiting" => UiCheckState::Pending,
        "deny" | "denied" | "blocked" | "conflict" | "conflicted" | "error" => {
            UiCheckState::Blocked
        }
        "skip" | "skipped" | "ignored" => UiCheckState::Skip,
        _ => UiCheckState::Fail,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UiDiagnostic {
    pub(crate) code: String,
    pub(crate) summary: String,
    pub(crate) cause: Option<String>,
    pub(crate) consequence: Option<String>,
    pub(crate) recovery: Option<UiNextAction>,
    pub(crate) alternatives: Vec<UiNextAction>,
}

impl UiDiagnostic {
    pub(crate) fn new(code: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            summary: summary.into(),
            cause: None,
            consequence: None,
            recovery: None,
            alternatives: Vec::new(),
        }
    }
}

pub(crate) fn render_document(document: &TerminalDocument, options: &RenderOptions) -> Result<()> {
    if options.quiet {
        return Ok(());
    }
    if should_page(document, options) {
        let rendered = render_document_bytes(document, options)?;
        if should_send_to_pager(&rendered, options) {
            match page(&rendered) {
                Ok(()) => return Ok(()),
                Err(error) if options.verbose => {
                    let notice = TerminalDocument::empty().block(UiBlock::Notice(format!(
                        "Pager unavailable ({}); wrote review content directly.",
                        error
                    )));
                    let _ = render_error_document(&notice, options);
                }
                Err(_) => {}
            }
        }
        return write_stdout(&rendered);
    }
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    let result = Renderer::new(&mut writer, options).render(document);
    match result {
        Ok(()) => match writer.flush() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => Err(Error::from(error)),
        },
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::from(error)),
    }
}

fn render_document_bytes(document: &TerminalDocument, options: &RenderOptions) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    Renderer::new(&mut output, options)
        .render(document)
        .map_err(Error::from)?;
    Ok(output)
}

fn should_page(document: &TerminalDocument, options: &RenderOptions) -> bool {
    document.pager_eligible
        && options.mode == RenderMode::Human
        && options.stdout_is_terminal
        && !options.quiet
        && !matches!(options.pager, PagerPolicy::Never)
}

fn should_send_to_pager(output: &[u8], options: &RenderOptions) -> bool {
    const MAX_PAGED_BYTES: usize = 16 * 1024 * 1024;
    if output.len() > MAX_PAGED_BYTES {
        return false;
    }
    matches!(options.pager, PagerPolicy::Always)
        || output.iter().filter(|byte| **byte == b'\n').count() > options.height
}

fn page(output: &[u8]) -> io::Result<()> {
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -FRX".to_string());
    let mut parts = pager.split_whitespace();
    let Some(program) = parts.next() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty PAGER"));
    };
    let mut child = Command::new(program)
        .args(parts)
        .stdin(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        match stdin.write_all(output) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    let _ = child.wait()?;
    Ok(())
}

fn write_stdout(output: &[u8]) -> Result<()> {
    let stdout = io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    match writer.write_all(output).and_then(|()| writer.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::from(error)),
    }
}

/// Writes an explicitly raw command payload (for example `lane read`) while
/// retaining the renderer's quiet and broken-pipe behavior. Command adapters
/// must use a semantic document for every other successful result.
pub(crate) fn render_raw_content(content: &str, options: &RenderOptions) -> Result<()> {
    if options.quiet {
        return Ok(());
    }
    write_stdout(content.as_bytes())
}

/// Emits a machine protocol acknowledgement. Unlike user-facing raw content,
/// this intentionally ignores `--quiet`: a native agent is waiting for it.
pub(crate) fn render_protocol_content(content: &str) -> Result<()> {
    write_stdout(content.as_bytes())
}

/// Writes a documented structured diagnostic to stderr without routing it
/// through the human renderer.
pub(crate) fn render_structured_error(content: &str) -> Result<()> {
    let stderr = io::stderr();
    let mut writer = BufWriter::new(stderr.lock());
    let result = writer
        .write_all(content.as_bytes())
        .and_then(|()| writer.write_all(b"\n"));
    match result.and_then(|()| writer.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::from(error)),
    }
}

pub(crate) fn render_error_document(
    document: &TerminalDocument,
    options: &RenderOptions,
) -> Result<()> {
    let stderr = io::stderr();
    let mut writer = BufWriter::new(stderr.lock());
    let mut error_options = options.clone();
    error_options.quiet = false;
    let result = Renderer::new(&mut writer, &error_options).render(document);
    match result {
        Ok(()) => match writer.flush() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            Err(error) => Err(Error::from(error)),
        },
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(Error::from(error)),
    }
}

#[cfg(test)]
pub(crate) fn render_document_to_string(
    document: &TerminalDocument,
    options: &RenderOptions,
) -> String {
    let mut output = Vec::new();
    Renderer::new(&mut output, options)
        .render(document)
        .expect("writing to Vec cannot fail");
    String::from_utf8(output).expect("renderer emits UTF-8")
}

pub(crate) fn sanitize_inline(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\u{1b}' => out.push_str("\\x1b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(out, "\\u{{{:04x}}}", character as u32);
            }
            character => out.push(character),
        }
    }
    out
}

pub(crate) fn sanitize_patch(value: &str) -> String {
    value
        .lines()
        .map(|line| sanitize_patch_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn format_timestamp(timestamp: i64, options: &RenderOptions) -> String {
    format_timestamp_at(
        timestamp,
        OffsetDateTime::now_utc().unix_timestamp(),
        options,
    )
}

pub(crate) fn format_timestamp_at(timestamp: i64, now: i64, options: &RenderOptions) -> String {
    if options.mode == RenderMode::Plain {
        return OffsetDateTime::from_unix_timestamp(timestamp)
            .ok()
            .and_then(|value| value.format(&Rfc3339).ok())
            .unwrap_or_else(|| format!("unix:{timestamp}"));
    }
    let elapsed = now.saturating_sub(timestamp);
    match elapsed {
        i64::MIN..=-1 => "just now".to_string(),
        0..=4 => "just now".to_string(),
        5..=59 => format!("{elapsed}s ago"),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        86_400..=604_799 => format!("{}d ago", elapsed / 86_400),
        _ => OffsetDateTime::from_unix_timestamp(timestamp)
            .ok()
            .and_then(|value| value.format(&Rfc3339).ok())
            .unwrap_or_else(|| format!("unix:{timestamp}")),
    }
}

pub(crate) fn format_duration(duration_ms: u64, options: &RenderOptions) -> String {
    if options.mode == RenderMode::Plain {
        return format!("{duration_ms} ms");
    }
    match duration_ms {
        0..=999 => format!("{duration_ms} ms"),
        1_000..=59_999 => format!("{:.1} s", duration_ms as f64 / 1_000.0),
        _ => format!("{:.1} m", duration_ms as f64 / 60_000.0),
    }
}

pub(crate) fn file_change_marker(kind: &FileChangeKind) -> char {
    match kind {
        FileChangeKind::Added => 'A',
        FileChangeKind::Modified => 'M',
        FileChangeKind::Deleted => 'D',
        FileChangeKind::Renamed => 'R',
        FileChangeKind::TypeChanged => 'T',
    }
}

pub(crate) fn file_change_label(kind: &FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "added",
        FileChangeKind::Modified => "modified",
        FileChangeKind::Deleted => "deleted",
        FileChangeKind::Renamed => "renamed",
        FileChangeKind::TypeChanged => "type changed",
    }
}

pub(crate) fn line_change_label(kind: &LineChangeKind) -> &'static str {
    match kind {
        LineChangeKind::Added => "added",
        LineChangeKind::Modified => "modified",
        LineChangeKind::Deleted => "deleted",
        LineChangeKind::Moved => "moved",
    }
}

pub(crate) fn operation_kind_label(kind: &OperationKind) -> &'static str {
    match kind {
        OperationKind::Init => "initialize",
        OperationKind::GitImport => "import from Git",
        OperationKind::FileEdit => "edit file",
        OperationKind::MultiFileEdit => "edit files",
        OperationKind::Format => "format",
        OperationKind::ManualCheckpoint => "checkpoint",
        OperationKind::ManualRecord => "record",
        OperationKind::WatchRecord => "watch record",
        OperationKind::Checkout => "checkout",
        OperationKind::Branch => "branch",
        OperationKind::Merge => "merge",
        OperationKind::LaneSpawn => "create lane",
        OperationKind::LanePatch => "apply lane patch",
        OperationKind::LaneRecord => "record lane",
        OperationKind::LaneRewind => "rewind lane",
        OperationKind::LaneMerge => "merge lane",
        OperationKind::GitExport => "export to Git",
    }
}

pub(crate) fn worktree_state_label(state: &WorktreeState) -> &'static str {
    match state {
        WorktreeState::Clean => "clean",
        WorktreeState::DirtyTracked => "unrecorded changes",
        WorktreeState::DirtyUntracked => "unrecorded changes including untracked paths",
    }
}

/// Converts externally stored status strings into stable user-facing phrases.
pub(crate) fn state_label(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "ok" | "pass" | "passed" | "healthy" => "passed".to_string(),
        "fail" | "failed" | "failure" => "failed".to_string(),
        "warn" | "warning" => "warning".to_string(),
        "pending" | "approval_required" => "pending approval".to_string(),
        "blocked" | "deny" | "denied" => "blocked".to_string(),
        "conflicted" | "conflict" => "conflicted".to_string(),
        "dirty" => "needs record".to_string(),
        "dirty_tracked" => "unrecorded changes".to_string(),
        "dirty_untracked" => "unrecorded changes including untracked paths".to_string(),
        value => value.replace(['_', '-'], " "),
    }
}

fn sanitize_patch_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\u{1b}' => out.push_str("\\x1b"),
            '\r' => out.push_str("\\r"),
            character if character.is_control() && character != '\t' => {
                let _ = write!(out, "\\u{{{:04x}}}", character as u32);
            }
            character => out.push(character),
        }
    }
    out
}

struct Renderer<'a, W: Write> {
    writer: &'a mut W,
    options: &'a RenderOptions,
}

impl<'a, W: Write> Renderer<'a, W> {
    fn new(writer: &'a mut W, options: &'a RenderOptions) -> Self {
        Self { writer, options }
    }

    fn render(&mut self, document: &TerminalDocument) -> io::Result<()> {
        let mut wrote = false;
        if let Some(lead) = &document.lead {
            self.line(&sanitize_inline(&lead.text), lead.tone, true)?;
            wrote = true;
        }
        if let Some(context) = &document.context {
            self.line(&sanitize_inline(context), UiTone::Muted, false)?;
            wrote = true;
        }
        for block in &document.blocks {
            if wrote {
                self.blank()?;
            }
            self.block(block, 0)?;
            wrote = true;
        }
        if let Some(next) = &document.next {
            if wrote {
                self.blank()?;
            }
            self.heading("Next:")?;
            self.command(next, 2)?;
            wrote = true;
        }
        if !document.more.is_empty() && self.options.verbose {
            if wrote {
                self.blank()?;
            }
            self.heading("More:")?;
            for action in &document.more {
                self.command(action, 2)?;
            }
        }
        Ok(())
    }

    fn block(&mut self, block: &UiBlock, indent: usize) -> io::Result<()> {
        match block {
            UiBlock::Paragraph { text, tone } => {
                self.line_indented(&sanitize_inline(text), *tone, false, indent)
            }
            UiBlock::Metadata(entries) => self.metadata(entries, indent),
            UiBlock::Section { title, blocks } => {
                self.heading_indented(title, indent)?;
                for (index, block) in blocks.iter().enumerate() {
                    if index > 0 {
                        self.blank()?;
                    }
                    self.block(block, indent + 2)?;
                }
                Ok(())
            }
            UiBlock::Table(table) => self.table(table, indent),
            UiBlock::Changes(changes) => self.changes(changes, indent),
            UiBlock::Checklist(checks) => self.checklist(checks, indent),
            UiBlock::Diagnostic(diagnostic) => self.diagnostic(diagnostic, indent),
            UiBlock::Notice(text) => {
                self.line_indented(&sanitize_inline(text), UiTone::Attention, false, indent)
            }
            UiBlock::Lines(lines) => {
                for (line, tone) in lines {
                    self.line_indented(&sanitize_inline(line), *tone, false, indent)?;
                }
                Ok(())
            }
            UiBlock::Patch { title, text } => self.patch(title, text, indent),
        }
    }

    fn metadata(&mut self, entries: &[(String, String)], indent: usize) -> io::Result<()> {
        let label_width = entries
            .iter()
            .map(|(label, _)| display_width(&sanitize_inline(label)))
            .max()
            .unwrap_or_default();
        for (label, value) in entries {
            let label = sanitize_inline(label);
            let value = sanitize_inline(value);
            let padding = " ".repeat(label_width.saturating_sub(display_width(&label)));
            let line = format!("{label}{padding}: {value}");
            self.line_indented(&line, UiTone::Neutral, false, indent)?;
        }
        Ok(())
    }

    fn table(&mut self, table: &UiTable, indent: usize) -> io::Result<()> {
        if table.columns.is_empty() || table.rows.is_empty() {
            return Ok(());
        }
        let columns = visible_columns(table, self.options.width.saturating_sub(indent));
        if columns.len() < 2 {
            return self.stacked_table(table, &[], indent);
        }
        let available = self.options.width.saturating_sub(indent);
        let widths = column_widths(table, &columns, available);
        if widths.iter().sum::<usize>() + widths.len().saturating_sub(1) * 2 > available {
            return self.stacked_table(table, &columns, indent);
        }
        let headers = columns
            .iter()
            .zip(&widths)
            .map(|(index, width)| {
                pad(
                    &sanitize_inline(&table.columns[*index].header),
                    *width,
                    table.columns[*index].align,
                )
            })
            .collect::<Vec<_>>()
            .join("  ");
        self.line_indented(&headers, UiTone::Muted, false, indent)?;
        for row in &table.rows {
            let rendered = columns
                .iter()
                .zip(&widths)
                .map(|(index, width)| {
                    let value = row.get(*index).map(String::as_str).unwrap_or("");
                    let value = truncate(&sanitize_inline(value), *width, self.options);
                    pad(&value, *width, table.columns[*index].align)
                })
                .collect::<Vec<_>>()
                .join("  ");
            self.line_indented(&rendered, UiTone::Neutral, false, indent)?;
        }
        Ok(())
    }

    fn stacked_table(
        &mut self,
        table: &UiTable,
        columns: &[usize],
        indent: usize,
    ) -> io::Result<()> {
        let columns = if columns.is_empty() {
            (0..table.columns.len()).collect::<Vec<_>>()
        } else {
            columns.to_vec()
        };
        for (row_index, row) in table.rows.iter().enumerate() {
            if row_index > 0 {
                self.blank()?;
            }
            for index in &columns {
                let value = row.get(*index).map(String::as_str).unwrap_or("");
                let line = format!(
                    "{}: {}",
                    sanitize_inline(&table.columns[*index].header),
                    sanitize_inline(value)
                );
                self.line_indented(&line, UiTone::Neutral, false, indent)?;
            }
        }
        Ok(())
    }

    fn changes(&mut self, changes: &UiChangeList, indent: usize) -> io::Result<()> {
        for change in &changes.changes {
            let mut line = format!("{}  ", change.marker);
            let path = match &change.old_path {
                Some(old_path) => format!(
                    "{} {} {}",
                    sanitize_inline(old_path),
                    self.options.unicode("→", "->"),
                    sanitize_inline(&change.path)
                ),
                None => sanitize_inline(&change.path),
            };
            line.push_str(&path);
            if change.additions > 0 || change.deletions > 0 {
                let _ = write!(line, "  +{} -{}", change.additions, change.deletions);
                if self.options.width >= indent + 48 {
                    line.push_str("  ");
                    line.push_str(&change_bar(change.additions, change.deletions));
                }
            }
            if let Some(detail) = &change.detail {
                let _ = write!(line, "  {}", sanitize_inline(detail));
            }
            self.line_indented(
                &truncate(
                    &line,
                    self.options.width.saturating_sub(indent),
                    self.options,
                ),
                UiTone::Neutral,
                false,
                indent,
            )?;
        }
        let summary = format!(
            "{} file{} changed, {} insertion{}, {} deletion{}",
            changes.changes.len(),
            if changes.changes.len() == 1 { "" } else { "s" },
            changes.additions,
            if changes.additions == 1 { "" } else { "s" },
            changes.deletions,
            if changes.deletions == 1 { "" } else { "s" }
        );
        self.line_indented(&summary, UiTone::Muted, false, indent)?;
        if let Some(omitted) = &changes.omitted {
            let message = format!(
                "{} {} more; run `{}`",
                self.options.unicode("…", "..."),
                omitted.count,
                sanitize_inline(&omitted.command)
            );
            self.line_indented(&message, UiTone::Attention, false, indent)?;
        }
        Ok(())
    }

    fn checklist(&mut self, checks: &[UiCheck], indent: usize) -> io::Result<()> {
        let mut checks = checks.to_vec();
        checks.sort_by(|left, right| check_rank(left.state).cmp(&check_rank(right.state)));
        let label_width = checks
            .iter()
            .map(|check| display_width(&sanitize_inline(&check.label)))
            .max()
            .unwrap_or_default();
        for check in checks {
            let status = check_state_label(check.state);
            let label = sanitize_inline(&check.label);
            let padding = " ".repeat(label_width.saturating_sub(display_width(&label)));
            let line = format!(
                "{status:<7}  {label}{padding}  {}",
                sanitize_inline(&check.detail)
            );
            self.line_indented(&line, check_state_tone(check.state), false, indent)?;
        }
        Ok(())
    }

    fn diagnostic(&mut self, diagnostic: &UiDiagnostic, indent: usize) -> io::Result<()> {
        self.line_indented(
            &format!(
                "error [{}]: {}",
                sanitize_inline(&diagnostic.code),
                sanitize_inline(&diagnostic.summary)
            ),
            UiTone::Failure,
            true,
            indent,
        )?;
        if let Some(cause) = &diagnostic.cause {
            self.line_indented(&sanitize_inline(cause), UiTone::Neutral, false, indent)?;
        }
        if let Some(consequence) = &diagnostic.consequence {
            self.line_indented(
                &sanitize_inline(consequence),
                UiTone::Attention,
                false,
                indent,
            )?;
        }
        if let Some(recovery) = &diagnostic.recovery {
            self.blank()?;
            self.heading_indented("Next:", indent)?;
            self.command_indented(recovery, indent + 2)?;
        }
        for alternative in &diagnostic.alternatives {
            self.command_indented(alternative, indent + 2)?;
        }
        Ok(())
    }

    fn patch(&mut self, title: &str, text: &str, indent: usize) -> io::Result<()> {
        self.line_indented(&sanitize_inline(title), UiTone::Info, true, indent)?;
        let rule = self
            .options
            .unicode("─", "-")
            .repeat(self.options.width.saturating_sub(indent).min(72).max(8));
        self.line_indented(&rule, UiTone::Muted, false, indent)?;
        for line in sanitize_patch(text).lines() {
            let tone = if line.starts_with('+') && !line.starts_with("+++") {
                UiTone::Success
            } else if line.starts_with('-') && !line.starts_with("---") {
                UiTone::Failure
            } else if line.starts_with("@@") {
                UiTone::Info
            } else {
                UiTone::Neutral
            };
            self.line_indented(line, tone, false, indent)?;
        }
        Ok(())
    }

    fn heading(&mut self, value: &str) -> io::Result<()> {
        self.line(value, UiTone::Info, true)
    }

    fn heading_indented(&mut self, value: &str, indent: usize) -> io::Result<()> {
        self.line_indented(value, UiTone::Info, true, indent)
    }

    fn command(&mut self, action: &UiNextAction, indent: usize) -> io::Result<()> {
        self.command_indented(action, indent)
    }

    fn command_indented(&mut self, action: &UiNextAction, indent: usize) -> io::Result<()> {
        self.line_indented(
            &sanitize_inline(&action.command),
            UiTone::Info,
            false,
            indent,
        )?;
        self.line_indented(
            &sanitize_inline(&action.reason),
            UiTone::Muted,
            false,
            indent + 2,
        )
    }

    fn blank(&mut self) -> io::Result<()> {
        self.writer.write_all(b"\n")
    }

    fn line(&mut self, value: &str, tone: UiTone, strong: bool) -> io::Result<()> {
        self.line_indented(value, tone, strong, 0)
    }

    fn line_indented(
        &mut self,
        value: &str,
        tone: UiTone,
        strong: bool,
        indent: usize,
    ) -> io::Result<()> {
        self.writer.write_all(" ".repeat(indent).as_bytes())?;
        let rendered = self.style(value, tone, strong);
        self.writer.write_all(rendered.as_bytes())?;
        self.writer.write_all(b"\n")
    }

    fn style(&self, value: &str, tone: UiTone, strong: bool) -> String {
        if !self.options.color {
            return value.to_string();
        }
        let color = match tone {
            UiTone::Success => AnsiColor::Green,
            UiTone::Attention => AnsiColor::Yellow,
            UiTone::Blocked | UiTone::Failure => AnsiColor::Red,
            UiTone::Info => AnsiColor::Cyan,
            UiTone::Muted => AnsiColor::BrightBlack,
            UiTone::Neutral => {
                return if strong {
                    format!(
                        "{}{}{}",
                        Style::new().bold().render(),
                        value,
                        Style::new().bold().render_reset()
                    )
                } else {
                    value.to_string()
                }
            }
        };
        let style = if strong {
            Style::new().bold().fg_color(Some(Color::Ansi(color)))
        } else {
            Style::new().fg_color(Some(Color::Ansi(color)))
        };
        format!("{}{}{}", style.render(), value, style.render_reset())
    }
}

fn visible_columns(table: &UiTable, available: usize) -> Vec<usize> {
    let mut columns = (0..table.columns.len()).collect::<Vec<_>>();
    loop {
        let minimum = columns
            .iter()
            .map(|index| table.columns[*index].min_width)
            .sum::<usize>()
            + columns.len().saturating_sub(1) * 2;
        if minimum <= available || columns.len() <= 1 {
            return columns;
        }
        let removable = columns
            .iter()
            .enumerate()
            .filter(|(_, index)| table.columns[**index].priority > 0)
            .max_by_key(|(_, index)| table.columns[**index].priority)
            .map(|(position, _)| position);
        match removable {
            Some(position) => {
                columns.remove(position);
            }
            None => return columns,
        }
    }
}

fn column_widths(table: &UiTable, columns: &[usize], available: usize) -> Vec<usize> {
    let gaps = columns.len().saturating_sub(1) * 2;
    let mut widths = columns
        .iter()
        .map(|index| {
            let header = display_width(&sanitize_inline(&table.columns[*index].header));
            let content = table
                .rows
                .iter()
                .map(|row| row.get(*index).map(String::as_str).unwrap_or(""))
                .map(sanitize_inline)
                .map(|value| display_width(&value))
                .max()
                .unwrap_or_default();
            header.max(content).max(table.columns[*index].min_width)
        })
        .collect::<Vec<_>>();
    let target = available.saturating_sub(gaps);
    let current = widths.iter().sum::<usize>();
    if current > target {
        let mut positions = (0..widths.len()).collect::<Vec<_>>();
        positions.sort_by(|left, right| {
            table.columns[columns[*right]]
                .priority
                .cmp(&table.columns[columns[*left]].priority)
                .then_with(|| widths[*right].cmp(&widths[*left]))
        });
        let mut excess = current - target;
        for position in positions {
            let minimum = table.columns[columns[position]].min_width;
            let reducible = widths[position].saturating_sub(minimum);
            let reduction = reducible.min(excess);
            widths[position] -= reduction;
            excess -= reduction;
            if excess == 0 {
                break;
            }
        }
    }
    widths
}

fn pad(value: &str, width: usize, align: UiAlign) -> String {
    let value_width = display_width(value);
    let padding = " ".repeat(width.saturating_sub(value_width));
    match align {
        UiAlign::Left => format!("{value}{padding}"),
        UiAlign::Right => format!("{padding}{value}"),
    }
}

fn truncate(value: &str, max_width: usize, options: &RenderOptions) -> String {
    if display_width(value) <= max_width {
        return value.to_string();
    }
    let marker = options.unicode("…", "...");
    let target = max_width.saturating_sub(display_width(marker));
    let mut output = String::new();
    let mut width = 0;
    for character in value.chars() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > target {
            break;
        }
        output.push(character);
        width += character_width;
    }
    output.push_str(marker);
    output
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn change_bar(additions: u64, deletions: u64) -> String {
    let total = additions + deletions;
    if total == 0 {
        return String::new();
    }
    let width = total.min(20) as usize;
    let plus = usize::try_from((additions * width as u64 + total - 1) / total).unwrap_or(width);
    format!(
        "{}{}",
        "+".repeat(plus),
        "-".repeat(width.saturating_sub(plus))
    )
}

fn check_rank(state: UiCheckState) -> u8 {
    match state {
        UiCheckState::Blocked => 0,
        UiCheckState::Fail => 1,
        UiCheckState::Warn => 2,
        UiCheckState::Pending => 3,
        UiCheckState::Pass => 4,
        UiCheckState::Skip => 5,
    }
}

fn check_state_label(state: UiCheckState) -> &'static str {
    match state {
        UiCheckState::Pass => "PASS",
        UiCheckState::Warn => "WARN",
        UiCheckState::Fail => "FAIL",
        UiCheckState::Blocked => "BLOCKED",
        UiCheckState::Pending => "PENDING",
        UiCheckState::Skip => "SKIP",
    }
}

fn check_state_tone(state: UiCheckState) -> UiTone {
    match state {
        UiCheckState::Pass => UiTone::Success,
        UiCheckState::Warn | UiCheckState::Pending => UiTone::Attention,
        UiCheckState::Fail => UiTone::Failure,
        UiCheckState::Blocked => UiTone::Blocked,
        UiCheckState::Skip => UiTone::Muted,
    }
}

pub(crate) struct TransientProgress {
    active: bool,
}

impl TransientProgress {
    pub(crate) fn start(options: &RenderOptions, message: &str) -> Self {
        if !options.progress_allowed() {
            return Self { active: false };
        }
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r{}", sanitize_inline(message));
        let _ = stderr.flush();
        Self { active: true }
    }

    pub(crate) fn finish(&mut self) {
        if !self.active {
            return;
        }
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = stderr.flush();
        self.active = false;
    }
}

impl Drop for TransientProgress {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_terminal_controls() {
        assert_eq!(
            sanitize_inline("one\u{1b}[31m\ntwo\t"),
            "one\\x1b[31m\\ntwo\\t"
        );
        assert_eq!(
            sanitize_patch("+ one\u{1b}[0m\n- two"),
            "+ one\\x1b[0m\n- two"
        );
    }

    #[test]
    fn table_stacks_when_narrow() {
        let document =
            TerminalDocument::new("Timeline", UiTone::Neutral).block(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("OPERATION", 0, 12),
                    UiColumn::left("BRANCH", 1, 8),
                    UiColumn::left("MESSAGE", 2, 16),
                ],
                vec![vec![
                    "change_alpha".into(),
                    "main".into(),
                    "Improve rendering".into(),
                ]],
            )));
        let output =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Plain, 21));
        assert!(output.contains("OPERATION: change_alpha"));
        assert!(output.contains("BRANCH: main"));
    }

    #[test]
    fn checklist_orders_blockers_before_warnings() {
        let document = TerminalDocument::empty().block(UiBlock::Checklist(vec![
            UiCheck::new(UiCheckState::Pass, "tests", "cargo test"),
            UiCheck::new(UiCheckState::Warn, "freshness", "behind main"),
            UiCheck::new(UiCheckState::Blocked, "worktree", "dirty"),
        ]));
        let output =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Plain, 80));
        assert!(output.find("BLOCKED").unwrap() < output.find("WARN").unwrap());
        assert!(output.find("WARN").unwrap() < output.find("PASS").unwrap());
    }

    #[test]
    fn terminal_layout_goldens_cover_widths_and_policies() {
        let document = TerminalDocument::new("Lane review is blocked", UiTone::Blocked)
            .context("2 blockers · 1 warning")
            .block(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("LANE", 0, 10),
                    UiColumn::left("STATE", 0, 9),
                    UiColumn::left("DETAIL", 2, 18),
                ],
                vec![
                    vec!["fix-login".into(), "blocked".into(), "dirty workdir".into()],
                    vec!["docs".into(), "warning".into(), "base is stale".into()],
                ],
            )))
            .block(UiBlock::Checklist(vec![
                UiCheck::new(UiCheckState::Warn, "base", "3 operations behind"),
                UiCheck::new(UiCheckState::Blocked, "workdir", "record local changes"),
            ]))
            .next(
                "trail lane record fix-login -m \"save local work\"",
                "record the highest-priority blocker before merging",
            );
        let narrow =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Human, 30));
        let standard =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Human, 70));
        let wide =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Human, 110));
        let plain =
            render_document_to_string(&document, &RenderOptions::test(RenderMode::Plain, 70));
        let narrow_expected = concat!(
            "Lane review is blocked\n",
            "2 blockers · 1 warning\n\n",
            "LANE        STATE    \n",
            "fix-login   blocked  \n",
            "docs        warning  \n\n",
            "BLOCKED  workdir  record local changes\n",
            "WARN     base     3 operations behind\n\n",
            "Next:\n",
            "  trail lane record fix-login -m \"save local work\"\n",
            "    record the highest-priority blocker before merging\n",
        );
        let standard_expected = concat!(
            "Lane review is blocked\n",
            "2 blockers · 1 warning\n\n",
            "LANE        STATE      DETAIL            \n",
            "fix-login   blocked    dirty workdir     \n",
            "docs        warning    base is stale     \n\n",
            "BLOCKED  workdir  record local changes\n",
            "WARN     base     3 operations behind\n\n",
            "Next:\n",
            "  trail lane record fix-login -m \"save local work\"\n",
            "    record the highest-priority blocker before merging\n",
        );
        assert_eq!(narrow, narrow_expected);
        assert_eq!(standard, standard_expected);
        assert_eq!(wide, standard_expected);
        assert_eq!(plain, standard_expected);

        let glyph_document = TerminalDocument::new("Rename", UiTone::Neutral).block(
            UiBlock::Changes(UiChangeList::new(vec![UiChange {
                marker: 'R',
                path: "new-name.rs".to_string(),
                old_path: Some("old-name.rs".to_string()),
                additions: 1,
                deletions: 1,
                detail: None,
            }])),
        );
        let mut colored = RenderOptions::test(RenderMode::Human, 80);
        colored.color = true;
        let mut ascii = RenderOptions::test(RenderMode::Human, 80);
        ascii.glyphs = GlyphSet::Ascii;
        assert!(render_document_to_string(&document, &colored).contains("\u{1b}["));
        assert!(render_document_to_string(&glyph_document, &ascii)
            .contains("old-name.rs -> new-name.rs"));
        assert!(render_document_to_string(
            &glyph_document,
            &RenderOptions::test(RenderMode::Human, 80),
        )
        .contains("old-name.rs → new-name.rs"));
    }

    /// An opt-in release benchmark for the renderer cutover. It deliberately
    /// stays dependency-free so it measures the production renderer rather
    /// than a benchmark framework's setup costs.
    #[test]
    #[ignore = "run with scripts/terminal-output-bench.sh before release"]
    fn terminal_render_baseline() {
        use std::time::{Duration, Instant};

        let table = UiTable::new(
            vec![
                UiColumn::left("TASK", 0, 12),
                UiColumn::left("STATE", 0, 10),
                UiColumn::left("DETAIL", 1, 18),
            ],
            (0..10_000)
                .map(|index| {
                    vec![
                        format!("task-{index:05}"),
                        "needs review".to_string(),
                        "rendered table benchmark row".to_string(),
                    ]
                })
                .collect(),
        );
        let table_document =
            TerminalDocument::new("Table benchmark", UiTone::Neutral).block(UiBlock::Table(table));
        let options = RenderOptions::test(RenderMode::Plain, 120);
        let table_start = Instant::now();
        let table_output = render_document_to_string(&table_document, &options);
        let table_elapsed = table_start.elapsed();
        assert!(table_output.lines().count() >= 10_000);
        assert!(
            table_elapsed < Duration::from_millis(750),
            "10,000-row render took {table_elapsed:?}; threshold is 750ms"
        );

        let patch =
            "+ benchmark source line with enough content to exercise sanitization\n".repeat(32_768);
        let patch_document =
            TerminalDocument::new("Patch benchmark", UiTone::Neutral).block(UiBlock::Patch {
                title: "benchmark.rs".to_string(),
                text: patch,
            });
        let patch_start = Instant::now();
        let patch_output = render_document_to_string(&patch_document, &options);
        let patch_elapsed = patch_start.elapsed();
        assert!(patch_output.len() > 1_000_000);
        assert!(
            patch_elapsed < Duration::from_millis(1_500),
            "large patch render took {patch_elapsed:?}; threshold is 1.5s"
        );
        eprintln!(
            "terminal rendering baseline: 10k table={table_elapsed:?}, large patch={patch_elapsed:?}"
        );
    }

    #[test]
    fn plain_timestamps_are_rfc3339_and_human_timestamps_are_relative() {
        let plain = RenderOptions::test(RenderMode::Plain, 80);
        let human = RenderOptions::test(RenderMode::Human, 80);
        assert_eq!(format_timestamp_at(0, 60, &plain), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp_at(0, 60, &human), "1m ago");
    }

    #[test]
    fn plain_durations_are_integer_milliseconds() {
        let plain = RenderOptions::test(RenderMode::Plain, 80);
        let human = RenderOptions::test(RenderMode::Human, 80);
        assert_eq!(format_duration(1_250, &plain), "1250 ms");
        assert_eq!(format_duration(1_250, &human), "1.2 s");
    }

    #[test]
    fn maps_model_states_to_user_facing_labels() {
        assert_eq!(
            worktree_state_label(&WorktreeState::DirtyTracked),
            "unrecorded changes"
        );
        assert_eq!(state_label("dirty"), "needs record");
        assert_eq!(state_label("approval_required"), "pending approval");
    }

    #[test]
    fn color_policy_honors_terminal_capabilities() {
        assert!(resolve_color(
            RenderMode::Human,
            ColorPolicy::Auto,
            true,
            false,
            false
        ));
        assert!(!resolve_color(
            RenderMode::Human,
            ColorPolicy::Auto,
            false,
            false,
            false
        ));
        assert!(!resolve_color(
            RenderMode::Human,
            ColorPolicy::Auto,
            true,
            true,
            false
        ));
        assert!(!resolve_color(
            RenderMode::Human,
            ColorPolicy::Auto,
            true,
            false,
            true
        ));
        assert!(resolve_color(
            RenderMode::Human,
            ColorPolicy::Always,
            false,
            true,
            true
        ));
    }

    #[test]
    fn pager_policy_requires_long_interactive_output() {
        let mut options = RenderOptions::test(RenderMode::Human, 80);
        options.height = 3;
        options.pager = PagerPolicy::Auto;
        assert!(should_send_to_pager(b"one\ntwo\nthree\nfour\n", &options));
        assert!(!should_send_to_pager(b"one\ntwo\n", &options));
        options.pager = PagerPolicy::Always;
        assert!(should_send_to_pager(b"one\n", &options));
    }
}
