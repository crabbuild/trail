use super::*;

pub(crate) fn write_ref_file(
    db_dir: &Path,
    name: &str,
    change_id: &ChangeId,
    root_id: &ObjectId,
    operation_id: &ObjectId,
    generation: i64,
) -> Result<()> {
    let path = db_dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "name": name,
        "change_id": change_id.0,
        "root_id": root_id.0,
        "operation_id": operation_id.0,
        "generation": generation,
        "updated_at": now_ts(),
    });
    fs::write(path, serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

pub(crate) fn remove_ref_file(db_dir: &Path, name: &str) -> Result<()> {
    let path = db_dir.join(name);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::Io(err)),
    }
}
