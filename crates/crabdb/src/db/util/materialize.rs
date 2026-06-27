use super::*;

pub(crate) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<u64> {
    if !source.exists() {
        return Ok(0);
    }
    fs::create_dir_all(destination)?;
    let mut bytes = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = fs::read_link(&source_path)?;
                symlink_file(target, destination_path)?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::InvalidInput(format!(
                    "cannot copy symlink `{}` on this platform",
                    source_path.display()
                )));
            }
        } else if metadata.is_dir() {
            bytes += copy_dir_recursive(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            bytes += fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(bytes)
}

pub(crate) fn materialize_into_batched_report<F>(
    workspace_root: &Path,
    output_root: &Path,
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    batch_size: usize,
    bytes_for_batch: F,
) -> Result<MaterializedWorkdir>
where
    F: Fn(&BTreeMap<String, FileEntry>) -> Result<BTreeMap<String, Vec<u8>>>,
{
    materialize_into_batched_with_durability(
        workspace_root,
        output_root,
        previous,
        target,
        batch_size,
        true,
        bytes_for_batch,
    )
}

pub(crate) fn materialize_into_batched_best_effort_report<F>(
    workspace_root: &Path,
    output_root: &Path,
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    batch_size: usize,
    bytes_for_batch: F,
) -> Result<MaterializedWorkdir>
where
    F: Fn(&BTreeMap<String, FileEntry>) -> Result<BTreeMap<String, Vec<u8>>>,
{
    materialize_into_batched_with_durability(
        workspace_root,
        output_root,
        previous,
        target,
        batch_size,
        false,
        bytes_for_batch,
    )
}

fn materialize_into_batched_with_durability<F>(
    workspace_root: &Path,
    output_root: &Path,
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    batch_size: usize,
    durable: bool,
    bytes_for_batch: F,
) -> Result<MaterializedWorkdir>
where
    F: Fn(&BTreeMap<String, FileEntry>) -> Result<BTreeMap<String, Vec<u8>>>,
{
    reject_case_insensitive_collisions(output_root, target)?;
    let mut report = MaterializedWorkdir::default();
    for path in previous.keys() {
        if !target.contains_key(path) {
            let abs = safe_join(output_root, path)?;
            if abs.exists() {
                fs::remove_file(abs)?;
            }
        }
    }

    let batch_size = batch_size.max(1);
    let mut batch = BTreeMap::new();
    for (path, entry) in target {
        batch.insert(path.clone(), entry.clone());
        if batch.len() >= batch_size {
            report.extend(materialize_batch(
                output_root,
                &batch,
                durable,
                &bytes_for_batch,
            )?);
            batch.clear();
        }
    }
    if !batch.is_empty() {
        report.extend(materialize_batch(
            output_root,
            &batch,
            durable,
            &bytes_for_batch,
        )?);
    }

    let _ = workspace_root;
    Ok(report)
}

fn materialize_batch<F>(
    output_root: &Path,
    batch: &BTreeMap<String, FileEntry>,
    durable: bool,
    bytes_for_batch: &F,
) -> Result<MaterializedWorkdir>
where
    F: Fn(&BTreeMap<String, FileEntry>) -> Result<BTreeMap<String, Vec<u8>>>,
{
    let bytes = bytes_for_batch(batch)?;
    let mut report = MaterializedWorkdir::default();
    for (path, entry) in batch {
        let abs = safe_join(output_root, path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let Some(file_bytes) = bytes.get(path) else {
            return Err(Error::Corrupt(format!(
                "missing materialized bytes for `{path}`"
            )));
        };
        let stamp = write_materialized_file_with_durability(
            &abs,
            path,
            file_bytes,
            entry.executable,
            durable,
        )?;
        report.insert_stamp(path.clone(), stamp);
    }
    Ok(report)
}

pub(crate) fn reject_case_insensitive_collisions(
    output_root: &Path,
    target: &BTreeMap<String, FileEntry>,
) -> Result<()> {
    if !is_case_insensitive_filesystem(output_root)? {
        return Ok(());
    }
    validate_no_case_fold_collisions(target.keys())
}

pub(crate) fn validate_no_case_fold_collisions<'a, I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut seen = HashMap::new();
    for path in paths {
        let folded = path.to_lowercase();
        if let Some(previous) = seen.insert(folded, path.clone()) {
            if previous != *path {
                return Err(Error::InvalidPath {
                    path: path.clone(),
                    reason: format!("case-insensitive path collision with `{previous}`"),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn is_case_insensitive_filesystem(root: &Path) -> Result<bool> {
    let root = root.canonicalize()?;
    for _ in 0..16 {
        let lower_name = format!(".crabdb-case-probe-{}-a", now_nanos());
        let upper_name = lower_name.to_ascii_uppercase();
        let lower = root.join(&lower_name);
        let upper = root.join(&upper_name);
        match OpenOptions::new().write(true).create_new(true).open(&lower) {
            Ok(_) => {
                let insensitive = upper.exists();
                let _ = fs::remove_file(&lower);
                return Ok(insensitive);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create filesystem case-sensitivity probe".to_string(),
    ))
}

pub(crate) fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let normalized = normalize_relative_path(rel)?;
    let root_canon = root.canonicalize()?;
    let candidate = root_canon.join(path_from_rel(&normalized));
    ensure_no_symlink_ancestors(&root_canon, &normalized, rel)?;
    if let Some(parent) = candidate.parent() {
        let parent = if parent.exists() {
            parent.canonicalize()?
        } else {
            let mut existing = parent;
            while !existing.exists() {
                existing = existing.parent().ok_or_else(|| Error::InvalidPath {
                    path: rel.to_string(),
                    reason: "path has no existing ancestor".to_string(),
                })?;
            }
            existing.canonicalize()?
        };
        if !parent.starts_with(root_canon) {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path escapes output root".to_string(),
            });
        }
    }
    Ok(candidate)
}

pub(crate) fn ensure_no_symlink_ancestors(root: &Path, normalized: &str, rel: &str) -> Result<()> {
    let mut current = root.to_path_buf();
    let rel_path = path_from_rel(normalized);
    let Some(parent) = rel_path.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path uses a symlink ancestor".to_string(),
                    });
                }
                if !metadata.is_dir() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path parent is not a directory".to_string(),
                    });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Ok(())
}

