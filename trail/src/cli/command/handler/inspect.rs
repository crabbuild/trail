use super::*;

pub(super) fn handle_timeline_command(ctx: &RuntimeContext, args: TimelineArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let branch = args.branch.as_deref().or(ctx.branch.as_deref());
    let entries = db.timeline_query(
        branch,
        args.session.as_deref(),
        args.lane.as_deref(),
        args.limit,
    )?;
    render_timeline(&entries, ctx.json, &ctx.render)
}

pub(super) fn handle_show_command(ctx: &RuntimeContext, args: ShowArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let result = db.show(&args.selector)?;
    render_show(&result, ctx.json, &ctx.render)
}

pub(super) fn handle_object_command(ctx: &RuntimeContext, object: ObjectCommand) -> Result<()> {
    match object.command {
        ObjectSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let report = db.inspect_object(&args.object_id)?;
            render_object_inspect(&report, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_root_command(ctx: &RuntimeContext, root: RootCommand) -> Result<()> {
    match root.command {
        RootSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let report = db.inspect_root(&args.root_id)?;
            render_root_inspect(&report, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_text_command(ctx: &RuntimeContext, text: TextCommand) -> Result<()> {
    match text.command {
        TextSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let report = db.inspect_text(&args.text_id, args.limit)?;
            render_text_inspect(&report, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_map_command(ctx: &RuntimeContext, map: MapCommand) -> Result<()> {
    match map.command {
        MapSubcommand::Range(args) => {
            let db = open_db(ctx)?;
            let report = db.inspect_map_range(
                &args.map_id,
                args.map_type.as_str(),
                args.start.as_deref(),
                args.end.as_deref(),
                args.limit,
            )?;
            render_map_range(&report, ctx.json, &ctx.render)
        }
        MapSubcommand::Diff(args) => {
            let db = open_db(ctx)?;
            let report = db.inspect_map_diff(
                &args.left_map_id,
                &args.right_map_id,
                args.map_type.as_str(),
                args.start.as_deref(),
                args.end.as_deref(),
                args.limit,
            )?;
            render_map_diff(&report, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_why_command(ctx: &RuntimeContext, args: WhyArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let at = args.at.as_deref().or(ctx.branch.as_deref());
    let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
        (Some(path_line), None) => db.why(path_line, at)?,
        (None, Some(line_id)) => db.why_line_id(line_id, at)?,
        (Some(_), Some(_)) => {
            return Err(Error::InvalidInput(
                "why accepts either PATH:LINE or --line-id, not both".to_string(),
            ));
        }
        (None, None) => {
            return Err(Error::InvalidInput(
                "why requires PATH:LINE or --line-id".to_string(),
            ));
        }
    };
    render_why(&result, ctx.json, &ctx.render)
}

pub(super) fn handle_history_command(ctx: &RuntimeContext, args: HistoryArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let result = match (
        args.selector.as_deref(),
        args.file_id.as_deref(),
        args.line_id.as_deref(),
    ) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            return Err(Error::InvalidInput(
                "history accepts one path, --file-id, or --line-id selector".to_string(),
            ));
        }
        (_, Some(file_id), None) => db.history_for_file_id(file_id)?,
        (_, None, Some(line_id)) => db.history_for_line_id(line_id)?,
        (Some(path), None, None) => db.history_for_path(path)?,
        (None, None, None) => {
            return Err(Error::InvalidInput(
                "history requires a path, --file-id, or --line-id".to_string(),
            ));
        }
    };
    render_history(&result, ctx.json, &ctx.render)
}

pub(super) fn handle_code_from_command(ctx: &RuntimeContext, args: CodeFromArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let result = db.code_from(&args.selector)?;
    render_code_from(&result, ctx.json, &ctx.render)
}
