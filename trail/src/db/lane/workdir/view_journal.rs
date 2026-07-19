use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};

#[cfg(debug_assertions)]
std::thread_local! {
    static FAIL_NEXT_VIEW_JOURNAL_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
std::thread_local! {
    static REPLACE_NEXT_VIEW_JOURNAL_AFTER_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(debug_assertions)]
pub(crate) fn fail_next_view_journal_sync_for_current_thread() {
    FAIL_NEXT_VIEW_JOURNAL_SYNC.with(|fail| fail.set(true));
}

#[cfg(debug_assertions)]
fn fail_view_journal_sync_if_requested() -> Result<()> {
    if FAIL_NEXT_VIEW_JOURNAL_SYNC.with(|fail| fail.replace(false)) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "injected workspace view journal sync failure",
        )));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn fail_view_journal_sync_if_requested() -> Result<()> {
    Ok(())
}

#[cfg(test)]
fn replace_next_view_journal_after_open_for_current_thread() {
    REPLACE_NEXT_VIEW_JOURNAL_AFTER_OPEN.with(|replace| replace.set(true));
}

#[cfg(test)]
fn replace_view_journal_after_open_if_requested(path: &Path) -> std::io::Result<()> {
    if REPLACE_NEXT_VIEW_JOURNAL_AFTER_OPEN.with(|replace| replace.replace(false)) {
        let held = path.with_extension("aba-held");
        fs::rename(path, &held)?;
        OpenOptions::new().create_new(true).write(true).open(path)?;
    }
    Ok(())
}

