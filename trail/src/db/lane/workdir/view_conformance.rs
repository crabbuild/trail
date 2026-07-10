use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewConformanceResult {
    pub(crate) changed_paths: BTreeSet<String>,
}

/// One protocol-independent operation trace used by mounted FUSE, NFS, and
/// Dokan acceptance tests. The trace deliberately exercises mixed lower/upper
/// directories, ranged writes, metadata, rename, delete, and remount-visible
/// state through ordinary filesystem APIs.
pub(crate) fn run_mounted_view_conformance(root: &Path) -> Result<ViewConformanceResult> {
    if fs::read(root.join("README.md"))? != b"baseline\n" {
        return Err(Error::InvalidInput(
            "view conformance fixture has the wrong README baseline".to_string(),
        ));
    }
    fs::write(root.join("README.md"), b"changed\n")?;
    fs::create_dir_all(root.join("src/generated"))?;
    fs::write(root.join("src/generated/new.txt"), b"new\n")?;
    fs::rename(root.join("src/lower.txt"), root.join("src/renamed.txt"))?;
    let file = OpenOptions::new()
        .write(true)
        .open(root.join("script.sh"))?;
    file.set_len(3)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("script.sh"), fs::Permissions::from_mode(0o755))?;
    }
    fs::remove_file(root.join("delete.txt"))?;
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("renamed.txt", root.join("src/link.txt"))?;
        if fs::read_link(root.join("src/link.txt"))? != Path::new("renamed.txt")
            || fs::read(root.join("src/link.txt"))? != b"lower\n"
        {
            return Err(Error::InvalidInput(
                "view conformance symlink behavior failed".to_string(),
            ));
        }
        fs::remove_file(root.join("src/link.txt"))?;
    }
    if fs::read(root.join("src/renamed.txt"))? != b"lower\n"
        || fs::read(root.join("script.sh"))? != b"abc"
    {
        return Err(Error::InvalidInput(
            "view conformance read-after-mutation failed".to_string(),
        ));
    }
    Ok(ViewConformanceResult {
        changed_paths: BTreeSet::from([
            "README.md".to_string(),
            "delete.txt".to_string(),
            "script.sh".to_string(),
            "src/generated/new.txt".to_string(),
            "src/lower.txt".to_string(),
            "src/renamed.txt".to_string(),
        ]),
    })
}
