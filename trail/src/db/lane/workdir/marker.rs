use super::*;

use crate::db::change_ledger::secure_fs::SecureDirectory;
use crate::db::change_ledger::{EvidenceCut, EvidenceSource, ScopeId};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const MARKER_FILE: &str = "workdir-manifest.json";
const MAX_MARKER_BYTES: u64 = 16 * 1024;
const SPARSE_SELECTION_FILE: &str = "sparse-selection.json";
const MAX_SPARSE_SELECTION_BYTES: u64 = 1024 * 1024;

pub(crate) const MATERIALIZED_LANE_MARKER_VERSION: u16 = 2;

/// Compact authority marker. It deliberately contains no path manifest: the
/// changed-path ledger is the only scalable candidate authority.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MaterializedLaneMarkerV2 {
    pub(crate) version: u16,
    pub(crate) scope_id: ScopeId,
    pub(crate) filesystem_identity: Vec<u8>,
    pub(crate) ref_name: String,
    pub(crate) ref_generation: u64,
    pub(crate) root_id: ObjectId,
    pub(crate) policy_fingerprint: [u8; 32],
    pub(crate) epoch: u64,
    pub(crate) provider_cut: EvidenceCut,
    pub(crate) provider_segment_id: String,
    pub(crate) sparse_selection_fingerprint: [u8; 32],
}

impl Trail {
    pub(crate) fn materialized_lane_sparse_selection_fingerprint(
        &self,
        workdir: &Path,
    ) -> Result<[u8; 32]> {
        actual_sparse_selection_fingerprint(workdir)
    }

    pub(crate) fn authenticated_lane_sparse_selection(
        &self,
        workdir: &Path,
    ) -> Result<Option<Vec<String>>> {
        let bytes = secure_marker_directory(workdir, false)?
            .map(|metadata| {
                metadata.read_regular_optional_bounded(
                    SPARSE_SELECTION_FILE,
                    MAX_SPARSE_SELECTION_BYTES,
                )
            })
            .transpose()?
            .flatten();
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        Ok(Some(parse_sparse_selection_paths(&bytes)?))
    }

    pub(crate) fn capture_materialized_lane_marker(
        &self,
        workdir: &Path,
    ) -> Result<Option<Vec<u8>>> {
        let Some(metadata) = secure_marker_directory(workdir, false)? else {
            return Ok(None);
        };
        metadata.read_regular_optional_bounded(MARKER_FILE, MAX_MARKER_BYTES)
    }

    pub(crate) fn restore_materialized_lane_marker(
        &self,
        workdir: &Path,
        bytes: Option<&[u8]>,
    ) -> Result<()> {
        match bytes {
            Some(bytes) => secure_marker_directory(workdir, true)?
                .ok_or_else(|| Error::Corrupt("lane metadata directory disappeared".into()))?
                .write_atomic_regular(MARKER_FILE, bytes),
            None => self.invalidate_materialized_lane_marker(workdir),
        }
    }

    pub(crate) fn publish_lane_marker_if_materialized(&self, lane: &str) -> Result<()> {
        // Until command authority is platform-qualified, the existing V1
        // clean-workdir manifest remains the only cache. Never replace it with
        // an epoch-zero V2 marker that could look like ledger authority.
        let branch = self.lane_branch(lane)?;
        let Some(workdir) = branch.workdir.as_deref() else {
            return Ok(());
        };
        let workdir = Path::new(workdir);
        if !workdir.is_dir() {
            return Ok(());
        }
        let head = self.get_ref(&branch.ref_name)?;
        let sparse_selection_fingerprint = actual_sparse_selection_fingerprint(workdir)?;
        self.publish_materialized_lane_marker_v2(
            &branch,
            workdir,
            &head,
            sparse_selection_fingerprint,
        )
    }