#[cfg(not(test))]
fn replace_view_journal_after_open_if_requested(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

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
#[serde(rename_all = "snake_case", tag = "operation", content = "path")]
pub(crate) enum ViewWhiteoutChange {
    Insert(String),
    Remove(String),
    RemoveTree(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ViewMutationRecord {
    pub(crate) sequence: u64,
    pub(crate) generation: u64,
    pub(crate) class: ViewPathClass,
    pub(crate) kind: ViewMutationKind,
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) destination_class: Option<ViewPathClass>,
    pub(crate) whiteouts: Vec<ViewWhiteoutChange>,
    phase: ViewJournalPhase,
    previous_hash: String,
    record_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ViewWhiteoutRecord {
    sequence: u64,
    generation: u64,
    changes: Vec<ViewWhiteoutChange>,
    phase: ViewJournalPhase,
    previous_hash: String,
    record_hash: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ViewJournalPhase {
    Committed,
    Intent,
    Commit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ViewJournalState {
    version: u16,
    active_generation: u64,
    base_sequence: u64,
    mutation_base_hash: String,
    whiteout_base_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ViewJournalTailAnchor {
    version: u16,
    generation: u64,
    mutation_sequence: u64,
    mutation_hash: String,
    whiteout_sequence: u64,
    whiteout_hash: String,
}

const VIEW_JOURNAL_STATE_VERSION: u16 = 2;
const VIEW_JOURNAL_TAIL_VERSION: u16 = 1;
const VIEW_JOURNAL_GENESIS_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Append-only dirty-path index for a workspace view.
///
/// A fully replayable journal is authoritative. Missing, truncated, corrupt,
/// or gapped input is explicitly unqualified so checkpointing reconciles the
/// upper tree instead of treating an incomplete path set as clean.
pub(crate) struct ViewMutationJournal {
    upperdir: PathBuf,
    path: PathBuf,
    whiteout_path: PathBuf,
    next_sequence: u64,
    base_sequence: u64,
    clean_sequence: u64,
    generation: u64,
    dirty_paths: BTreeSet<String>,
    dirty_source_paths: BTreeSet<String>,
    dirty_generated_paths: BTreeSet<String>,
    whiteouts: BTreeSet<String>,
    recovery_whiteouts: BTreeSet<String>,
    mutation_hash: String,
    whiteout_hash: String,
    whiteout_sequence: u64,
    pending_mutations: BTreeMap<u64, ViewMutationRecord>,
    pending_whiteouts: BTreeMap<u64, ViewWhiteoutRecord>,
    qualified: bool,
    whiteouts_qualified: bool,
}

pub(crate) type ViewIntentWriter = ViewMutationJournal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewJournalCut {
    pub(crate) sequence: u64,
    pub(crate) generation: u64,
    pub(crate) qualified: bool,
    pub(crate) recovery_qualified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewJournalRecoveryState {
    pub(crate) generation: u64,
    pub(crate) base_sequence: u64,
    pub(crate) last_sequence: u64,
    pub(crate) mutation_base_hash: String,
    pub(crate) whiteout_base_hash: String,
    pub(crate) recovery_qualified: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ViewGenerationLeaseRecord {
    generation: u64,
    pid: u32,
    process_start_token: String,
}

pub(crate) struct ViewGenerationLease {
    path: PathBuf,
    generation: u64,
}

impl ViewGenerationLease {
    pub(crate) fn acquire(upperdir: &Path, generation: u64) -> Result<Self> {
        static NEXT_LEASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
        let dir = layout.meta_dir.join("active-handles");
        fs::create_dir_all(&dir)?;
        let id = NEXT_LEASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("{}-{id}.json", std::process::id()));
        let record = ViewGenerationLeaseRecord {
            generation,
            pid: std::process::id(),
            process_start_token: crate::db::util::current_process_start_token(),
        };
        write_file_atomic(&path, &serde_json::to_vec(&record)?, false)?;
        Ok(Self { path, generation })
    }

    pub(crate) fn advance(&mut self, upperdir: &Path, generation: u64) -> Result<()> {
        if self.generation == generation {
            return Ok(());
        }
        let replacement = Self::acquire(upperdir, generation)?;
        let old = std::mem::replace(self, replacement);
        drop(old);
        Ok(())
    }
}

impl Drop for ViewGenerationLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewCheckpointCandidates {
    pub(crate) journal_sequence: u64,
    pub(crate) paths: BTreeSet<String>,
    pub(crate) generated_paths: BTreeSet<String>,
    pub(crate) qualified: bool,
    pub(crate) upper_recovery_walks: u64,
}

#[cfg(test)]
pub(crate) fn recover_view_checkpoint_candidates(
    upperdir: &Path,
    lower_files: &BTreeMap<String, FileEntry>,
) -> Result<ViewCheckpointCandidates> {
    let journal = ViewMutationJournal::open(upperdir)?;
    let mut paths = journal.dirty_source_paths().clone();
    let upper_recovery_walks = if journal.is_qualified() {
        0
    } else {
        scan_source_upper(upperdir, &mut paths)?
    };
    ensure_checkpoint_recovery_qualified(upperdir, &journal)?;
    let whiteouts = journal.checkpoint_whiteouts().clone();
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
        generated_paths: journal.dirty_generated_paths().clone(),
        qualified: journal.is_qualified(),
        upper_recovery_walks,
    })
}

pub(crate) fn recover_view_checkpoint_candidates_for_root(
    db: &Trail,
    upperdir: &Path,
    root_id: &ObjectId,
) -> Result<ViewCheckpointCandidates> {
    let journal = ViewMutationJournal::open(upperdir)?;
    let mut paths = journal.dirty_source_paths().clone();
    let upper_recovery_walks = if journal.is_qualified() {
        0
    } else {
        scan_source_upper(upperdir, &mut paths)?
    };
    ensure_checkpoint_recovery_qualified(upperdir, &journal)?;
    let whiteouts = journal.checkpoint_whiteouts().clone();
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
        generated_paths: journal.dirty_generated_paths().clone(),
        qualified: journal.is_qualified(),
        upper_recovery_walks,
    })
}

fn ensure_checkpoint_recovery_qualified(
    upperdir: &Path,
    journal: &ViewMutationJournal,
) -> Result<()> {
    if journal.recovery_is_qualified() {
        return Ok(());
    }
    Err(Error::ChangeLedgerReconcileRequired {
        scope: upperdir.display().to_string(),
        state: "unqualified_view_journal".into(),
        reason: "both the changed-path journal and independent whiteout journal are missing, corrupt, or gapped; refusing a potentially false-clean checkpoint".into(),
        command: "trail ledger reconcile".into(),
    })
}

impl ViewMutationJournal {
    /// Establish the durable empty journal pair for a newly-created view.
    /// The state file is written last and is the qualification anchor: if it
    /// survives while either stream is missing, recovery knows evidence was
    /// lost and fails closed rather than mistaking the view for pristine.
    pub(crate) fn initialize_storage(upperdir: &Path) -> Result<()> {
        let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
        layout.ensure()?;
        let state_path = layout.journal_state_path();
        if state_path.exists() {
            return Ok(());
        }
        if layout.journal_path().exists() || layout.whiteout_journal_path().exists() {
            return Err(Error::Corrupt(format!(
                "workspace view journal state is missing under `{}`; reinitialize this workspace",
                layout.meta_dir.display()
            )));
        }
        for path in [layout.journal_path(), layout.whiteout_journal_path()] {
            let file = OpenOptions::new().create_new(true).write(true).open(path)?;
            file.sync_all()?;
        }
        sync_directory_strict(&layout.meta_dir)?;
        write_tail_anchor(
            &layout,
            &ViewJournalTailAnchor {
                version: VIEW_JOURNAL_TAIL_VERSION,
                generation: 0,
                mutation_sequence: 0,
                mutation_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
                whiteout_sequence: 0,
                whiteout_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
            },
        )?;
        write_file_atomic(
            &state_path,
            &serde_json::to_vec(&ViewJournalState {
                version: VIEW_JOURNAL_STATE_VERSION,
                active_generation: 0,
                base_sequence: 0,
                mutation_base_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
                whiteout_base_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
            })?,
            true,
        )?;
        sync_directory_strict(&layout.meta_dir)
    }

    pub(crate) fn open(upperdir: &Path) -> Result<Self> {
        let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
        let state = read_journal_state(&layout);
        let state_qualified = state.is_ok();
        let tail_anchor = read_tail_anchor(&layout);
        let tail_qualified = tail_anchor.is_ok();
        let state = state.unwrap_or(ViewJournalState {
            version: VIEW_JOURNAL_STATE_VERSION,
            active_generation: 0,
            base_sequence: 0,
            mutation_base_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
            whiteout_base_hash: VIEW_JOURNAL_GENESIS_HASH.to_string(),
        });
        let path = mutation_journal_path_for_generation(&layout, state.active_generation);
        let whiteout_path = whiteout_journal_path_for_generation(&layout, state.active_generation);
        let mut journal = Self {
            upperdir: upperdir.to_path_buf(),
            path,
            whiteout_path,
            next_sequence: state.base_sequence.saturating_add(1),
            base_sequence: state.base_sequence,
            clean_sequence: state.base_sequence,
            generation: state.active_generation,
            dirty_paths: BTreeSet::new(),
            dirty_source_paths: BTreeSet::new(),
            dirty_generated_paths: BTreeSet::new(),
            whiteouts: BTreeSet::new(),
            recovery_whiteouts: BTreeSet::new(),
            mutation_hash: state.mutation_base_hash,
            whiteout_hash: state.whiteout_base_hash,
            whiteout_sequence: state.base_sequence,
            pending_mutations: BTreeMap::new(),
            pending_whiteouts: BTreeMap::new(),
            qualified: state_qualified && tail_qualified,
            whiteouts_qualified: state_qualified && tail_qualified,
        };
        journal.replay()?;
        journal.replay_whiteouts()?;
        match tail_anchor {
            Ok(anchor)
                if anchor.version == VIEW_JOURNAL_TAIL_VERSION
                    && anchor.generation == journal.generation =>
            {
                if anchor.mutation_sequence != journal.last_sequence()
                    || anchor.mutation_hash != journal.mutation_hash
                {
                    journal.qualified = false;
                }
                if anchor.whiteout_sequence != journal.whiteout_sequence
                    || anchor.whiteout_hash != journal.whiteout_hash
                {
                    journal.whiteouts_qualified = false;
                }
            }
            _ => {
                journal.qualified = false;
                journal.whiteouts_qualified = false;
            }
        }
        Ok(journal)
    }

    #[cfg(test)]
    pub(crate) fn append(
        &mut self,
        kind: ViewMutationKind,
        path: &str,
        destination: Option<&str>,
    ) -> Result<u64> {
        let class = classify_view_path(path);
        let destination_class = destination.map(classify_view_path);
        self.append_classified(
            kind,
            path.to_string(),
            class,
            destination.map(str::to_string),
            destination_class,
        )
    }

    pub(crate) fn append_classified(
        &mut self,
        kind: ViewMutationKind,
        path: String,
        class: ViewPathClass,
        destination: Option<String>,
        destination_class: Option<ViewPathClass>,
    ) -> Result<u64> {
        self.append_classified_with_whiteouts(
            kind,
            path,
            class,
            destination,
            destination_class,
            Vec::new(),
        )
    }

    pub(crate) fn append_classified_with_whiteouts(
        &mut self,
        kind: ViewMutationKind,
        path: String,
        class: ViewPathClass,
        destination: Option<String>,
        destination_class: Option<ViewPathClass>,
        whiteouts: Vec<ViewWhiteoutChange>,
    ) -> Result<u64> {
        let path = normalize_relative_path(&path)?;
        let destination = destination
            .as_deref()
            .map(normalize_relative_path)
            .transpose()?;
        if matches!(kind, ViewMutationKind::Write | ViewMutationKind::Metadata)
            && self.dirty_paths.contains(&path)
            && destination.is_none()
            && whiteouts.is_empty()
        {
            return Ok(self.next_sequence.saturating_sub(1));
        }
        let phase = if whiteouts.is_empty() {
            ViewJournalPhase::Committed
        } else {
            ViewJournalPhase::Intent
        };
        let mut record = ViewMutationRecord {
            sequence: self.next_sequence,
            generation: self.generation,
            class,
            kind,
            path,
            destination,
            destination_class,
            whiteouts: whiteouts
                .into_iter()
                .map(normalize_whiteout_change)
                .collect::<Result<Vec<_>>>()?,
            phase,
            previous_hash: self.mutation_hash.clone(),
            record_hash: String::new(),
        };
        record.record_hash = mutation_record_hash(&record)?;
        fail_view_journal_sync_if_requested()?;
        append_authenticated_record(&self.path, &record)?;
        self.mutation_hash = record.record_hash.clone();
        self.apply_dirty(&record);
        if phase == ViewJournalPhase::Committed {
            self.apply_whiteouts(&record);
        } else {
            self.pending_mutations
                .insert(record.sequence, record.clone());
        }
        self.next_sequence += 1;
        self.persist_tail_anchor()?;

        let mut whiteout_record = ViewWhiteoutRecord {
            sequence: record.sequence,
            generation: record.generation,
            changes: record.whiteouts.clone(),
            phase,
            previous_hash: self.whiteout_hash.clone(),
            record_hash: String::new(),
        };
        whiteout_record.record_hash = whiteout_record_hash(&whiteout_record)?;
        if let Err(error) = append_authenticated_record(&self.whiteout_path, &whiteout_record) {
            self.whiteouts_qualified = false;
            return Err(error);
        }
        self.whiteout_hash = whiteout_record.record_hash.clone();
        self.whiteout_sequence = whiteout_record.sequence;
        if phase == ViewJournalPhase::Committed {
            apply_whiteout_changes(&mut self.recovery_whiteouts, &record.whiteouts);
        } else {
            self.pending_whiteouts
                .insert(record.sequence, whiteout_record);
        }
        self.persist_tail_anchor()?;
        Ok(record.sequence)
    }

    /// Publish the semantic whiteout half of a durable filesystem mutation.
    /// Intent records remain useful dirty-path evidence, but replay never
    /// applies their whiteouts until this matching commit is durable.
    pub(crate) fn commit_whiteouts(&mut self, sequence: u64) -> Result<()> {
        let intent = self
            .pending_mutations
            .get(&sequence)
            .cloned()
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "workspace view mutation {sequence} has no pending whiteout intent"
                ))
            })?;
        let whiteout_intent = self
            .pending_whiteouts
            .get(&sequence)
            .cloned()
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "workspace view mutation {sequence} has no independent whiteout intent"
                ))
            })?;

        let mut commit = intent.clone();
        commit.phase = ViewJournalPhase::Commit;
        commit.previous_hash = self.mutation_hash.clone();
        commit.record_hash.clear();
        commit.record_hash = mutation_record_hash(&commit)?;
        append_authenticated_record(&self.path, &commit)?;
        self.mutation_hash = commit.record_hash.clone();
        self.persist_tail_anchor()?;
        self.apply_whiteouts(&intent);
        self.pending_mutations.remove(&sequence);

        let mut whiteout_commit = whiteout_intent.clone();
        whiteout_commit.phase = ViewJournalPhase::Commit;
        whiteout_commit.previous_hash = self.whiteout_hash.clone();
        whiteout_commit.record_hash.clear();
        whiteout_commit.record_hash = whiteout_record_hash(&whiteout_commit)?;
        if let Err(error) = append_authenticated_record(&self.whiteout_path, &whiteout_commit) {
            self.whiteouts_qualified = false;
            return Err(error);
        }
        self.whiteout_hash = whiteout_commit.record_hash;
        self.whiteout_sequence = whiteout_commit.sequence;
        self.persist_tail_anchor()?;
        apply_whiteout_changes(&mut self.recovery_whiteouts, &whiteout_intent.changes);
        self.pending_whiteouts.remove(&sequence);
        Ok(())
    }

    fn persist_tail_anchor(&self) -> Result<()> {
        let layout = ViewUpperLayout::from_source_upper(self.upperdir.clone());
        write_tail_anchor(
            &layout,
            &ViewJournalTailAnchor {
                version: VIEW_JOURNAL_TAIL_VERSION,
                generation: self.generation,
                mutation_sequence: self.last_sequence(),
                mutation_hash: self.mutation_hash.clone(),
                whiteout_sequence: self.whiteout_sequence,
                whiteout_hash: self.whiteout_hash.clone(),
            },
        )
    }

    pub(crate) fn observe_checkpoint(&mut self, sequence: u64, generation: u64) -> Result<()> {
        if sequence == self.clean_sequence && generation == self.generation {
            return Ok(());
        }
        let reloaded = Self::open(&self.upperdir)?;
        if reloaded.base_sequence != sequence || reloaded.generation != generation {
            return Err(Error::Corrupt(format!(
                "workspace checkpoint cut ({sequence}, {generation}) does not match journal state ({}, {})",
                reloaded.base_sequence, reloaded.generation,
            )));
        }
        *self = reloaded;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn dirty_paths(&self) -> &BTreeSet<String> {
        &self.dirty_paths
    }

    pub(crate) fn dirty_source_paths(&self) -> &BTreeSet<String> {
        &self.dirty_source_paths
    }

    pub(crate) fn dirty_generated_paths(&self) -> &BTreeSet<String> {
        &self.dirty_generated_paths
    }

    pub(crate) fn whiteouts(&self) -> &BTreeSet<String> {
        &self.whiteouts
    }

    fn checkpoint_whiteouts(&self) -> &BTreeSet<String> {
        if self.qualified {
            &self.whiteouts
        } else {
            &self.recovery_whiteouts
        }
    }

    pub(crate) fn recovery_is_qualified(&self) -> bool {
        self.qualified || self.whiteouts_qualified
    }

    pub(crate) fn last_sequence(&self) -> u64 {
        self.next_sequence.saturating_sub(1)
    }

    pub(crate) fn cut(&self) -> ViewJournalCut {
        ViewJournalCut {
            sequence: self.last_sequence(),
            generation: self.generation,
            qualified: self.qualified,
            recovery_qualified: self.recovery_is_qualified(),
        }
    }

    pub(crate) fn is_qualified(&self) -> bool {
        self.qualified
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn rotate_after_checkpoint(
        upperdir: &Path,
        sequence: u64,
        next_generation: u64,
    ) -> Result<()> {
        let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
        let state = read_journal_state(&layout)?;
        if state.base_sequence == sequence && state.active_generation == next_generation {
            return Ok(());
        }
        let journal = Self::open(upperdir)?;
        if journal.last_sequence() != sequence || next_generation <= state.active_generation {
            return Err(Error::Corrupt(format!(
                "cannot rotate workspace journal generation {} at sequence {}",
                state.active_generation, sequence
            )));
        }
        for path in [
            mutation_journal_path_for_generation(&layout, next_generation),
            whiteout_journal_path_for_generation(&layout, next_generation),
        ] {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(file) => file.sync_all()?,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(&path)?;
                    if !metadata.file_type().is_file() || metadata.len() != 0 {
                        return Err(Error::Corrupt(format!(
                            "workspace journal generation path `{}` is unsafe or non-empty",
                            path.display()
                        )));
                    }
                }
                Err(err) => return Err(Error::Io(err)),
            }
        }
        sync_directory_strict(&layout.meta_dir)?;
        write_file_atomic(
            &layout.journal_state_path(),
            &serde_json::to_vec(&ViewJournalState {
                version: VIEW_JOURNAL_STATE_VERSION,
                active_generation: next_generation,
                base_sequence: sequence,
                mutation_base_hash: journal.mutation_hash.clone(),
                whiteout_base_hash: journal.whiteout_hash.clone(),
            })?,
            true,
        )?;
        write_tail_anchor(
            &layout,
            &ViewJournalTailAnchor {
                version: VIEW_JOURNAL_TAIL_VERSION,
                generation: next_generation,
                mutation_sequence: sequence,
                mutation_hash: journal.mutation_hash.clone(),
                whiteout_sequence: sequence,
                whiteout_hash: journal.whiteout_hash.clone(),
            },
        )?;
        sync_directory_strict(&layout.meta_dir)?;
        compact_inactive_generations(&layout, next_generation)
    }

    fn replay(&mut self) -> Result<()> {
        let bytes = match read_regular_no_follow(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.qualified = false;
                return Ok(());
            }
            Err(err) => return Err(Error::Io(err)),
        };
        let mut previous = self.base_sequence;
        let mut previous_hash = self.mutation_hash.clone();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if line.last() != Some(&b'\n') {
                self.qualified = false;
                break;
            }
            let payload = &line[..line.len() - 1];
            if payload.is_empty() {
                continue;
            }
            let mut record: ViewMutationRecord = match serde_json::from_slice(payload) {
                Ok(record) => record,
                Err(_) => {
                    self.qualified = false;
                    break;
                }
            };
            if record.generation != self.generation
                || record.previous_hash != previous_hash
                || !mutation_record_hash(&record).is_ok_and(|hash| hash == record.record_hash)
            {
                self.qualified = false;
                break;
            }
            record.path = match normalize_relative_path(&record.path) {
                Ok(path) => path,
                Err(_) => {
                    self.qualified = false;
                    break;
                }
            };
            let destination = record
                .destination
                .as_deref()
                .map(normalize_relative_path)
                .transpose();
            record.destination = match destination {
                Ok(destination) => destination,
                Err(_) => {
                    self.qualified = false;
                    break;
                }
            };
            for change in &mut record.whiteouts {
                *change = match normalize_whiteout_change(change.clone()) {
                    Ok(change) => change,
                    Err(_) => {
                        self.qualified = false;
                        break;
                    }
                };
            }
            if !self.qualified {
                break;
            }
            match record.phase {
                ViewJournalPhase::Committed => {
                    if !record.whiteouts.is_empty() || record.sequence != previous.saturating_add(1)
                    {
                        self.qualified = false;
                        break;
                    }
                    previous = record.sequence;
                    self.apply(&record);
                }
                ViewJournalPhase::Intent => {
                    if record.whiteouts.is_empty() || record.sequence != previous.saturating_add(1)
                    {
                        self.qualified = false;
                        break;
                    }
                    previous = record.sequence;
                    self.apply_dirty(&record);
                    self.pending_mutations
                        .insert(record.sequence, record.clone());
                }
                ViewJournalPhase::Commit => {
                    let Some(intent) = self.pending_mutations.remove(&record.sequence) else {
                        self.qualified = false;
                        break;
                    };
                    if !same_mutation_payload(&intent, &record) {
                        self.qualified = false;
                        break;
                    }
                    self.apply_whiteouts(&intent);
                }
            }
            previous_hash = record.record_hash;
        }
        self.next_sequence = previous.saturating_add(1).max(1);
        self.mutation_hash = previous_hash;
        Ok(())
    }

    fn replay_whiteouts(&mut self) -> Result<()> {
        let bytes = match read_regular_no_follow(&self.whiteout_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.whiteouts_qualified = false;
                return Ok(());
            }
            Err(err) => return Err(Error::Io(err)),
        };
        let mut previous = self.base_sequence;
        let mut previous_hash = self.whiteout_hash.clone();
        for line in bytes.split_inclusive(|byte| *byte == b'\n') {
            if line.last() != Some(&b'\n') {
                self.whiteouts_qualified = false;
                break;
            }
            let payload = &line[..line.len() - 1];
            if payload.is_empty() {
                continue;
            }
            let mut record: ViewWhiteoutRecord = match serde_json::from_slice(payload) {
                Ok(record) => record,
                Err(_) => {
                    self.whiteouts_qualified = false;
                    break;
                }
            };
            if record.generation != self.generation
                || record.previous_hash != previous_hash
                || !whiteout_record_hash(&record).is_ok_and(|hash| hash == record.record_hash)
            {
                self.whiteouts_qualified = false;
                break;
            }
            for change in &mut record.changes {
                *change = match normalize_whiteout_change(change.clone()) {
                    Ok(change) => change,
                    Err(_) => {
                        self.whiteouts_qualified = false;
                        break;
                    }
                };
            }
            if !self.whiteouts_qualified {
                break;
            }
            match record.phase {
                ViewJournalPhase::Committed => {
                    if !record.changes.is_empty() || record.sequence != previous.saturating_add(1) {
                        self.whiteouts_qualified = false;
                        break;
                    }
                    previous = record.sequence;
                }
                ViewJournalPhase::Intent => {
                    if record.changes.is_empty() || record.sequence != previous.saturating_add(1) {
                        self.whiteouts_qualified = false;
                        break;
                    }
                    previous = record.sequence;
                    self.pending_whiteouts
                        .insert(record.sequence, record.clone());
                }
                ViewJournalPhase::Commit => {
                    let Some(intent) = self.pending_whiteouts.remove(&record.sequence) else {
                        self.whiteouts_qualified = false;
                        break;
                    };
                    if !same_whiteout_payload(&intent, &record) {
                        self.whiteouts_qualified = false;
                        break;
                    }
                    apply_whiteout_changes(&mut self.recovery_whiteouts, &intent.changes);
                }
            }
            previous_hash = record.record_hash;
        }
        self.whiteout_hash = previous_hash;
        self.whiteout_sequence = previous;
        if !self.pending_whiteouts.is_empty() {
            // An intent without a commit is deliberately not independent
            // whiteout authority. The mutation stream still supplies safe
            // dirty-path evidence and can qualify recovery on its own.
            self.whiteouts_qualified = false;
        }
        if self.whiteouts_qualified && previous != self.last_sequence() {
            // Each mutation has a matching independent whiteout record. The
            // longer complete stream is evidence that the shorter stream lost
            // a valid suffix, even when that suffix ended on a record boundary.
            if previous > self.last_sequence() {
                self.qualified = false;
            } else {
                self.whiteouts_qualified = false;
            }
        }
        Ok(())
    }

    fn apply(&mut self, record: &ViewMutationRecord) {
        self.apply_whiteouts(record);
        self.apply_dirty(record);
    }

    fn apply_dirty(&mut self, record: &ViewMutationRecord) {
        self.dirty_paths.insert(record.path.clone());
        if record.class.checkpoints() {
            self.dirty_source_paths.insert(record.path.clone());
        } else if matches!(
            record.class,
            ViewPathClass::Dependency | ViewPathClass::Generated
        ) {
            self.dirty_generated_paths.insert(record.path.clone());
        }
        if let Some(destination) = &record.destination {
            self.dirty_paths.insert(destination.clone());
            let destination_class = record
                .destination_class
                .unwrap_or_else(|| classify_view_path(destination));
            if destination_class.checkpoints() {
                self.dirty_source_paths.insert(destination.clone());
            } else if matches!(
                destination_class,
                ViewPathClass::Dependency | ViewPathClass::Generated
            ) {
                self.dirty_generated_paths.insert(destination.clone());
            }
        }
    }

    fn apply_whiteouts(&mut self, record: &ViewMutationRecord) {
        for change in &record.whiteouts {
            match change {
                ViewWhiteoutChange::Insert(path) => {
                    self.whiteouts.insert(path.clone());
                }
                ViewWhiteoutChange::Remove(path) => {
                    self.whiteouts.remove(path);
                }
                ViewWhiteoutChange::RemoveTree(path) => {
                    let prefix = format!("{path}/");
                    self.whiteouts
                        .retain(|item| item != path && !item.starts_with(&prefix));
                }
            }
        }
    }
}

