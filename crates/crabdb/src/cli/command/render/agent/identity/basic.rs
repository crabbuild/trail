use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_agent_spawn(report: &AgentSpawnReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Spawned {} at {}", report.agent_id, report.base_change.0);
        if let Some(workdir) = &report.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_list(entries: &[AgentDetails], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} {} {}",
                entry.record.name,
                entry.branch.status,
                entry.branch.head_change.0,
                entry.branch.ref_name
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_details(details: &AgentDetails, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Agent: {}", details.record.name);
        println!("ID: {}", details.record.agent_id);
        println!("Ref: {}", details.branch.ref_name);
        println!("Status: {}", details.branch.status);
        println!("Base: {}", details.branch.base_change.0);
        println!("Head: {}", details.branch.head_change.0);
        if let Some(provider) = &details.record.provider {
            println!("Provider: {provider}");
        }
        if let Some(model) = &details.record.model {
            println!("Model: {model}");
        }
        if let Some(session_id) = &details.branch.session_id {
            println!("Session: {session_id}");
        }
        if let Some(workdir) = &details.branch.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}
