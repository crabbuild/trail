use super::*;

pub(crate) fn merge_files_with_resolution(
    base: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    source: &BTreeMap<String, FileEntry>,
    conflict_paths: &BTreeSet<String>,
    take: ConflictTake,
) -> Result<BTreeMap<String, FileEntry>> {
    let mut merged = target.clone();
    let mut unresolved = Vec::new();
    let mut paths = BTreeSet::new();
    paths.extend(base.keys().cloned());
    paths.extend(target.keys().cloned());
    paths.extend(source.keys().cloned());
    for path in paths {
        let base_entry = base.get(&path);
        let target_entry = target.get(&path);
        let source_entry = source.get(&path);
        let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
        let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
        match (target_changed, source_changed) {
            (false, true) => match source_entry {
                Some(entry) => {
                    merged.insert(path.clone(), entry.clone());
                }
                None => {
                    merged.remove(&path);
                }
            },
            (true, true) if entry_hash(target_entry) != entry_hash(source_entry) => {
                if !conflict_paths.contains(&path) {
                    unresolved.push(format!("conflict path `{path}` was not recorded"));
                    continue;
                }
                if take == ConflictTake::Source {
                    match source_entry {
                        Some(entry) => {
                            merged.insert(path.clone(), entry.clone());
                        }
                        None => {
                            merged.remove(&path);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !unresolved.is_empty() {
        return Err(Error::Conflict(unresolved.join("; ")));
    }
    Ok(merged)
}

pub(crate) fn entry_hash(entry: Option<&FileEntry>) -> Option<(&str, bool, &FileKind)> {
    entry.map(|entry| (entry.content_hash.as_str(), entry.executable, &entry.kind))
}