    pub(crate) fn publish_lane_marker_for_sparse_selection(
        &self,
        lane: &str,
        sparse_selection_fingerprint: [u8; 32],
    ) -> Result<()> {
        let branch = self.lane_branch(lane)?;
        let Some(workdir) = branch.workdir.as_deref() else {
            return Ok(());
        };
        let workdir = Path::new(workdir);
        if !workdir.is_dir() {
            return Ok(());
        }
        let head = self.get_ref(&branch.ref_name)?;
        self.publish_materialized_lane_marker_v2(
            &branch,
            workdir,
            &head,
            sparse_selection_fingerprint,
        )
    }

    pub(crate) fn invalidate_lane_marker_if_materialized(&self, branch: &LaneBranch) -> Result<()> {
        if let Some(workdir) = branch.workdir.as_deref() {
            let workdir = Path::new(workdir);
            if workdir.is_dir() {
                self.invalidate_materialized_lane_marker(workdir)?;
            }
        }
        Ok(())
    }

    pub(crate) fn publish_materialized_lane_marker_v2(
        &self,
        branch: &LaneBranch,
        workdir: &Path,
        head: &RefRecord,
        sparse_selection_fingerprint: [u8; 32],
    ) -> Result<()> {
        if branch.ref_name != head.name
            || branch.head_change != head.change_id
            || branch.head_root != head.root_id
            || branch.workdir.as_deref() != Some(workdir.to_string_lossy().as_ref())
        {
            return Err(Error::StaleBranch(branch.ref_name.clone()));
        }
        let scope_id = crate::db::change_ledger::materialized_lane_scope_id(
            &self.config.workspace.id.0,
            &branch.lane_id,
        );
        let runtime_cut =
            crate::db::change_ledger::materialized_lane_daemon_marker_cut(self, &branch.lane_id)?;
        let scope = self
            .conn
            .query_row(
                "SELECT scope.epoch,scope.durable_offset,scope.folded_offset,
                        scope.filesystem_identity,scope.policy_fingerprint
                 FROM changed_path_scopes scope
                 WHERE scope.scope_id=?1 AND scope.scope_kind='materialized_lane' AND scope.owner_id=?2
                   AND ref_name=?3 AND ref_generation=?4 AND baseline_root_id=?5
                   AND retired_at IS NULL",
                params![
                    scope_id.to_text(),
                    branch.lane_id,
                    head.name,
                    head.generation,
                    head.root_id.0,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        // Before a qualified lane daemon owns the scope the marker remains an
        // explicit reconciliation marker, never clean authority.
        let (epoch, provider_cut, provider_segment_id, filesystem_identity, policy_fingerprint) =
            match (scope, runtime_cut) {
                (
                    Some((epoch, durable, folded, filesystem_identity, policy_fingerprint)),
                    Some((runtime_cut, segment_id)),
                ) => (
                    u64::try_from(epoch)
                        .map_err(|_| Error::Corrupt("negative lane epoch".into()))?,
                    if runtime_cut.durable_offset
                        == u64::try_from(durable)
                            .map_err(|_| Error::Corrupt("negative lane durable cut".into()))?
                        && runtime_cut.folded_offset
                            == u64::try_from(folded)
                                .map_err(|_| Error::Corrupt("negative lane folded cut".into()))?
                    {
                        runtime_cut
                    } else {
                        return Err(Error::ChangeLedgerReconcileRequired {
                            scope: scope_id.to_text(),
                            state: "untrusted_gap".into(),
                            reason: "lane marker runtime cut does not match persisted scope".into(),
                            command: format!("trail lane status {}", branch.lane_id),
                        });
                    },
                    segment_id,
                    hex::decode(filesystem_identity)
                        .map_err(|_| Error::Corrupt("invalid lane filesystem identity".into()))?,
                    decode_marker_fingerprint(&policy_fingerprint)?,
                ),
                // No live qualified runtime means there is no V2 authority to
                // publish. Preserve any legacy cache in authority-off mode; a
                // later exact runtime reconciliation will replace it atomically.
                _ => return Ok(()),
            };
        let marker = MaterializedLaneMarkerV2 {
            version: MATERIALIZED_LANE_MARKER_VERSION,
            scope_id,
            filesystem_identity,
            ref_name: head.name.clone(),
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative lane ref generation".into()))?,
            root_id: head.root_id.clone(),
            policy_fingerprint,
            epoch,
            provider_cut,
            provider_segment_id,
            sparse_selection_fingerprint,
        };
        let metadata = secure_marker_directory(workdir, true)?.ok_or_else(|| {
            Error::Corrupt("materialized lane metadata directory disappeared".into())
        })?;
        metadata.write_atomic_regular(MARKER_FILE, &serde_json::to_vec(&marker)?)
    }

    pub(crate) fn invalidate_materialized_lane_marker(&self, workdir: &Path) -> Result<()> {
        if let Some(metadata) = secure_marker_directory(workdir, false)? {
            metadata.remove_leaf(MARKER_FILE)?;
        }
        Ok(())
    }

    /// Return a marker only when its full binding still matches a trusted,
    /// qualified materialized-lane scope. Every other case is reconciliation,
    /// including absent/v1/future markers and owner/provider loss.
    pub(crate) fn validated_materialized_lane_marker_v2(
        &self,
        branch: &LaneBranch,
        workdir: &Path,
        head: &RefRecord,
    ) -> Result<Option<MaterializedLaneMarkerV2>> {
        let Some(metadata) = secure_marker_directory(workdir, false)? else {
            return Ok(None);
        };
        let Some(bytes) = metadata.read_regular_optional_bounded(MARKER_FILE, MAX_MARKER_BYTES)?
        else {
            return Ok(None);
        };
        let marker = match serde_json::from_slice::<MaterializedLaneMarkerV2>(&bytes) {
            Ok(marker) if marker.version == MATERIALIZED_LANE_MARKER_VERSION => marker,
            _ => {
                metadata.remove_leaf(MARKER_FILE)?;
                return Ok(None);
            }
        };
        let generation = u64::try_from(head.generation)
            .map_err(|_| Error::Corrupt("negative lane ref generation".into()))?;
        let expected_scope = crate::db::change_ledger::materialized_lane_scope_id(
            &self.config.workspace.id.0,
            &branch.lane_id,
        );
        let structural = marker.scope_id == expected_scope
            && marker.filesystem_identity == materialized_lane_root_identity(workdir)?
            && marker.ref_name == head.name
            && marker.ref_generation == generation
            && marker.root_id == head.root_id
            && marker.sparse_selection_fingerprint == actual_sparse_selection_fingerprint(workdir)?
            && marker.epoch > 0
            && marker.provider_cut.source == EvidenceSource::Observer
            && marker.provider_cut.durable_offset == marker.provider_cut.folded_offset
            && !marker.provider_segment_id.is_empty();
        if !structural {
            metadata.remove_leaf(MARKER_FILE)?;
            return Ok(None);
        }
        let qualified: bool = self.conn.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM changed_path_scopes scope
               JOIN changed_path_observer_owners owner
                 ON owner.scope_id=scope.scope_id AND owner.epoch=scope.epoch
               JOIN changed_path_observer_segments segment
                 ON segment.scope_id=scope.scope_id AND segment.epoch=scope.epoch
                AND segment.segment_id=?10
               WHERE scope.scope_id=?1 AND scope.scope_kind='materialized_lane' AND scope.owner_id=?2
                 AND scope.epoch=?3 AND scope.ref_name=?4 AND scope.ref_generation=?5
                 AND scope.baseline_root_id=?6 AND scope.policy_fingerprint=?7
                 AND scope.filesystem_identity=?8 AND scope.trust_state='trusted'
                 AND scope.clean_proof_allowed=1 AND scope.linearizable_fence=1
                 AND scope.filesystem_supported=1 AND scope.power_loss_durability=1
                 AND scope.durable_offset=?9 AND scope.folded_offset=?9
                 AND owner.lease_state='active' AND owner.error_state IS NULL
                 AND owner.error_at IS NULL AND owner.expires_at>?12
                 AND owner.fence_nonce IS NOT NULL
                 AND owner.provider_id=scope.provider_id
                 AND owner.provider_identity=scope.provider_identity
                 AND segment.owner_token=owner.owner_token
                 AND segment.provider_id=owner.provider_id
                 AND segment.first_sequence<=?11 AND segment.last_sequence>=?11
                 AND segment.durable_end_offset>=?9 AND segment.folded_end_offset=?9
                 AND segment.state IN ('open','sealed')
                 AND scope.retired_at IS NULL)",
            params![
                marker.scope_id.to_text(),
                branch.lane_id,
                i64::try_from(marker.epoch).map_err(|_| Error::InvalidInput("lane epoch overflow".into()))?,
                marker.ref_name,
                i64::try_from(marker.ref_generation).map_err(|_| Error::InvalidInput("lane ref generation overflow".into()))?,
                marker.root_id.0,
                hex::encode(marker.policy_fingerprint),
                hex::encode(&marker.filesystem_identity),
                i64::try_from(marker.provider_cut.durable_offset).map_err(|_| Error::InvalidInput("lane provider cut overflow".into()))?,
                marker.provider_segment_id,
                i64::try_from(marker.provider_cut.sequence).map_err(|_| Error::InvalidInput("lane provider sequence overflow".into()))?,
                now_ts(),
            ],
            |row| row.get(0),
        )?;
        if !qualified {
            return Ok(None);
        }
        Ok(Some(marker))
    }
}