pub(crate) fn recovery_state(upperdir: &Path) -> Result<ViewJournalRecoveryState> {
    let (state, journal) = authenticated_recovery_journal(upperdir)?;
    Ok(ViewJournalRecoveryState {
        generation: state.active_generation,
        base_sequence: state.base_sequence,
        last_sequence: journal.last_sequence(),
        mutation_base_hash: state.mutation_base_hash,
        whiteout_base_hash: state.whiteout_base_hash,
        recovery_qualified: true,
    })
}

pub(crate) fn authenticated_recovery_cut_hashes(
    upperdir: &Path,
    sequence: u64,
) -> Result<(String, String)> {
    let (state, journal) = authenticated_recovery_journal(upperdir)?;
    if sequence == state.base_sequence {
        return Ok((state.mutation_base_hash, state.whiteout_base_hash));
    }
    if sequence == journal.last_sequence() {
        return Ok((journal.mutation_hash, journal.whiteout_hash));
    }
    Err(Error::Corrupt(format!(
        "workspace journal cannot authenticate recovery cut {sequence} in generation {}",
        state.active_generation
    )))
}

fn authenticated_recovery_journal(
    upperdir: &Path,
) -> Result<(ViewJournalState, ViewMutationJournal)> {
    let layout = ViewUpperLayout::from_source_upper(upperdir.to_path_buf());
    let state = read_journal_state(&layout)?;
    let journal = ViewMutationJournal::open(upperdir)?;
    if journal.generation != state.active_generation
        || journal.base_sequence != state.base_sequence
        || !journal.qualified
        || !journal.whiteouts_qualified
    {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: upperdir.display().to_string(),
            state: "unqualified_view_journal_recovery".into(),
            reason: "workspace checkpoint recovery requires authenticated state, mutation and whiteout journals, and tail identity".into(),
            command: "trail ledger reconcile".into(),
        });
    }
    Ok((state, journal))
}

