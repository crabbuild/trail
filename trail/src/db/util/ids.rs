use super::*;

pub(crate) fn branch_ref(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("{MAIN_REF_PREFIX}{branch}")
    }
}

pub(crate) fn lane_ref(lane: &str) -> String {
    if lane.starts_with("refs/") {
        lane.to_string()
    } else {
        format!("{LANE_REF_PREFIX}{lane}")
    }
}

pub(crate) fn content_object_id(content: &FileContentRef) -> &ObjectId {
    match content {
        FileContentRef::Text(object_id)
        | FileContentRef::Opaque(object_id)
        | FileContentRef::Binary(object_id) => object_id,
    }
}

pub(crate) fn file_id_key(file_id: &FileId) -> String {
    format!("{}:{}", file_id.origin_change.0, file_id.local_seq)
}

pub(crate) fn line_id_key_value(line_id: &LineId) -> String {
    format!("{}:{}", line_id.origin_change.0, line_id.local_seq)
}

pub(crate) fn parse_line_id_key(value: &str) -> Result<LineId> {
    let (change_id, local_seq) = value.rsplit_once(':').ok_or_else(|| {
        Error::InvalidInput("line id must look like `ch_...:<local_seq>`".to_string())
    })?;
    if !change_id.starts_with("ch_") {
        return Err(Error::InvalidInput(format!(
            "line id change id must start with `ch_`, got `{change_id}`"
        )));
    }
    let local_seq = local_seq.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("invalid line id local sequence `{local_seq}`"))
    })?;
    Ok(LineId::new(ChangeId(change_id.to_string()), local_seq))
}

pub(crate) trait LineChangeExt {
    fn line_id_key(&self) -> String;
}

impl LineChangeExt for LineChange {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}

pub(crate) trait LineEntryExt {
    fn line_id_key(&self) -> String;
}

impl LineEntryExt for LineEntry {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}
