use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewMutationKind {
    Write,
    Create,
    Mkdir,
    Metadata,
    Delete,
    Rename,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ViewMutationRecord {
    pub(crate) sequence: u64,
    #[serde(default)]
    pub(crate) class: ViewPathClass,
    pub(crate) kind: ViewMutationKind,
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) destination_class: Option<ViewPathClass>,
}

/// Append-only dirty-path index for a workspace view.
///
/// The upper tree and whiteouts remain authoritative. A missing or truncated
/// journal tail is recoverable by scanning the upper; a complete corrupt line
/// is rejected because it cannot be attributed to an interrupted append.
pub(crate) struct ViewMutationJournal {
    path: PathBuf,
    next_sequence: u64,
    clean_sequence: u64,
    dirty_paths: BTreeSet<String>,
    dirty_source_paths: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewCheckpointCandidates {
    pub(crate) journal_sequence: u64,
    pub(crate) paths: BTreeSet<String>,
}

#[cfg(test)]
pub(crate) fn recover_view_checkpoint_candidates(
    upperdir: &Path,
    lower_files: &BTreeMap<String, FileEntry>,
) -> Result<ViewCheckpointCandidates> {
    let journal = ViewMutationJournal::open(upperdir)?;
    let mut paths = journal.dirty_source_paths().clone();
    for entry in walkdir::WalkDir::new(upperdir) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = normalize_relative_path(
            &entry
                .path()
                .strip_prefix(upperdir)
                .map_err(|err| Error::InvalidInput(err.to_string()))?
                .to_string_lossy(),
        )?;
        if path == ".trail" || path.starts_with(".trail/") {
            continue;
        }
        if classify_view_path(&path).checkpoints() {
            paths.insert(path);
        }
    }
    let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
    let whiteouts = load_source_whiteouts(&layout)?;
    for whiteout in whiteouts {
        let whiteout = normalize_relative_path(&whiteout)?;
        if lower_files.contains_key(&whiteout) {
            paths.insert(whiteout.clone());
        }
        let prefix = format!("{whiteout}/");
        paths.extend(
            lower_files
                .keys()
                .filter(|path| path.starts_with(&prefix))
                .cloned(),
        );
    }
    Ok(ViewCheckpointCandidates {
        journal_sequence: journal.last_sequence(),
        paths,
    })
}

pub(crate) fn recover_view_checkpoint_candidates_for_root(
    db: &Trail,
    upperdir: &Path,
    root_id: &ObjectId,
) -> Result<ViewCheckpointCandidates> {
    let journal = ViewMutationJournal::open(upperdir)?;
    let mut paths = journal.dirty_source_paths().clone();
    for entry in walkdir::WalkDir::new(upperdir) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = normalize_relative_path(
            &entry
                .path()
                .strip_prefix(upperdir)
                .map_err(|err| Error::InvalidInput(err.to_string()))?
                .to_string_lossy(),
        )?;
        if path == ".trail" || path.starts_with(".trail/") {
            continue;
        }
        if classify_view_path(&path).checkpoints() {
            paths.insert(path);
        }
    }
    let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
    let whiteouts = load_source_whiteouts(&layout)?;
    for whiteout in whiteouts {
        let whiteout = normalize_relative_path(&whiteout)?;
        paths.extend(
            db.load_root_files_for_selections(root_id, &[whiteout])?
                .into_keys(),
        );
    }
    Ok(ViewCheckpointCandidates {
        journal_sequence: journal.last_sequence(),
        paths,
    })
}

impl ViewMutationJournal {
    pub(crate) fn open(upperdir: &Path) -> Result<Self> {
        let path = ViewUpperLayout::from_source_upper(upperdir.to_path_buf()).journal_path();
        let mut journal = Self {
            path,
            next_sequence: 1,
            clean_sequence: clean_checkpoint_sequence(upperdir)?,
            dirty_paths: BTreeSet::new(),
            dirty_source_paths: BTreeSet::new(),
        };
        journal.replay()?;
        Ok(journal)
    }