fn mutation_record_hash(record: &ViewMutationRecord) -> Result<String> {
    let mut authenticated = record.clone();
    authenticated.record_hash.clear();
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(
        &authenticated,
    )?)))
}

fn whiteout_record_hash(record: &ViewWhiteoutRecord) -> Result<String> {
    let mut authenticated = record.clone();
    authenticated.record_hash.clear();
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(
        &authenticated,
    )?)))
}

fn same_mutation_payload(intent: &ViewMutationRecord, commit: &ViewMutationRecord) -> bool {
    intent.sequence == commit.sequence
        && intent.generation == commit.generation
        && intent.class == commit.class
        && intent.kind == commit.kind
        && intent.path == commit.path
        && intent.destination == commit.destination
        && intent.destination_class == commit.destination_class
        && intent.whiteouts == commit.whiteouts
        && intent.phase == ViewJournalPhase::Intent
        && commit.phase == ViewJournalPhase::Commit
}

fn same_whiteout_payload(intent: &ViewWhiteoutRecord, commit: &ViewWhiteoutRecord) -> bool {
    intent.sequence == commit.sequence
        && intent.generation == commit.generation
        && intent.changes == commit.changes
        && intent.phase == ViewJournalPhase::Intent
        && commit.phase == ViewJournalPhase::Commit
}