fn materialized_lane_root_identity(workdir: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(workdir)?;
        let metadata = file.metadata()?;
        return Ok(format!(
            "root-v1:dev={};ino={};mode={};uid={};gid={}",
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.uid(),
            metadata.gid()
        )
        .into_bytes());
    }
    #[cfg(not(unix))]
    {
        let canonical = workdir.canonicalize()?;
        Ok(format!("root-v1:path={}", canonical.display()).into_bytes())
    }
}

fn actual_sparse_selection_fingerprint(workdir: &Path) -> Result<[u8; 32]> {
    let mut digest = Sha256::new();
    digest.update(b"trail-sparse-selection-v2\0");
    let bytes = secure_marker_directory(workdir, false)?
        .map(|metadata| {
            metadata
                .read_regular_optional_bounded(SPARSE_SELECTION_FILE, MAX_SPARSE_SELECTION_BYTES)
        })
        .transpose()?
        .flatten();
    match bytes {
        Some(bytes) => {
            for path in parse_sparse_selection_paths(&bytes)? {
                digest.update(path.as_bytes());
                digest.update([0]);
            }
        }
        None => digest.update(b"full"),
    }
    Ok(digest.finalize().into())
}

fn parse_sparse_selection_paths(bytes: &[u8]) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let raw_paths = value.get("materialized_paths").ok_or_else(|| {
        Error::Corrupt(
            "invalid sparse selection: required `materialized_paths` field is missing".into(),
        )
    })?;
    let raw_paths = raw_paths.as_array().ok_or_else(|| {
        Error::Corrupt("invalid sparse selection: `materialized_paths` must be an array".into())
    })?;
    let mut paths = raw_paths
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = value.as_str().ok_or_else(|| {
                Error::Corrupt(format!(
                    "invalid sparse selection: `materialized_paths[{index}]` must be a string"
                ))
            })?;
            normalize_relative_path(path)
        })
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn decode_marker_fingerprint(encoded: &str) -> Result<[u8; 32]> {
    hex::decode(encoded)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| Error::Corrupt("invalid materialized lane policy fingerprint".into()))
}

fn secure_marker_directory(workdir: &Path, create: bool) -> Result<Option<SecureDirectory>> {
    let root = SecureDirectory::open_absolute(workdir)?;
    match root.open_dir(".trail") {
        Ok(directory) => {
            directory.restrict_private()?;
            Ok(Some(directory))
        }
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound && create => {
            root.create_private_dir(".trail").map(Some)
        }
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
