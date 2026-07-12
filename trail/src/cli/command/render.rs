use trail::Result;

mod acp;
mod agent;
mod collaboration;
mod config;
mod diff;
mod git;
mod guardrails;
mod ignore;
mod inspection;
mod lane;
mod maintenance;
mod ui;
mod workspace;

pub(crate) use acp::*;
pub(crate) use agent::*;
pub(crate) use collaboration::*;
pub(crate) use config::*;
pub(crate) use diff::*;
pub(crate) use git::*;
pub(crate) use guardrails::*;
pub(crate) use ignore::*;
pub(crate) use inspection::*;
pub(crate) use lane::*;
pub(crate) use maintenance::*;
pub(crate) use ui::*;
pub(crate) use workspace::*;

pub(crate) fn render_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<()> {
    render_structured_content(&serde_json::to_string_pretty(value)?)
}

/// Emits one JSON record for a command whose documented contract is a record
/// stream. Human formatting must never use this path.
pub(crate) fn render_ndjson<T: serde::Serialize + ?Sized>(value: &T) -> Result<()> {
    render_structured_content(&serde_json::to_string(value)?)
}

fn render_structured_content(content: &str) -> Result<()> {
    let mut content = content.to_string();
    content.push('\n');
    render_raw_content(
        &content,
        &RenderOptions {
            mode: RenderMode::Plain,
            color: false,
            glyphs: GlyphSet::Ascii,
            width: 80,
            height: 24,
            stdout_is_terminal: false,
            stderr_is_terminal: false,
            verbose: false,
            quiet: false,
            pager: PagerPolicy::Never,
        },
    )
}
