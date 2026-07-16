use super::*;
use crate::db::core::backup::publication::{
    atomic_exchange, publish_staged_tree, remove_any, sync_directory_strict,
};
use std::fs::File;

const RESTORE_MARKER: &str = ".trail-restore-transaction.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum RestorePhase {
    Prepared,
    PolicyPublished,
    RollingBack,
    Finalizing,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct RestoreMarker {
    format_version: u32,
    phase: RestorePhase,
    db_stage_leaf: String,
    policy_stage_leaf: String,
    had_old_db: bool,
    had_old_policy: bool,
    new_db_sha256: String,
    new_policy_sha256: String,
    old_policy_sha256: Option<String>,
}

pub(super) struct RestorePublication {
    workspace_root: PathBuf,
    marker: RestoreMarker,
}

impl RestorePublication {
    pub(super) fn prepare(
        workspace_root: &Path,
        db_stage: &Path,
        backup_path: &Path,
        force: bool,
    ) -> Result<(Self, bool)> {
        let policy_target = workspace_root.join(".trailignore");
        let backup_policy = backup_path.join(".trailignore");
        let restored_policy = backup_policy.is_file() && (force || !policy_target.exists());
        let policy_stage = allocate_policy_stage(workspace_root)?;
        if restored_policy {
            fs::copy(&backup_policy, &policy_stage)?;
        } else if policy_target.is_file() {
            fs::copy(&policy_target, &policy_stage)?;
        } else {
            fs::write(
                &policy_stage,
                format!("{}\n", DEFAULT_CRABIGNORE_PATTERNS.join("\n")),
            )?;
        }
        OpenOptions::new()
            .read(true)
            .open(&policy_stage)?
            .sync_all()?;
        sync_directory_strict(workspace_root)?;

        let db_stage_leaf = confined_leaf(db_stage, workspace_root, "restore DB stage")?;
        let policy_stage_leaf =
            confined_leaf(&policy_stage, workspace_root, "restore policy stage")?;
        let marker = RestoreMarker {
            format_version: 1,
            phase: RestorePhase::Prepared,
            db_stage_leaf,
            policy_stage_leaf,
            had_old_db: workspace_root.join(".trail").exists(),
            had_old_policy: policy_target.exists(),
            new_db_sha256: digest_file(&db_stage.join(DB_RELATIVE_PATH))?,
            new_policy_sha256: digest_file(&policy_stage)?,
            old_policy_sha256: policy_target
                .is_file()
                .then(|| digest_file(&policy_target))
                .transpose()?,
        };
        write_marker(workspace_root, &marker)?;
        test_crash_point("restore_after_policy_staging");
        Ok((
            Self {
                workspace_root: workspace_root.to_path_buf(),
                marker,
            },
            restored_policy,
        ))
    }

    pub(super) fn publish(mut self) -> Result<()> {
        let policy_stage = self.path(&self.marker.policy_stage_leaf);
        let policy_target = self.workspace_root.join(".trailignore");
        if let Err(error) = publish_policy_entry(&policy_stage, &policy_target) {
            let _ = self.rollback();
            return Err(error);
        }
        test_crash_point("restore_after_policy_exchange_before_marker");
        self.set_phase(RestorePhase::PolicyPublished)?;
        test_crash_point("restore_after_policy_publication");

        #[cfg(test)]
        if std::env::var_os("TRAIL_TEST_RESTORE_FORCE_ROLLBACK").is_some() {
            self.set_phase(RestorePhase::RollingBack)?;
            test_crash_point("restore_during_rollback");
            self.rollback()?;
            return Err(Error::Conflict("forced restore rollback".into()));
        }

        let db_stage = self.path(&self.marker.db_stage_leaf);
        let db_target = self.workspace_root.join(".trail");
        if let Err(error) = publish_staged_tree(&db_stage, &db_target) {
            self.set_phase(RestorePhase::RollingBack)?;
            test_crash_point("restore_during_rollback");
            self.rollback()?;
            return Err(error);
        }
        self.set_phase(RestorePhase::Finalizing)?;
        test_crash_point("restore_after_trail_publication");
        test_crash_point("restore_during_finalization");
        test_crash_point("restore_before_retained_cleanup");
        finish_new_pair(&self.workspace_root, &self.marker)?;
        test_crash_point("restore_after_retained_cleanup");
        Ok(())
    }