fn write_materialized_file_with_durability(
    path: &Path,
    rel: &str,
    bytes: &[u8],
    executable: bool,
    durable: bool,
) -> Result<WorkdirFileStamp> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "refusing to follow symlink for write".to_string(),
            });
        }
        if !metadata.is_file() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path is not a regular file".to_string(),
            });
        }
    }
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: rel.to_string(),
        reason: "path has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;

    let (tmp, mut file) = create_materialize_temp_file(parent, path)?;
    let result = (|| -> Result<WorkdirFileStamp> {
        file.write_all(bytes)?;
        if durable {
            file.sync_all()?;
        }
        drop(file);
        set_executable(&tmp, executable)?;
        fs::rename(&tmp, path)?;
        if durable {
            sync_directory(parent);
        }
        let metadata = fs::symlink_metadata(path)?;
        Ok(WorkdirFileStamp::from_metadata(&metadata))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub(crate) fn create_materialize_temp_file(
    parent: &Path,
    path: &Path,
) -> Result<(PathBuf, fs::File)> {
    let leaf = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "file".into());
    for _ in 0..16 {
        let tmp = parent.join(format!(".{leaf}.crabdb-tmp-{}", now_nanos()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create materialization temp file".to_string(),
    ))
}

pub(crate) fn sync_directory(path: &Path) {
    if let Ok(dir) = OpenOptions::new().read(true).open(path) {
        let _ = dir.sync_all();
    }
}
