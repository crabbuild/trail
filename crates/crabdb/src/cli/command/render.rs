use crabdb::Result;

mod agent;
mod collaboration;
mod config;
mod diff;
mod git;
mod guardrails;
mod ignore;
mod inspection;
mod maintenance;
mod workspace;

pub(crate) use agent::*;
pub(crate) use collaboration::*;
pub(crate) use config::*;
pub(crate) use diff::*;
pub(crate) use git::*;
pub(crate) use guardrails::*;
pub(crate) use ignore::*;
pub(crate) use inspection::*;
pub(crate) use maintenance::*;
pub(crate) use workspace::*;

pub(crate) fn render_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