    fn set_phase(&mut self, phase: RestorePhase) -> Result<()> {
        self.marker.phase = phase;
        write_marker(&self.workspace_root, &self.marker)
    }

    fn rollback(&self) -> Result<()> {
        restore_old_pair(&self.workspace_root, &self.marker)
    }

    fn path(&self, leaf: &str) -> PathBuf {
        self.workspace_root.join(leaf)
    }
}

pub(crate) fn recover_restore_publication(workspace_root: &Path) -> Result<()> {
    let marker_path = workspace_root.join(RESTORE_MARKER);
    let bytes = match fs::read(&marker_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let marker: RestoreMarker = serde_json::from_slice(&bytes)?;
    validate_marker(&marker)?;
    match marker.phase {
        RestorePhase::Prepared | RestorePhase::RollingBack => {
            restore_old_pair(workspace_root, &marker)
        }
        RestorePhase::PolicyPublished => {
            if db_target_is_new(workspace_root, &marker)? {
                finish_new_pair(workspace_root, &marker)
            } else {
                restore_old_pair(workspace_root, &marker)
            }
        }
        RestorePhase::Finalizing => finish_new_pair(workspace_root, &marker),
    }
}

fn restore_old_pair(workspace_root: &Path, marker: &RestoreMarker) -> Result<()> {
    restore_old_entry(
        workspace_root,
        ".trailignore",
        &marker.policy_stage_leaf,
        marker.had_old_policy,
        marker.old_policy_sha256.as_deref(),
        &marker.new_policy_sha256,
    )?;
    let db_target = workspace_root.join(".trail");
    let db_stage = workspace_root.join(&marker.db_stage_leaf);
    if db_target_is_new(workspace_root, marker)? {
        if marker.had_old_db {
            exchange_required(&db_stage, &db_target, "restore DB rollback")?;
        } else {
            remove_any(&db_target)?;
        }
    }
    remove_any(&db_stage)?;
    remove_any(&workspace_root.join(&marker.policy_stage_leaf))?;
    remove_marker(workspace_root)
}

fn finish_new_pair(workspace_root: &Path, marker: &RestoreMarker) -> Result<()> {
    if !db_target_is_new(workspace_root, marker)? {
        return Err(Error::Corrupt(
            "restore transaction cannot finalize without the staged DB generation".into(),
        ));
    }
    let policy_target = workspace_root.join(".trailignore");
    if digest_optional(&policy_target)?.as_deref() != Some(marker.new_policy_sha256.as_str()) {
        let policy_stage = workspace_root.join(&marker.policy_stage_leaf);
        if digest_optional(&policy_stage)?.as_deref() != Some(marker.new_policy_sha256.as_str()) {
            return Err(Error::Corrupt(
                "restore transaction lost the staged policy generation".into(),
            ));
        }
        publish_policy_entry(&policy_stage, &policy_target)?;
    }
    remove_any(&workspace_root.join(&marker.db_stage_leaf))?;
    remove_any(&workspace_root.join(&marker.policy_stage_leaf))?;
    remove_marker(workspace_root)
}

fn restore_old_entry(
    workspace_root: &Path,
    target_leaf: &str,
    stage_leaf: &str,
    had_old: bool,
    old_digest: Option<&str>,
    new_digest: &str,
) -> Result<()> {
    let target = workspace_root.join(target_leaf);
    let stage = workspace_root.join(stage_leaf);
    if had_old {
        if digest_optional(&target)?.as_deref() == old_digest {
            return Ok(());
        }
        if digest_optional(&stage)?.as_deref() != old_digest {
            return Err(Error::Corrupt(format!(
                "restore transaction lost retained `{target_leaf}`"
            )));
        }
        if target.exists() {
            exchange_required(&stage, &target, "restore policy rollback")?;
        } else {
            fs::rename(&stage, &target)?;
            sync_directory_strict(workspace_root)?;
        }
    } else if digest_optional(&target)?.as_deref() == Some(new_digest) {
        remove_any(&target)?;
        sync_directory_strict(workspace_root)?;
    }
    Ok(())
}

fn publish_policy_entry(stage: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidInput("restore policy has no parent".into()))?;
    OpenOptions::new().read(true).open(stage)?.sync_all()?;
    sync_directory_strict(parent)?;
    if target.exists() {
        exchange_required(stage, target, "restore policy publication")?;
    } else {
        fs::rename(stage, target)?;
        sync_directory_strict(parent)?;
    }
    Ok(())
}