    pub(crate) fn append(
        &mut self,
        kind: ViewMutationKind,
        path: &str,
        destination: Option<&str>,
    ) -> Result<u64> {
        let path = normalize_relative_path(path)?;
        let destination = destination.map(normalize_relative_path).transpose()?;
        let class = classify_view_path(&path);
        let destination_class = destination.as_deref().map(classify_view_path);
        if matches!(kind, ViewMutationKind::Write | ViewMutationKind::Metadata)
            && self.dirty_paths.contains(&path)
            && destination.is_none()
        {
            return Ok(self.next_sequence.saturating_sub(1));
        }
        let record = ViewMutationRecord {
            sequence: self.next_sequence,
            class,
            kind,
            path,
            destination,
            destination_class,
        };
        let mut encoded = serde_json::to_vec(&record)?;
        encoded.push(b'\n');
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&encoded)?;
        file.sync_data()?;
        self.apply(&record);
        self.next_sequence += 1;
        Ok(record.sequence)
    }

    pub(crate) fn observe_checkpoint(&mut self, sequence: u64) {
        if sequence <= self.clean_sequence {
            return;
        }
        self.clean_sequence = sequence;
        self.dirty_paths.clear();
        self.dirty_source_paths.clear();
    }

    #[cfg(test)]
    pub(crate) fn dirty_paths(&self) -> &BTreeSet<String> {
        &self.dirty_paths
    }

    pub(crate) fn dirty_source_paths(&self) -> &BTreeSet<String> {
        &self.dirty_source_paths
    }

    pub(crate) fn last_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    fn replay(&mut self) -> Result<()> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(Error::Io(err)),
        };
        let mut previous = 0;
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if line.last() != Some(&b'\n') {
                break;
            }
            let payload = &line[..line.len() - 1];
            if payload.is_empty() {
                continue;
            }
            let mut record: ViewMutationRecord =
                serde_json::from_slice(payload).map_err(|err| {
                    Error::Corrupt(format!(
                    "workspace view mutation journal `{}` has an invalid complete record: {err}",
                    self.path.display()
                ))
                })?;
            record.path = normalize_relative_path(&record.path)?;
            record.destination = record
                .destination
                .as_deref()
                .map(normalize_relative_path)
                .transpose()?;
            if record.sequence <= previous {
                return Err(Error::Corrupt(format!(
                    "workspace view mutation journal `{}` is not strictly ordered at sequence {}",
                    self.path.display(),
                    record.sequence
                )));
            }
            previous = record.sequence;
            if record.sequence > self.clean_sequence {
                self.apply(&record);
            }
        }
        self.next_sequence = previous.saturating_add(1).max(1);
        Ok(())
    }

    fn apply(&mut self, record: &ViewMutationRecord) {
        self.dirty_paths.insert(record.path.clone());
        if record.class.checkpoints() {
            self.dirty_source_paths.insert(record.path.clone());
        }
        if let Some(destination) = &record.destination {
            self.dirty_paths.insert(destination.clone());
            if record
                .destination_class
                .unwrap_or_else(|| classify_view_path(destination))
                .checkpoints()
            {
                self.dirty_source_paths.insert(destination.clone());
            }
        }
    }
}

fn clean_checkpoint_sequence(upperdir: &Path) -> Result<u64> {
    let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
    let path = layout.meta_dir.join("clean-checkpoint.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(Error::Io(err)),
    };
    let marker: serde_json::Value = serde_json::from_slice(&bytes)?;
    marker["journal_sequence"].as_u64().ok_or_else(|| {
        Error::Corrupt(format!(
            "workspace checkpoint marker `{}` has no journal sequence",
            path.display()
        ))
    })
}

fn load_source_whiteouts(layout: &ViewUpperLayout) -> Result<Vec<String>> {
    for path in [
        layout.whiteouts_path(ViewPathClass::Source),
        layout.legacy_whiteouts_path(),
    ] {
        match fs::read(path) {
            Ok(bytes) => return serde_json::from_slice(&bytes).map_err(Error::from),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_mutation_journal_replays_paths_and_ignores_truncated_tail() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        journal
            .append(ViewMutationKind::Write, "README.md", None)
            .unwrap();
        journal
            .append(ViewMutationKind::Rename, "src/old.rs", Some("src/new.rs"))
            .unwrap();
        let path = ViewUpperLayout::from_source_upper(upper.clone()).journal_path();
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(br#"{"sequence":3,"kind":"delete""#)
            .unwrap();

        let replayed = ViewMutationJournal::open(&upper).unwrap();
        assert_eq!(replayed.last_sequence(), 2);
        assert_eq!(
            replayed.dirty_paths(),
            &BTreeSet::from([
                "README.md".to_string(),
                "src/new.rs".to_string(),
                "src/old.rs".to_string(),
            ])
        );
    }

    #[test]
    fn view_mutation_journal_rejects_corrupt_complete_record() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        let path = ViewUpperLayout::from_source_upper(upper.clone()).journal_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-json\n").unwrap();

        assert!(matches!(
            ViewMutationJournal::open(&upper),
            Err(Error::Corrupt(_))
        ));
    }
}
