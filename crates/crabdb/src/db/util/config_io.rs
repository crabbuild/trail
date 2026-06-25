use super::*;

pub(crate) fn read_config(db_dir: &Path) -> Result<CrabConfig> {
    let text = fs::read_to_string(db_dir.join(CONFIG_FILE))?;
    Ok(toml::from_str(&text)?)
}

pub(crate) fn write_config(db_dir: &Path, config: &CrabConfig) -> Result<()> {
    let path = db_dir.join(CONFIG_FILE);
    let temp = db_dir.join(format!("{CONFIG_FILE}.tmp.{}", now_nanos()));
    fs::write(&temp, toml::to_string_pretty(config)?)?;
    if let Err(err) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(Error::Io(err));
    }
    Ok(())
}
