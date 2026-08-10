use super::*;
use crate::ids::ArtifactTreeId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewConformanceResult {
    pub(crate) changed_paths: BTreeSet<String>,
}

pub(crate) fn lazy_artifact_conformance_binding(
    db: &Trail,
    fixture_root: &Path,
) -> Result<(WorkspaceLayerBinding, ArtifactTreeId, PathBuf)> {
    let source = fixture_root.join("lazy-artifact-input");
    fs::create_dir_all(source.join("payload/pkg"))?;
    fs::write(source.join("payload/pkg/index.js"), b"shared artifact\n")?;
    fs::write(source.join("payload/pkg/tool.js"), b"tool artifact\n")?;
    let tree_id = {
        let _lock = db.acquire_write_lock()?;
        db.ingest_artifact_tree_under_write_lock(&source)?.0
    };
    let missing_cache = fixture_root.join("never-materialized-artifact-layer");
    Ok((
        WorkspaceLayerBinding {
            binding_identity: "lazy-artifact-conformance".into(),
            layer_id: Some("lazy-artifact-conformance".into()),
            mount_path: "node_modules".into(),
            storage_path: Some(missing_cache.clone()),
            artifact_tree_id: Some(tree_id.clone()),
            artifact_subpath: "payload".into(),
            kind: "dependency".into(),
            priority: 100,
        },
        tree_id,
        missing_cache,
    ))
}

pub(crate) fn run_mounted_lazy_artifact_conformance(
    root: &Path,
    missing_cache: &Path,
) -> Result<()> {
    let index = root.join("node_modules/pkg/index.js");
    let tool = root.join("node_modules/pkg/tool.js");
    if fs::read(&index)? != b"shared artifact\n" || fs::read(&tool)? != b"tool artifact\n" {
        return Err(Error::InvalidInput(
            "mounted lazy artifact baseline is invalid".into(),
        ));
    }
    if missing_cache.exists() {
        return Err(Error::InvalidInput(
            "lazy artifact backend unexpectedly materialized the complete layer".into(),
        ));
    }
    let file = OpenOptions::new().write(true).open(&index)?;
    use std::io::{Seek, SeekFrom, Write};
    let mut file = file;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(b"private")?;
    file.sync_all()?;
    fs::remove_file(&tool)?;
    if fs::read(&index)? != b"privateartifact\n" || tool.exists() || missing_cache.exists() {
        return Err(Error::InvalidInput(
            "mounted lazy artifact copy-up or whiteout behavior failed".into(),
        ));
    }
    Ok(())
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