fn exchange_required(left: &Path, right: &Path, label: &str) -> Result<()> {
    if !atomic_exchange(left, right)? {
        return Err(Error::Conflict(format!(
            "atomic exchange is unsupported for {label}; live entries were not moved"
        )));
    }
    let parent = right
        .parent()
        .ok_or_else(|| Error::InvalidInput(format!("{label} target has no parent")))?;
    sync_directory_strict(parent)
}

fn db_target_is_new(workspace_root: &Path, marker: &RestoreMarker) -> Result<bool> {
    Ok(
        digest_optional(&workspace_root.join(".trail").join(DB_RELATIVE_PATH))?.as_deref()
            == Some(marker.new_db_sha256.as_str()),
    )
}

fn write_marker(workspace_root: &Path, marker: &RestoreMarker) -> Result<()> {
    validate_marker(marker)?;
    let marker_path = workspace_root.join(RESTORE_MARKER);
    let temporary = workspace_root.join(format!(".{RESTORE_MARKER}.tmp-{}", now_nanos()));
    fs::write(&temporary, serde_json::to_vec(marker)?)?;
    OpenOptions::new().read(true).open(&temporary)?.sync_all()?;
    fs::rename(&temporary, &marker_path)?;
    sync_directory_strict(workspace_root)
}

fn remove_marker(workspace_root: &Path) -> Result<()> {
    remove_any(&workspace_root.join(RESTORE_MARKER))?;
    sync_directory_strict(workspace_root)
}

fn validate_marker(marker: &RestoreMarker) -> Result<()> {
    if marker.format_version != 1 {
        return Err(Error::Corrupt(
            "unsupported restore transaction marker".into(),
        ));
    }
    validate_marker_leaf(&marker.db_stage_leaf)?;
    validate_marker_leaf(&marker.policy_stage_leaf)?;
    if marker.db_stage_leaf == marker.policy_stage_leaf
        || marker.new_db_sha256.len() != 64
        || marker.new_policy_sha256.len() != 64
        || marker
            .old_policy_sha256
            .as_ref()
            .is_some_and(|hash| hash.len() != 64)
    {
        return Err(Error::Corrupt("invalid restore transaction marker".into()));
    }
    Ok(())
}

fn validate_marker_leaf(leaf: &str) -> Result<()> {
    let mut components = Path::new(leaf).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) || leaf.contains(['/', '\0'])
    {
        return Err(Error::Corrupt(format!(
            "restore transaction path is not confined: `{leaf}`"
        )));
    }
    Ok(())
}

fn confined_leaf(path: &Path, parent: &Path, label: &str) -> Result<String> {
    if path.parent() != Some(parent) {
        return Err(Error::InvalidInput(format!("{label} is not a sibling")));
    }
    let leaf = path
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| Error::InvalidInput(format!("{label} has no UTF-8 leaf")))?
        .to_string();
    validate_marker_leaf(&leaf)?;
    Ok(leaf)
}

fn allocate_policy_stage(workspace_root: &Path) -> Result<PathBuf> {
    for _ in 0..32 {
        let path = workspace_root.join(format!("..trailignore.restore-stage-{}", now_nanos()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => {
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Conflict(
        "could not allocate restore policy stage".into(),
    ))
}

fn digest_optional(path: &Path) -> Result<Option<String>> {
    match digest_file(path) {
        Ok(digest) => Ok(Some(digest)),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn digest_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}