fn append_authenticated_record<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        if !fs::symlink_metadata(parent)?.file_type().is_dir() {
            return Err(Error::Corrupt(format!(
                "workspace journal parent `{}` is not a real directory",
                parent.display()
            )));
        }
    }
    let mut encoded = serde_json::to_vec(record)?;
    encoded.push(b'\n');
    let mut file = open_regular_no_follow(path, true, true)?;
    replace_view_journal_after_open_if_requested(path)?;
    file.write_all(&encoded)?;
    file.sync_all()?;
    validate_regular_file_identity(path, &file)?;
    if let Some(parent) = path.parent() {
        sync_directory_strict(parent)?;
    }
    validate_regular_file_identity(path, &file)?;
    Ok(())
}

fn read_regular_no_follow(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = open_regular_no_follow(path, false, false)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn open_regular_no_follow(path: &Path, append: bool, create: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(!append)
        .write(append)
        .append(append)
        .create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "workspace journal is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "workspace journal has an unsafe hard-link count",
            ));
        }
    }
    Ok(file)
}

fn validate_regular_file_identity(path: &Path, file: &File) -> std::io::Result<()> {
    let path_metadata = fs::symlink_metadata(path)?;
    let held = file.metadata()?;
    if !path_metadata.file_type().is_file() || !held.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "workspace journal pathname was replaced",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if held.nlink() != 1
            || path_metadata.nlink() != 1
            || held.dev() != path_metadata.dev()
            || held.ino() != path_metadata.ino()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "workspace journal pathname changed while appending",
            ));
        }
    }
    Ok(())
}

