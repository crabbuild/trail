use serde_json::Value;

mod agent;
mod annotations;
mod collaboration;
mod core;
mod merge;
mod turns;

pub(crate) fn tools() -> Value {
    let mut tools = core::tools();
    append_tools(&mut tools, agent::tools());
    append_tools(&mut tools, collaboration::tools());
    append_tools(&mut tools, merge::tools());
    append_tools(&mut tools, turns::tools());
    annotations::annotate_tools(&mut tools);
    tools
}

fn append_tools(tools: &mut Value, more: Value) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    let Some(more) = more.as_array() else {
        return;
    };
    tools.extend(more.iter().cloned());
}
