use super::*;

pub(super) fn handle_acp_command(ctx: &RuntimeContext, acp: AcpCommand) -> Result<()> {
    match acp.command {
        AcpSubcommand::Relay(args) => handle_acp_relay(ctx, args),
    }
}

fn handle_acp_relay(ctx: &RuntimeContext, args: AcpRelayArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let materialize = if args.no_materialize {
        false
    } else if let Some(materialize) = args.materialize {
        materialize
    } else {
        true
    };

    crabdb::acp::run_stdio_relay(AcpRelayOptions {
        workspace_root: db.workspace_root().to_path_buf(),
        db_dir: db.db_dir().to_path_buf(),
        lane: args.lane,
        from_ref: args.from,
        provider: args.provider,
        model: args.model,
        materialize,
        workdir: args.workdir,
        inject_mcp: !args.no_mcp,
        upstream_command: args.command,
    })
}