fn journal_tail_anchor_path(layout: &ViewUpperLayout) -> PathBuf {
    layout.meta_dir.join("journal-tail.json")
}

fn read_tail_anchor(layout: &ViewUpperLayout) -> Result<ViewJournalTailAnchor> {
    let anchor: ViewJournalTailAnchor =
        serde_json::from_slice(&read_regular_no_follow(&journal_tail_anchor_path(layout))?)?;
    if anchor.version != VIEW_JOURNAL_TAIL_VERSION {
        return Err(Error::Corrupt(format!(
            "unsupported workspace view journal tail version {}",
            anchor.version
        )));
    }
    Ok(anchor)
}

fn write_tail_anchor(layout: &ViewUpperLayout, anchor: &ViewJournalTailAnchor) -> Result<()> {
    write_file_atomic(
        &journal_tail_anchor_path(layout),
        &serde_json::to_vec(anchor)?,
        true,
    )?;
    sync_directory_strict(&layout.meta_dir)
}

fn read_journal_state(layout: &ViewUpperLayout) -> Result<ViewJournalState> {
    let state: ViewJournalState =
        serde_json::from_slice(&read_regular_no_follow(&layout.journal_state_path())?)?;
    if state.version != VIEW_JOURNAL_STATE_VERSION {
        return Err(Error::Corrupt(format!(
            "unsupported workspace view journal state version {}",
            state.version
        )));
    }
    Ok(state)
}

