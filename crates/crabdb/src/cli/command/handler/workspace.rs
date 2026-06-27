use super::*;

pub(super) fn handle_config_command(ctx: &RuntimeContext, config: ConfigCommand) -> Result<()> {
    match config.command {
        ConfigSubcommand::List => {
            let db = open_db(ctx)?;
            let entries = db.config_entries();
            render_config_list(&entries, ctx.json, ctx.quiet)
        }
        ConfigSubcommand::Get(args) => {
            let db = open_db(ctx)?;
            let entry = db.config_get(&args.key)?;
            render_config_entry(&entry, ctx.json, ctx.quiet)
        }
        ConfigSubcommand::Set(args) => {
            let mut db = open_db(ctx)?;
            let report = db.config_set(&args.key, &args.value)?;
            render_config_set(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_ignore_command(ctx: &RuntimeContext, ignore: IgnoreCommand) -> Result<()> {
    match ignore.command {
        IgnoreSubcommand::List => {
            let db = open_db(ctx)?;
            let report = db.ignore_list()?;
            render_ignore_list(&report, ctx.json, ctx.quiet)
        }
        IgnoreSubcommand::Add(args) => {
            let mut db = open_db(ctx)?;
            let report = db.ignore_add(&args.pattern)?;
            render_ignore_add(&report, ctx.json, ctx.quiet)
        }
        IgnoreSubcommand::Remove(args) => {
            let mut db = open_db(ctx)?;
            let report = db.ignore_remove(&args.pattern)?;
            render_ignore_remove(&report, ctx.json, ctx.quiet)
        }
        IgnoreSubcommand::Check(args) => {
            let db = open_db(ctx)?;
            let report = db.ignore_check(&args.path)?;
            render_ignore_check(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_guardrails_command(
    ctx: &RuntimeContext,
    guardrails: GuardrailsCommand,
) -> Result<()> {
    match guardrails.command {
        GuardrailsSubcommand::Check(args) => {
            let db = open_db(ctx)?;
            let payload = parse_optional_json(args.payload_json.as_deref())?;
            let report = db.guardrail_check(
                args.lane.as_deref(),
                &args.action,
                args.summary.as_deref(),
                payload,
                &args.paths,
            )?;
            render_guardrail_check(&report, ctx.json, ctx.quiet)
        }
    }
}