fn mutation_journal_path_for_generation(layout: &ViewUpperLayout, generation: u64) -> PathBuf {
    if generation == 0 {
        layout.journal_path()
    } else {
        layout
            .meta_dir
            .join(format!("mutation-journal.g{generation}.jsonl"))
    }
}

fn whiteout_journal_path_for_generation(layout: &ViewUpperLayout, generation: u64) -> PathBuf {
    if generation == 0 {
        layout.whiteout_journal_path()
    } else {
        layout
            .meta_dir
            .join(format!("whiteout-journal.g{generation}.jsonl"))
    }
}

fn compact_inactive_generations(layout: &ViewUpperLayout, active_generation: u64) -> Result<()> {
    let leases_dir = layout.meta_dir.join("active-handles");
    let mut retained = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(&leases_dir) {
        for entry in entries {
            let entry = entry?;
            let record = fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ViewGenerationLeaseRecord>(&bytes).ok());
            match record {
                Some(record)
                    if crate::db::util::process_matches_start_token(
                        record.pid,
                        &record.process_start_token,
                    ) =>
                {
                    retained.insert(record.generation);
                }
                _ => {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
    // Retain one prior generation for post-crash inspection. Everything
    // older is bounded unless an active adapter still owns that generation.
    retained.insert(active_generation);
    retained.insert(active_generation.saturating_sub(1));
    for generation in 0..active_generation.saturating_sub(1) {
        if retained.contains(&generation) {
            continue;
        }
        for path in [
            mutation_journal_path_for_generation(layout, generation),
            whiteout_journal_path_for_generation(layout, generation),
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::Io(err)),
            }
        }
    }
    sync_directory_strict(&layout.meta_dir)
}

fn apply_whiteout_changes(whiteouts: &mut BTreeSet<String>, changes: &[ViewWhiteoutChange]) {
    for change in changes {
        match change {
            ViewWhiteoutChange::Insert(path) => {
                whiteouts.insert(path.clone());
            }
            ViewWhiteoutChange::Remove(path) => {
                whiteouts.remove(path);
            }
            ViewWhiteoutChange::RemoveTree(path) => {
                let prefix = format!("{path}/");
                whiteouts.retain(|item| item != path && !item.starts_with(&prefix));
            }
        }
    }
}

fn normalize_whiteout_change(change: ViewWhiteoutChange) -> Result<ViewWhiteoutChange> {
    Ok(match change {
        ViewWhiteoutChange::Insert(path) => {
            ViewWhiteoutChange::Insert(normalize_relative_path(&path)?)
        }
        ViewWhiteoutChange::Remove(path) => {
            ViewWhiteoutChange::Remove(normalize_relative_path(&path)?)
        }
        ViewWhiteoutChange::RemoveTree(path) => {
            ViewWhiteoutChange::RemoveTree(normalize_relative_path(&path)?)
        }
    })
}

fn scan_source_upper(upperdir: &Path, paths: &mut BTreeSet<String>) -> Result<u64> {
    let mut walks = 0_u64;
    for entry in walkdir::WalkDir::new(upperdir) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        walks = walks.saturating_add(1);
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
    Ok(walks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_mutation_journal_replays_paths_and_ignores_truncated_tail() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
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
        assert!(!replayed.is_qualified());
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
    fn view_mutation_journal_marks_corrupt_complete_record_unqualified() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let path = ViewUpperLayout::from_source_upper(upper.clone()).journal_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not-json\n").unwrap();

        let journal = ViewMutationJournal::open(&upper).unwrap();
        assert!(!journal.is_qualified());
        assert_eq!(journal.last_sequence(), 0);
    }

    #[test]
    fn authenticated_record_rejects_valid_json_path_and_generation_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        journal
            .append(ViewMutationKind::Write, "changed.txt", None)
            .unwrap();
        let path = ViewUpperLayout::from_source_upper(upper.clone()).journal_path();
        let mut record: serde_json::Value = serde_json::from_slice(
            fs::read(&path)
                .unwrap()
                .split(|byte| *byte == b'\n')
                .next()
                .unwrap(),
        )
        .unwrap();
        record["path"] = serde_json::Value::String("innocent.txt".into());
        record["generation"] = serde_json::Value::from(99_u64);
        let mut bytes = serde_json::to_vec(&record).unwrap();
        bytes.push(b'\n');
        fs::write(path, bytes).unwrap();

        let replayed = ViewMutationJournal::open(&upper).unwrap();
        assert!(!replayed.is_qualified());
        assert!(!replayed.dirty_paths().contains("innocent.txt"));
    }

    #[test]
    fn tail_anchor_rejects_correlated_valid_boundary_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        journal
            .append(ViewMutationKind::Write, "changed.txt", None)
            .unwrap();
        drop(journal);

        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        fs::write(layout.journal_path(), b"").unwrap();
        fs::write(layout.whiteout_journal_path(), b"").unwrap();

        let replayed = ViewMutationJournal::open(&upper).unwrap();
        assert!(!replayed.is_qualified());
        assert!(!replayed.recovery_is_qualified());
        assert!(recover_view_checkpoint_candidates(&upper, &BTreeMap::new()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn journal_append_rejects_post_open_pathname_aba() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        replace_next_view_journal_after_open_for_current_thread();

        assert!(journal
            .append(ViewMutationKind::Write, "changed.txt", None)
            .is_err());

        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        assert_eq!(fs::read(layout.journal_path()).unwrap(), b"");
        let anchor = read_tail_anchor(&layout).unwrap();
        assert_eq!(anchor.mutation_sequence, 0);
        assert_eq!(anchor.whiteout_sequence, 0);
    }

    #[test]
    fn uncommitted_whiteout_intent_is_dirty_evidence_but_not_view_state() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Rename,
                "old.txt".into(),
                ViewPathClass::Source,
                Some("new.txt".into()),
                Some(ViewPathClass::Source),
                vec![
                    ViewWhiteoutChange::Insert("old.txt".into()),
                    ViewWhiteoutChange::RemoveTree("new.txt".into()),
                ],
            )
            .unwrap();
        drop(journal); // crash after intent sync, before the filesystem rename

        let replayed = ViewMutationJournal::open(&upper).unwrap();
        assert!(replayed.is_qualified());
        assert!(replayed.dirty_paths().contains("old.txt"));
        assert!(replayed.dirty_paths().contains("new.txt"));
        assert!(!replayed.whiteouts().contains("old.txt"));
    }

    #[test]
    fn authenticated_journal_separates_generated_dirty_paths_without_upper_walk() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        journal
            .append_classified(
                ViewMutationKind::Write,
                "README.md".into(),
                ViewPathClass::Source,
                None,
                None,
            )
            .unwrap();
        journal
            .append_classified(
                ViewMutationKind::Write,
                "target/debug/app".into(),
                ViewPathClass::Generated,
                None,
                None,
            )
            .unwrap();
        journal
            .append_classified(
                ViewMutationKind::Write,
                "node_modules/pkg/index.js".into(),
                ViewPathClass::Dependency,
                None,
                None,
            )
            .unwrap();
        drop(journal);

        let candidates = recover_view_checkpoint_candidates(&upper, &BTreeMap::new()).unwrap();
        assert_eq!(candidates.paths, BTreeSet::from(["README.md".into()]));
        assert_eq!(
            candidates.generated_paths,
            BTreeSet::from([
                "node_modules/pkg/index.js".into(),
                "target/debug/app".into()
            ])
        );
        assert_eq!(candidates.upper_recovery_walks, 0);
    }

    #[cfg(unix)]
    #[test]
    fn journal_append_refuses_symlink_leaf_without_touching_target() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        let path = ViewUpperLayout::from_source_upper(upper).journal_path();
        let victim = temp.path().join("victim.txt");
        fs::write(&victim, b"preserve me").unwrap();
        fs::remove_file(&path).unwrap();
        symlink(&victim, &path).unwrap();

        assert!(journal
            .append(ViewMutationKind::Write, "changed.txt", None)
            .is_err());
        assert_eq!(fs::read(victim).unwrap(), b"preserve me");
    }

    #[test]
    fn journal_generations_rotate_with_global_sequences_and_bounded_growth() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();

        for generation in 1..=8_u64 {
            let mut journal = ViewMutationJournal::open(&upper).unwrap();
            assert_eq!(journal.generation(), generation - 1);
            let sequence = journal
                .append(ViewMutationKind::Write, &format!("file-{generation}"), None)
                .unwrap();
            assert_eq!(sequence, generation);
            ViewMutationJournal::rotate_after_checkpoint(&upper, sequence, generation).unwrap();
        }

        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        let journal_files = fs::read_dir(&layout.meta_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains("journal.g"))
            .count();
        assert!(
            journal_files <= 4,
            "inactive generations were not compacted"
        );
        let reopened = ViewMutationJournal::open(&upper).unwrap();
        assert_eq!(reopened.generation(), 8);
        assert_eq!(reopened.last_sequence(), 8);
        assert!(reopened.is_qualified());
    }

    #[test]
    fn active_generation_lease_retains_old_journals_until_handle_closes() {
        let temp = tempfile::tempdir().unwrap();
        let upper = temp.path().join("upper");
        ViewMutationJournal::initialize_storage(&upper).unwrap();
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        let lease = ViewGenerationLease::acquire(&upper, 0).unwrap();

        for generation in 1..=3_u64 {
            let mut journal = ViewMutationJournal::open(&upper).unwrap();
            let sequence = journal
                .append(
                    ViewMutationKind::Write,
                    &format!("leased-{generation}"),
                    None,
                )
                .unwrap();
            ViewMutationJournal::rotate_after_checkpoint(&upper, sequence, generation).unwrap();
        }
        assert!(mutation_journal_path_for_generation(&layout, 0).exists());

        drop(lease);
        let mut journal = ViewMutationJournal::open(&upper).unwrap();
        let sequence = journal
            .append(ViewMutationKind::Write, "after-close", None)
            .unwrap();
        ViewMutationJournal::rotate_after_checkpoint(&upper, sequence, 4).unwrap();
        assert!(!mutation_journal_path_for_generation(&layout, 0).exists());
        assert!(!whiteout_journal_path_for_generation(&layout, 0).exists());
    }
}
