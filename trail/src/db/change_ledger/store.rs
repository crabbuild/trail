use rusqlite::{params, Connection, OptionalExtension};

use super::types::*;
use crate::db::util::now_ts;
use crate::error::{Error, Result};

pub(crate) struct ChangedPathLedger<'a> {
    pub(super) conn: &'a Connection,
}

#[derive(Debug)]
struct ScopeRow {
    state: TrustState,
    reason: String,
    durable_offset: u64,
    folded_offset: u64,
    max_candidate_rows: u64,
    max_prefix_rows: u64,
}

impl<'a> ChangedPathLedger<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub(crate) fn begin_scope(
        &self,
        identity: &ScopeIdentity,
        baseline: &BaselineIdentity,
        policy: &PolicyIdentity,
        filesystem: &FilesystemIdentity,
        provider: &ProviderIdentity,
    ) -> Result<ScopeId> {
        if identity.owner_id.is_empty() {
            return Err(Error::InvalidInput(
                "changed-path scope owner cannot be empty".into(),
            ));
        }
        let now = now_ts();
        let scope_id = identity.scope_id.to_text();
        let filesystem_identity = hex::encode(&filesystem.0);
        let provider_identity = hex::encode(&provider.identity);
        let policy_fingerprint = hex::encode(policy.fingerprint);
        self.conn.execute(
            "INSERT INTO changed_path_scopes(
                 scope_id, schema_version, scope_kind, owner_id,
                 scope_root, scope_root_identity, filesystem_identity,
                 filesystem_kind, case_sensitive, ref_name, ref_generation,
                 change_id, baseline_root_id, policy_fingerprint,
                 policy_dependency_generation, trust_state, trust_reason, epoch,
                 provider_id, provider_identity, durable_cursor,
                 linearizable_fence, rename_pairing, overflow_scope,
                 filesystem_supported, clean_proof_allowed,
                 power_loss_durability, created_at, updated_at
             ) VALUES(
                 ?1, 1, ?2, ?3, '', ?4, ?4, 'unknown', 1, ?5, ?6, ?7, ?8,
                 ?9, ?10, 'untrusted_gap', 'initial_reconciliation_required', 1,
                 ?11, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?19
             )",
            params![
                scope_id,
                identity.kind.as_str(),
                identity.owner_id,
                filesystem_identity,
                baseline.ref_name,
                sql_u64(baseline.ref_generation, "ref generation")?,
                baseline.change_id.0,
                baseline.root_id.0,
                policy_fingerprint,
                sql_u64(policy.generation, "policy generation")?,
                provider_identity,
                bool_sql(provider.capabilities.durable_cursor),
                bool_sql(provider.capabilities.linearizable_fence),
                bool_sql(provider.capabilities.rename_pairing),
                bool_sql(provider.capabilities.overflow_scope),
                bool_sql(provider.capabilities.filesystem_supported),
                bool_sql(provider.capabilities.clean_proof_allowed),
                bool_sql(provider.capabilities.power_loss_durability),
                now,
            ],
        )?;
        Ok(identity.scope_id)
    }

    pub(crate) fn mark_prefix_dirty(
        &self,
        expected: &ExpectedScope,
        prefix: &DirtyPrefix,
    ) -> Result<()> {
        if !prefix.complete {
            return Err(Error::InvalidInput(
                "incomplete dirty prefixes cannot be persisted as authoritative evidence".into(),
            ));
        }
        if prefix.reason.is_empty() {
            return Err(Error::InvalidInput(
                "dirty prefix completeness reason cannot be empty".into(),
            ));
        }
        if prefix.first_sequence > prefix.last_sequence {
            return Err(Error::InvalidInput(
                "dirty prefix first sequence exceeds last sequence".into(),
            ));
        }

        let mut tx = self.conn.unchecked_transaction()?;
        let scope = evidence_write_guard(&tx, expected)?;
        let mut savepoint = tx.savepoint()?;
        let scope_id = expected.scope_id.to_text();
        let incoming = prefix.path.as_str();
        let mut statement = savepoint.prepare(
            "SELECT normalized_prefix, completeness_reason, source_mask,
                    first_sequence, last_sequence
             FROM changed_path_prefixes
             WHERE scope_id = ?1
               AND (
                    normalized_prefix = ?2 COLLATE BINARY
                    OR (
                        normalized_prefix >= (?2 || '/') COLLATE BINARY
                        AND normalized_prefix < (?2 || '0') COLLATE BINARY
                    )
                    OR (
                        ?2 >= (normalized_prefix || '/') COLLATE BINARY
                        AND ?2 < (normalized_prefix || '0') COLLATE BINARY
                    )
               )
             ORDER BY normalized_prefix COLLATE BINARY",
        )?;
        let overlaps = statement
            .query_map(params![scope_id, incoming], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);

        let ancestor = overlaps
            .iter()
            .filter(|(path, ..)| component_prefix(path, incoming))
            .min_by_key(|(path, ..)| path.len());
        let (merged_path, merged_reason) = ancestor
            .map(|(path, reason, ..)| (path.clone(), reason.clone()))
            .unwrap_or_else(|| (incoming.to_string(), prefix.reason.clone()));
        let mut first_sequence = prefix.first_sequence;
        let mut last_sequence = prefix.last_sequence;
        let mut source_mask = EvidenceSource::Reconciliation.mask();
        for row in &overlaps {
            first_sequence = first_sequence.min(db_u64(row.3, "prefix first sequence")?);
            last_sequence = last_sequence.max(db_u64(row.4, "prefix last sequence")?);
            source_mask |= row.2;
        }
        for (path, ..) in &overlaps {
            savepoint.execute(
                "DELETE FROM changed_path_prefixes
                 WHERE scope_id = ?1 AND normalized_prefix = ?2 COLLATE BINARY",
                params![scope_id, path],
            )?;
        }
        let now = now_ts();
        savepoint.execute(
            "INSERT INTO changed_path_prefixes(
                 scope_id, normalized_prefix, completeness_reason, event_flags,
                 source_mask, first_sequence, last_sequence, provider_id,
                 provider_sequence, intent_id, created_at, updated_at
             ) VALUES(?1, ?2, ?3, 0, ?4, ?5, ?6, 'reconciliation', NULL, NULL, ?7, ?7)",
            params![
                scope_id,
                merged_path,
                merged_reason,
                source_mask,
                sql_u64(first_sequence, "prefix first sequence")?,
                sql_u64(last_sequence, "prefix last sequence")?,
                now,
            ],
        )?;
        let overflowed = caps_exceeded(&savepoint, &scope, &scope_id)?;
        if overflowed {
            savepoint.rollback()?;
        }
        savepoint.commit()?;
        if overflowed {
            mark_scope_overflow(&tx, expected)?;
            tx.commit()?;
            return Err(reconcile_error(
                expected,
                TrustState::Overflow,
                "persisted evidence cap exceeded",
            ));
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn mark_untrusted(
        &self,
        expected: &ExpectedScope,
        state: TrustState,
        reason: &str,
    ) -> Result<()> {
        if state == TrustState::Trusted {
            return Err(Error::InvalidInput(
                "mark_untrusted cannot promote a scope to trusted".into(),
            ));
        }
        let encoded = EncodedExpected::new(expected)?;
        let changed = self.conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = ?1, trust_reason = ?2,
                 continuity_generation = continuity_generation + 1, updated_at = ?3
             WHERE scope_id = ?4 AND epoch = ?5 AND ref_name = ?6
               AND ref_generation = ?7 AND baseline_root_id = ?8
               AND policy_fingerprint = ?9
               AND policy_dependency_generation = ?10
               AND filesystem_identity = ?11 AND provider_identity = ?12",
            params![
                state.as_str(),
                reason,
                now_ts(),
                encoded.scope_id,
                encoded.epoch,
                expected.ref_name,
                encoded.ref_generation,
                expected.baseline_root.0,
                encoded.policy_fingerprint,
                encoded.policy_generation,
                encoded.filesystem_identity,
                encoded.provider_identity,
            ],
        )?;
        if changed == 0 {
            return Err(stale_cas_error(self.conn, expected));
        }
        Ok(())
    }

    pub(crate) fn snapshot_candidates(
        &self,
        expected: &ExpectedScope,
    ) -> Result<CandidateSnapshot> {
        self.snapshot_candidates_in_transaction(expected, || {})
    }

    #[cfg(test)]
    fn snapshot_candidates_with_phase_hook<F>(
        &self,
        expected: &ExpectedScope,
        after_scope_read: F,
    ) -> Result<CandidateSnapshot>
    where
        F: FnOnce(),
    {
        self.snapshot_candidates_in_transaction(expected, after_scope_read)
    }

    fn snapshot_candidates_in_transaction<F>(
        &self,
        expected: &ExpectedScope,
        after_scope_read: F,
    ) -> Result<CandidateSnapshot>
    where
        F: FnOnce(),
    {
        let tx = self.conn.unchecked_transaction()?;
        let scope = load_scope(&tx, expected)?;
        if scope.state != TrustState::Trusted {
            return Err(reconcile_error(expected, scope.state, &scope.reason));
        }
        if scope.durable_offset != scope.folded_offset {
            return Err(reconcile_error(
                expected,
                TrustState::UntrustedGap,
                "durable observer evidence is not fully folded",
            ));
        }
        after_scope_read();
        let scope_id = expected.scope_id.to_text();
        let exact_count = row_count(&tx, "changed_path_entries", &scope_id)?;
        let prefix_count = row_count(&tx, "changed_path_prefixes", &scope_id)?;
        if exact_count > scope.max_candidate_rows || prefix_count > scope.max_prefix_rows {
            return Err(reconcile_error(
                expected,
                TrustState::Overflow,
                "persisted evidence cap exceeded",
            ));
        }

        let exact_path_values = tx
            .prepare(
                "SELECT normalized_path FROM changed_path_entries
                 WHERE scope_id = ?1 ORDER BY normalized_path COLLATE BINARY",
            )?
            .query_map([&scope_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let exact_paths = exact_path_values
            .iter()
            .map(|path| LedgerPath::parse(path))
            .collect::<Result<Vec<_>>>()?;

        let prefix_values = tx
            .prepare(
                "SELECT normalized_prefix, completeness_reason,
                        first_sequence, last_sequence
                 FROM changed_path_prefixes
                 WHERE scope_id = ?1 ORDER BY normalized_prefix COLLATE BINARY",
            )?
            .query_map([&scope_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let prefixes = prefix_values
            .into_iter()
            .map(|(path, reason, first, last)| {
                Ok(DirtyPrefix {
                    path: LedgerPath::parse(&path)?,
                    complete: true,
                    reason,
                    first_sequence: db_u64(first, "prefix first sequence")?,
                    last_sequence: db_u64(last, "prefix last sequence")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let sequence = tx.query_row(
            "SELECT COALESCE(MAX(provider_sequence), 0)
             FROM (
                 SELECT provider_sequence FROM changed_path_entries
                 WHERE scope_id = ?1 AND (source_mask & ?2) != 0
                   AND provider_sequence IS NOT NULL
                 UNION ALL
                 SELECT provider_sequence FROM changed_path_prefixes
                 WHERE scope_id = ?1 AND (source_mask & ?2) != 0
                   AND provider_sequence IS NOT NULL
             )",
            params![scope_id, EvidenceSource::Observer.mask()],
            |row| row.get::<_, i64>(0),
        )?;
        let snapshot = CandidateSnapshot {
            expected: expected.clone(),
            cut: EvidenceCut {
                source: EvidenceSource::Observer,
                sequence: db_u64(sequence, "observer sequence")?,
                durable_offset: scope.durable_offset,
                folded_offset: scope.folded_offset,
            },
            exact_paths,
            prefixes,
            trust: TrustState::Trusted,
        };
        tx.commit()?;
        Ok(snapshot)
    }

    pub(crate) fn acknowledge(
        &self,
        expected: &ExpectedScope,
        cut: &EvidenceCut,
        owned: &OwnedEvidence,
    ) -> Result<()> {
        if cut.source != owned.source {
            return Err(Error::InvalidInput(
                "evidence cut and ownership source must match".into(),
            ));
        }
        if owned.prefixes.iter().any(|prefix| !prefix.complete) {
            return Err(Error::InvalidInput(
                "incomplete prefixes cannot be acknowledged as authoritative".into(),
            ));
        }
        let through = cut.sequence.min(owned.through_sequence);
        let tx = self.conn.unchecked_transaction()?;
        cas_guard(&tx, expected, true)?;
        let scope = load_scope(&tx, expected)?;
        validate_cut_boundaries(expected, &scope, cut)?;
        let scope_id = expected.scope_id.to_text();
        for path in &owned.exact_paths {
            tx.execute(
                "DELETE FROM changed_path_entries
                 WHERE scope_id = ?1 AND normalized_path = ?2 COLLATE BINARY
                   AND source_mask = ?3
                   AND (
                       (?3 = ?5 AND provider_sequence IS NOT NULL
                           AND provider_sequence <= ?4)
                       OR (?3 != ?5 AND last_sequence <= ?4)
                   )",
                params![
                    scope_id,
                    path.as_str(),
                    owned.source.mask(),
                    sql_u64(through, "acknowledgement sequence")?,
                    EvidenceSource::Observer.mask(),
                ],
            )?;
        }
        for prefix in &owned.prefixes {
            tx.execute(
                "DELETE FROM changed_path_prefixes
                 WHERE scope_id = ?1 AND normalized_prefix = ?2 COLLATE BINARY
                   AND source_mask = ?3
                   AND (
                       (?3 = ?5 AND provider_sequence IS NOT NULL
                           AND provider_sequence <= ?4)
                       OR (?3 != ?5 AND last_sequence <= ?4)
                   )",
                params![
                    scope_id,
                    prefix.path.as_str(),
                    owned.source.mask(),
                    sql_u64(through, "acknowledgement sequence")?,
                    EvidenceSource::Observer.mask(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn advance_baseline(
        &self,
        expected: &ExpectedScope,
        target: &BaselineIdentity,
        cut: &EvidenceCut,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        cas_guard(&tx, expected, true)?;
        let scope = load_scope(&tx, expected)?;
        validate_cut_boundaries(expected, &scope, cut)?;
        let encoded = EncodedExpected::new(expected)?;
        let changed = tx.execute(
            "UPDATE changed_path_scopes
             SET ref_name = ?1, ref_generation = ?2, change_id = ?3,
                 baseline_root_id = ?4, updated_at = ?5
             WHERE scope_id = ?6 AND epoch = ?7 AND ref_name = ?8
               AND ref_generation = ?9 AND baseline_root_id = ?10
               AND policy_fingerprint = ?11
               AND policy_dependency_generation = ?12
               AND filesystem_identity = ?13 AND provider_identity = ?14
               AND trust_state = 'trusted'",
            params![
                target.ref_name,
                sql_u64(target.ref_generation, "target ref generation")?,
                target.change_id.0,
                target.root_id.0,
                now_ts(),
                encoded.scope_id,
                encoded.epoch,
                expected.ref_name,
                encoded.ref_generation,
                expected.baseline_root.0,
                encoded.policy_fingerprint,
                encoded.policy_generation,
                encoded.filesystem_identity,
                encoded.provider_identity,
            ],
        )?;
        if changed == 0 {
            return Err(stale_cas_error(&tx, expected));
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn upsert_exact(
        &self,
        expected: &ExpectedScope,
        path: &LedgerPath,
        flags: EvidenceFlags,
        source: EvidenceSource,
        sequence: u64,
    ) -> Result<()> {
        let mut tx = self.conn.unchecked_transaction()?;
        let scope = evidence_write_guard(&tx, expected)?;
        let mut savepoint = tx.savepoint()?;
        let now = now_ts();
        let sequence = sql_u64(sequence, "source sequence")?;
        let provider_sequence = (source == EvidenceSource::Observer).then_some(sequence);
        savepoint.execute(
            "INSERT INTO changed_path_entries(
                 scope_id, normalized_path, event_flags, source_mask,
                 first_sequence, last_sequence, provider_id,
                 provider_sequence, intent_id, created_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, NULL, ?8, ?8)
             ON CONFLICT(scope_id, normalized_path) DO UPDATE SET
                 event_flags = changed_path_entries.event_flags | excluded.event_flags,
                 source_mask = changed_path_entries.source_mask | excluded.source_mask,
                 first_sequence = MIN(changed_path_entries.first_sequence, excluded.first_sequence),
                 last_sequence = MAX(changed_path_entries.last_sequence, excluded.last_sequence),
                 provider_id = CASE
                     WHEN changed_path_entries.source_mask = excluded.source_mask
                     THEN excluded.provider_id ELSE NULL END,
                 provider_sequence = CASE
                     WHEN excluded.provider_sequence IS NULL
                     THEN changed_path_entries.provider_sequence
                     WHEN changed_path_entries.provider_sequence IS NULL
                     THEN excluded.provider_sequence
                     ELSE MAX(changed_path_entries.provider_sequence, excluded.provider_sequence)
                 END,
                 updated_at = excluded.updated_at",
            params![
                expected.scope_id.to_text(),
                path.as_str(),
                flags.0,
                source.mask(),
                sequence,
                source.as_str(),
                provider_sequence,
                now,
            ],
        )?;
        let scope_id = expected.scope_id.to_text();
        let overflowed = caps_exceeded(&savepoint, &scope, &scope_id)?;
        if overflowed {
            savepoint.rollback()?;
        }
        savepoint.commit()?;
        if overflowed {
            mark_scope_overflow(&tx, expected)?;
            tx.commit()?;
            return Err(reconcile_error(
                expected,
                TrustState::Overflow,
                "persisted evidence cap exceeded",
            ));
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn all_exact(&self, expected: &ExpectedScope) -> Result<Vec<ExactEvidence>> {
        load_scope(self.conn, expected)?;
        let values = self
            .conn
            .prepare(
                "SELECT normalized_path, event_flags, source_mask,
                        first_sequence, last_sequence
                 FROM changed_path_entries
                 WHERE scope_id = ?1 ORDER BY normalized_path COLLATE BINARY",
            )?
            .query_map([expected.scope_id.to_text()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        values
            .into_iter()
            .map(|(path, flags, source_mask, first, last)| {
                Ok(ExactEvidence {
                    path: LedgerPath::parse(&path)?,
                    flags: EvidenceFlags(flags),
                    source_mask,
                    first_sequence: db_u64(first, "entry first sequence")?,
                    last_sequence: db_u64(last, "entry last sequence")?,
                })
            })
            .collect()
    }
}

struct EncodedExpected {
    scope_id: String,
    epoch: i64,
    ref_generation: i64,
    policy_fingerprint: String,
    policy_generation: i64,
    filesystem_identity: String,
    provider_identity: String,
}

impl EncodedExpected {
    fn new(expected: &ExpectedScope) -> Result<Self> {
        Ok(Self {
            scope_id: expected.scope_id.to_text(),
            epoch: sql_u64(expected.epoch, "scope epoch")?,
            ref_generation: sql_u64(expected.ref_generation, "ref generation")?,
            policy_fingerprint: hex::encode(expected.policy_fingerprint),
            policy_generation: sql_u64(expected.policy_generation, "policy generation")?,
            filesystem_identity: hex::encode(&expected.filesystem_identity),
            provider_identity: hex::encode(&expected.provider_identity),
        })
    }
}

fn cas_guard(conn: &Connection, expected: &ExpectedScope, require_trusted: bool) -> Result<()> {
    let encoded = EncodedExpected::new(expected)?;
    let changed = conn.execute(
        "UPDATE changed_path_scopes SET updated_at = updated_at
         WHERE scope_id = ?1 AND epoch = ?2 AND ref_name = ?3
           AND ref_generation = ?4 AND baseline_root_id = ?5
           AND policy_fingerprint = ?6 AND policy_dependency_generation = ?7
           AND filesystem_identity = ?8 AND provider_identity = ?9
           AND (?10 = 0 OR trust_state = 'trusted')",
        params![
            encoded.scope_id,
            encoded.epoch,
            expected.ref_name,
            encoded.ref_generation,
            expected.baseline_root.0,
            encoded.policy_fingerprint,
            encoded.policy_generation,
            encoded.filesystem_identity,
            encoded.provider_identity,
            bool_sql(require_trusted),
        ],
    )?;
    if changed == 0 {
        return Err(stale_cas_error(conn, expected));
    }
    Ok(())
}

fn load_scope(conn: &Connection, expected: &ExpectedScope) -> Result<ScopeRow> {
    let encoded = EncodedExpected::new(expected)?;
    let row = conn
        .query_row(
            "SELECT trust_state, trust_reason, durable_offset, folded_offset,
                    max_candidate_rows, max_prefix_rows
             FROM changed_path_scopes
             WHERE scope_id = ?1 AND epoch = ?2 AND ref_name = ?3
               AND ref_generation = ?4 AND baseline_root_id = ?5
               AND policy_fingerprint = ?6 AND policy_dependency_generation = ?7
               AND filesystem_identity = ?8 AND provider_identity = ?9",
            params![
                encoded.scope_id,
                encoded.epoch,
                expected.ref_name,
                encoded.ref_generation,
                expected.baseline_root.0,
                encoded.policy_fingerprint,
                encoded.policy_generation,
                encoded.filesystem_identity,
                encoded.provider_identity,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((state, reason, durable_offset, folded_offset, max_candidate_rows, max_prefix_rows)) =
        row
    else {
        return Err(stale_cas_error(conn, expected));
    };
    Ok(ScopeRow {
        state: TrustState::parse(&state)?,
        reason,
        durable_offset: db_u64(durable_offset, "durable offset")?,
        folded_offset: db_u64(folded_offset, "folded offset")?,
        max_candidate_rows: db_u64(max_candidate_rows, "candidate row cap")?,
        max_prefix_rows: db_u64(max_prefix_rows, "prefix row cap")?,
    })
}

fn evidence_write_guard(conn: &Connection, expected: &ExpectedScope) -> Result<ScopeRow> {
    cas_guard(conn, expected, false)?;
    let scope = load_scope(conn, expected)?;
    if matches!(scope.state, TrustState::Overflow | TrustState::Corrupt) {
        return Err(reconcile_error(expected, scope.state, &scope.reason));
    }
    Ok(scope)
}

fn caps_exceeded(conn: &Connection, scope: &ScopeRow, scope_id: &str) -> Result<bool> {
    let candidates = row_count(conn, "changed_path_entries", scope_id)?;
    let prefixes = row_count(conn, "changed_path_prefixes", scope_id)?;
    Ok(candidates > scope.max_candidate_rows || prefixes > scope.max_prefix_rows)
}

fn mark_scope_overflow(conn: &Connection, expected: &ExpectedScope) -> Result<()> {
    let encoded = EncodedExpected::new(expected)?;
    let changed = conn.execute(
        "UPDATE changed_path_scopes
         SET trust_state = 'overflow', trust_reason = 'persisted evidence cap exceeded',
             continuity_generation = continuity_generation + 1, updated_at = ?1
         WHERE scope_id = ?2 AND epoch = ?3 AND ref_name = ?4
           AND ref_generation = ?5 AND baseline_root_id = ?6
           AND policy_fingerprint = ?7 AND policy_dependency_generation = ?8
           AND filesystem_identity = ?9 AND provider_identity = ?10",
        params![
            now_ts(),
            encoded.scope_id,
            encoded.epoch,
            expected.ref_name,
            encoded.ref_generation,
            expected.baseline_root.0,
            encoded.policy_fingerprint,
            encoded.policy_generation,
            encoded.filesystem_identity,
            encoded.provider_identity,
        ],
    )?;
    if changed == 0 {
        return Err(stale_cas_error(conn, expected));
    }
    Ok(())
}

fn validate_cut_boundaries(
    expected: &ExpectedScope,
    scope: &ScopeRow,
    cut: &EvidenceCut,
) -> Result<()> {
    if cut.durable_offset != scope.durable_offset || cut.folded_offset != scope.folded_offset {
        return Err(reconcile_error(
            expected,
            TrustState::UntrustedGap,
            "evidence cut does not match the persisted observer boundaries",
        ));
    }
    Ok(())
}

fn row_count(conn: &Connection, table: &str, scope_id: &str) -> Result<u64> {
    let sql = match table {
        "changed_path_entries" => "SELECT COUNT(*) FROM changed_path_entries WHERE scope_id = ?1",
        "changed_path_prefixes" => "SELECT COUNT(*) FROM changed_path_prefixes WHERE scope_id = ?1",
        _ => return Err(Error::Corrupt("unknown changed-path row table".into())),
    };
    let count = conn.query_row(sql, [scope_id], |row| row.get::<_, i64>(0))?;
    db_u64(count, "evidence row count")
}

fn component_prefix(prefix: &str, path: &str) -> bool {
    prefix == path
        || path
            .strip_prefix(prefix)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn stale_cas_error(conn: &Connection, expected: &ExpectedScope) -> Error {
    let observed = conn
        .query_row(
            "SELECT trust_state, trust_reason FROM changed_path_scopes WHERE scope_id = ?1",
            [expected.scope_id.to_text()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let (state, reason) = observed.unwrap_or_else(|| {
        (
            "missing".into(),
            "scope is missing or its exact identity changed".into(),
        )
    });
    reconcile_error(
        expected,
        TrustState::parse(&state).unwrap_or(TrustState::UntrustedGap),
        &format!("stale scope CAS: {reason}"),
    )
}

fn reconcile_error(expected: &ExpectedScope, state: TrustState, reason: &str) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: expected.scope_id.to_text(),
        state: state.as_str().into(),
        reason: reason.into(),
        command: "trail status".into(),
    }
}

fn bool_sql(value: bool) -> i64 {
    i64::from(value)
}

fn sql_u64(value: u64, label: &str) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("{label} exceeds SQLite INTEGER range")))
}

fn db_u64(value: i64, label: &str) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| Error::Corrupt(format!("{label} is negative")))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rusqlite::{params, Connection};

    use super::*;
    use crate::db::{InitImportMode, Trail};
    use crate::error::Error;
    use crate::{ChangeId, ObjectId};

    const FIXTURE_SCHEMA: &str = "
        PRAGMA foreign_keys = ON;
        CREATE TABLE changed_path_scopes (
            scope_id TEXT NOT NULL PRIMARY KEY,
            schema_version INTEGER NOT NULL DEFAULT 1,
            scope_kind TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            scope_root TEXT NOT NULL,
            scope_root_identity TEXT NOT NULL,
            filesystem_identity TEXT NOT NULL,
            filesystem_kind TEXT NOT NULL,
            case_sensitive INTEGER NOT NULL,
            ref_name TEXT NOT NULL,
            ref_generation INTEGER NOT NULL,
            change_id TEXT NOT NULL,
            baseline_root_id TEXT NOT NULL,
            policy_fingerprint TEXT NOT NULL,
            policy_dependency_generation INTEGER NOT NULL,
            trust_state TEXT NOT NULL,
            trust_reason TEXT NOT NULL,
            continuity_generation INTEGER NOT NULL DEFAULT 1,
            epoch INTEGER NOT NULL,
            provider_id TEXT,
            provider_identity TEXT,
            durable_cursor INTEGER NOT NULL DEFAULT 0,
            linearizable_fence INTEGER NOT NULL DEFAULT 0,
            rename_pairing INTEGER NOT NULL DEFAULT 0,
            overflow_scope INTEGER NOT NULL DEFAULT 0,
            filesystem_supported INTEGER NOT NULL DEFAULT 0,
            clean_proof_allowed INTEGER NOT NULL DEFAULT 0,
            power_loss_durability INTEGER NOT NULL DEFAULT 0,
            durable_offset INTEGER NOT NULL DEFAULT 0,
            folded_offset INTEGER NOT NULL DEFAULT 0,
            max_candidate_rows INTEGER NOT NULL DEFAULT 250000,
            max_prefix_rows INTEGER NOT NULL DEFAULT 16384,
            max_observer_log_bytes INTEGER NOT NULL DEFAULT 268435456,
            max_segment_bytes INTEGER NOT NULL DEFAULT 16777216,
            max_unfolded_tail_records INTEGER NOT NULL DEFAULT 65536,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(scope_kind, owner_id)
        );
        CREATE TABLE changed_path_entries (
            scope_id TEXT NOT NULL REFERENCES changed_path_scopes(scope_id) ON DELETE CASCADE,
            normalized_path TEXT COLLATE BINARY NOT NULL,
            event_flags INTEGER NOT NULL,
            source_mask INTEGER NOT NULL,
            first_sequence INTEGER NOT NULL,
            last_sequence INTEGER NOT NULL,
            provider_id TEXT,
            provider_sequence INTEGER,
            intent_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(scope_id, normalized_path)
        );
        CREATE TABLE changed_path_prefixes (
            scope_id TEXT NOT NULL REFERENCES changed_path_scopes(scope_id) ON DELETE CASCADE,
            normalized_prefix TEXT COLLATE BINARY NOT NULL,
            completeness_reason TEXT NOT NULL,
            event_flags INTEGER NOT NULL,
            source_mask INTEGER NOT NULL,
            first_sequence INTEGER NOT NULL,
            last_sequence INTEGER NOT NULL,
            provider_id TEXT,
            provider_sequence INTEGER,
            intent_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(scope_id, normalized_prefix)
        );";

    fn scope_identity() -> ScopeIdentity {
        ScopeIdentity {
            scope_id: ScopeId([0xab; 32]),
            kind: ScopeKind::Workspace,
            owner_id: "workspace-main".into(),
        }
    }

    fn baseline() -> BaselineIdentity {
        BaselineIdentity {
            ref_name: "refs/branches/main".into(),
            ref_generation: 7,
            change_id: ChangeId("change-base".into()),
            root_id: ObjectId("root-base".into()),
        }
    }

    fn policy() -> PolicyIdentity {
        PolicyIdentity {
            fingerprint: [0xcd; 32],
            generation: 11,
        }
    }

    fn filesystem() -> FilesystemIdentity {
        FilesystemIdentity(vec![0, 0xff, b'f', b's'])
    }

    fn provider() -> ProviderIdentity {
        ProviderIdentity {
            identity: vec![0x80, 0, b'p'],
            capabilities: ProviderCapabilities {
                durable_cursor: true,
                linearizable_fence: true,
                rename_pairing: false,
                overflow_scope: true,
                filesystem_supported: true,
                clean_proof_allowed: false,
                power_loss_durability: true,
            },
        }
    }

    fn expected() -> ExpectedScope {
        ExpectedScope {
            scope_id: scope_identity().scope_id,
            epoch: 1,
            ref_name: baseline().ref_name,
            ref_generation: baseline().ref_generation,
            baseline_root: baseline().root_id,
            policy_fingerprint: policy().fingerprint,
            policy_generation: policy().generation,
            filesystem_identity: filesystem().0,
            provider_identity: provider().identity,
        }
    }

    fn fixture(max_candidates: u64, max_prefixes: u64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(FIXTURE_SCHEMA).unwrap();
        ChangedPathLedger::new(&conn)
            .begin_scope(
                &scope_identity(),
                &baseline(),
                &policy(),
                &filesystem(),
                &provider(),
            )
            .unwrap();
        conn.execute(
            "UPDATE changed_path_scopes
             SET max_candidate_rows = ?1, max_prefix_rows = ?2
             WHERE scope_id = ?3",
            params![
                max_candidates,
                max_prefixes,
                scope_identity().scope_id.to_text()
            ],
        )
        .unwrap();
        conn
    }

    fn set_trust(conn: &Connection, state: TrustState) {
        conn.execute(
            "UPDATE changed_path_scopes SET trust_state = ?1 WHERE scope_id = ?2",
            params![state.as_str(), scope_identity().scope_id.to_text()],
        )
        .unwrap();
    }

    fn prefix(path: &str, sequence: u64) -> DirtyPrefix {
        DirtyPrefix {
            path: LedgerPath::parse(path).unwrap(),
            complete: true,
            reason: format!("rescan-{path}"),
            first_sequence: sequence,
            last_sequence: sequence,
        }
    }

    #[test]
    fn begin_scope_is_untrusted_and_encodes_binary_identities_canonically() {
        let conn = fixture(10, 10);
        let row = conn
            .query_row(
                "SELECT scope_id, trust_state, policy_fingerprint,
                        filesystem_identity, provider_identity,
                        durable_cursor, linearizable_fence, clean_proof_allowed
                 FROM changed_path_scopes",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "ab".repeat(32));
        assert_eq!(row.1, "untrusted_gap");
        assert_eq!(row.2, "cd".repeat(32));
        assert_eq!(row.3, "00ff6673");
        assert_eq!(row.4, "800070");
        assert_eq!((row.5, row.6, row.7), (1, 1, 0));
    }

    #[test]
    fn begin_scope_matches_the_fresh_v18_schema() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        ChangedPathLedger::new(&db.conn)
            .begin_scope(
                &scope_identity(),
                &baseline(),
                &policy(),
                &filesystem(),
                &provider(),
            )
            .unwrap();
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT trust_state FROM changed_path_scopes WHERE scope_id = ?1",
                    [scope_identity().scope_id.to_text()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "untrusted_gap"
        );
    }

    #[test]
    fn ledger_path_accepts_only_normalized_relative_slash_paths_and_preserves_case() {
        assert_eq!(
            LedgerPath::parse("Src/Crab.rs").unwrap().as_str(),
            "Src/Crab.rs"
        );
        for invalid in [
            "",
            "/root",
            ".",
            "..",
            "a/./b",
            "a/../b",
            "a//b",
            "a/",
            "a\\b",
            "nul\0byte",
            "e\u{301}.txt",
        ] {
            assert!(LedgerPath::parse(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(LedgerPath::parse("é.txt").is_ok());
    }

    #[test]
    fn only_trusted_scope_can_return_an_authoritative_snapshot() {
        let conn = fixture(10, 10);
        let ledger = ChangedPathLedger::new(&conn);
        for state in [
            TrustState::Reconciling,
            TrustState::Overflow,
            TrustState::UntrustedGap,
            TrustState::StaleBaseline,
            TrustState::Corrupt,
        ] {
            set_trust(&conn, state);
            assert!(matches!(
                ledger.snapshot_candidates(&expected()),
                Err(Error::ChangeLedgerReconcileRequired { .. })
            ));
        }
        set_trust(&conn, TrustState::Trusted);
        assert_eq!(
            ledger.snapshot_candidates(&expected()).unwrap().trust,
            TrustState::Trusted
        );
    }

    #[test]
    fn snapshot_rejects_an_unfolded_durable_tail() {
        let conn = fixture(10, 10);
        conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = 'trusted', durable_offset = 8, folded_offset = 5",
            [],
        )
        .unwrap();

        assert!(matches!(
            ChangedPathLedger::new(&conn).snapshot_candidates(&expected()),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
    }

    #[test]
    fn snapshot_candidates_uses_one_read_snapshot_across_all_phases() {
        let workspace = tempfile::tempdir().unwrap();
        let database = workspace.path().join("ledger.db");
        let reader = Connection::open(&database).unwrap();
        reader.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
        reader.execute_batch(FIXTURE_SCHEMA).unwrap();
        ChangedPathLedger::new(&reader)
            .begin_scope(
                &scope_identity(),
                &baseline(),
                &policy(),
                &filesystem(),
                &provider(),
            )
            .unwrap();
        let before = LedgerPath::parse("before").unwrap();
        ChangedPathLedger::new(&reader)
            .upsert_exact(
                &expected(),
                &before,
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                1,
            )
            .unwrap();
        set_trust(&reader, TrustState::Trusted);
        let writer = Connection::open(&database).unwrap();

        let snapshot = ChangedPathLedger::new(&reader)
            .snapshot_candidates_with_phase_hook(&expected(), || {
                ChangedPathLedger::new(&writer)
                    .upsert_exact(
                        &expected(),
                        &LedgerPath::parse("after").unwrap(),
                        EvidenceFlags::CONTENT,
                        EvidenceSource::Observer,
                        2,
                    )
                    .unwrap();
            })
            .unwrap();

        assert_eq!(snapshot.exact_paths, vec![before]);
        assert_eq!(snapshot.cut.sequence, 1);
        assert_eq!(
            writer
                .query_row("SELECT COUNT(*) FROM changed_path_entries", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            2
        );
    }

    #[test]
    fn every_scope_mutation_rejects_a_stale_exact_cas_tuple() {
        let conn = fixture(10, 10);
        set_trust(&conn, TrustState::Trusted);
        let ledger = ChangedPathLedger::new(&conn);
        let stale_values = {
            let base = expected();
            let mut values = Vec::new();
            let mut stale = base.clone();
            stale.epoch += 1;
            values.push(stale);
            let mut stale = base.clone();
            stale.ref_name.push_str("-stale");
            values.push(stale);
            let mut stale = base.clone();
            stale.ref_generation += 1;
            values.push(stale);
            let mut stale = base.clone();
            stale.baseline_root = ObjectId("root-stale".into());
            values.push(stale);
            let mut stale = base.clone();
            stale.policy_fingerprint[0] ^= 1;
            values.push(stale);
            let mut stale = base.clone();
            stale.policy_generation += 1;
            values.push(stale);
            let mut stale = base.clone();
            stale.filesystem_identity.push(1);
            values.push(stale);
            let mut stale = base;
            stale.provider_identity.push(1);
            values.push(stale);
            values
        };

        for stale in stale_values {
            let assertions = [
                ledger.mark_untrusted(&stale, TrustState::StaleBaseline, "stale"),
                ledger.mark_prefix_dirty(&stale, &prefix("src", 1)),
                ledger.upsert_exact(
                    &stale,
                    &LedgerPath::parse("src/lib.rs").unwrap(),
                    EvidenceFlags::CONTENT,
                    EvidenceSource::Observer,
                    1,
                ),
                ledger.acknowledge(
                    &stale,
                    &EvidenceCut {
                        source: EvidenceSource::Observer,
                        sequence: 1,
                        durable_offset: 1,
                        folded_offset: 1,
                    },
                    &OwnedEvidence {
                        source: EvidenceSource::Observer,
                        through_sequence: 1,
                        exact_paths: Vec::new(),
                        prefixes: Vec::new(),
                    },
                ),
                ledger.advance_baseline(
                    &stale,
                    &BaselineIdentity {
                        ref_name: "refs/branches/main".into(),
                        ref_generation: 8,
                        change_id: ChangeId("change-next".into()),
                        root_id: ObjectId("root-next".into()),
                    },
                    &EvidenceCut {
                        source: EvidenceSource::Observer,
                        sequence: 1,
                        durable_offset: 1,
                        folded_offset: 1,
                    },
                ),
            ];
            assert!(assertions
                .into_iter()
                .all(|result| matches!(result, Err(Error::ChangeLedgerReconcileRequired { .. }))));
        }
    }

    #[test]
    fn mark_untrusted_rejects_trusted_as_an_input() {
        let conn = fixture(10, 10);
        let error = ChangedPathLedger::new(&conn)
            .mark_untrusted(&expected(), TrustState::Trusted, "not allowed")
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
    }

    #[test]
    fn mark_untrusted_advances_continuity_generation_atomically() {
        let conn = fixture(10, 10);
        let ledger = ChangedPathLedger::new(&conn);
        let before: i64 = conn
            .query_row(
                "SELECT continuity_generation FROM changed_path_scopes",
                [],
                |row| row.get(0),
            )
            .unwrap();

        ledger
            .mark_untrusted(&expected(), TrustState::StaleBaseline, "policy invalidated")
            .unwrap();

        let after: (String, i64) = conn
            .query_row(
                "SELECT trust_state,continuity_generation FROM changed_path_scopes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(after, ("stale_baseline".into(), before + 1));
    }

    #[test]
    fn complete_prefixes_coalesce_on_component_boundaries() {
        let conn = fixture(10, 10);
        let ledger = ChangedPathLedger::new(&conn);
        ledger
            .mark_prefix_dirty(&expected(), &prefix("dir/sub", 9))
            .unwrap();
        ledger
            .mark_prefix_dirty(&expected(), &prefix("directory", 4))
            .unwrap();
        ledger
            .mark_prefix_dirty(&expected(), &prefix("dir", 2))
            .unwrap();

        set_trust(&conn, TrustState::Trusted);
        let snapshot = ledger.snapshot_candidates(&expected()).unwrap();
        assert_eq!(snapshot.prefixes.len(), 2);
        assert_eq!(snapshot.prefixes[0].path.as_str(), "dir");
        assert!(snapshot.prefixes[0].complete);
        assert_eq!(snapshot.prefixes[0].reason, "rescan-dir");
        assert_eq!(
            (
                snapshot.prefixes[0].first_sequence,
                snapshot.prefixes[0].last_sequence
            ),
            (2, 9)
        );
        assert_eq!(snapshot.prefixes[1].path.as_str(), "directory");
    }

    #[test]
    fn incomplete_prefixes_are_rejected_instead_of_persisted_as_authoritative() {
        let conn = fixture(10, 10);
        let mut incomplete = prefix("dir", 1);
        incomplete.complete = false;
        let error = ChangedPathLedger::new(&conn)
            .mark_prefix_dirty(&expected(), &incomplete)
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM changed_path_prefixes", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
    }

    #[test]
    fn non_observer_sequences_do_not_inflate_the_observer_cut() {
        let conn = fixture(10, 10);
        let ledger = ChangedPathLedger::new(&conn);
        let mixed = LedgerPath::parse("mixed").unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &mixed,
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                5,
            )
            .unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &mixed,
                EvidenceFlags::MODE,
                EvidenceSource::Intent,
                500,
            )
            .unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &LedgerPath::parse("intent-only").unwrap(),
                EvidenceFlags::CREATE,
                EvidenceSource::Intent,
                900,
            )
            .unwrap();

        assert_eq!(
            conn.query_row(
                "SELECT provider_sequence FROM changed_path_entries
                 WHERE normalized_path = 'intent-only'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .unwrap(),
            None
        );
        set_trust(&conn, TrustState::Trusted);
        assert_eq!(
            ledger
                .snapshot_candidates(&expected())
                .unwrap()
                .cut
                .sequence,
            5
        );
    }

    #[test]
    fn observer_acknowledgement_retains_mixed_and_later_observer_evidence() {
        let conn = fixture(10, 10);
        let ledger = ChangedPathLedger::new(&conn);
        let mixed = LedgerPath::parse("mixed").unwrap();
        let later = LedgerPath::parse("later").unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &mixed,
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                5,
            )
            .unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &mixed,
                EvidenceFlags::MODE,
                EvidenceSource::Intent,
                500,
            )
            .unwrap();
        ledger
            .upsert_exact(
                &expected(),
                &later,
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                6,
            )
            .unwrap();
        set_trust(&conn, TrustState::Trusted);

        ledger
            .acknowledge(
                &expected(),
                &EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence: 5,
                    durable_offset: 0,
                    folded_offset: 0,
                },
                &OwnedEvidence {
                    source: EvidenceSource::Observer,
                    through_sequence: 5,
                    exact_paths: vec![mixed, later],
                    prefixes: Vec::new(),
                },
            )
            .unwrap();

        assert_eq!(
            ledger
                .all_exact(&expected())
                .unwrap()
                .into_iter()
                .map(|evidence| evidence.path.0)
                .collect::<Vec<_>>(),
            vec!["later".to_string(), "mixed".to_string()]
        );
    }

    #[test]
    fn acknowledgement_rejects_scope_boundary_mismatch_before_deleting_evidence() {
        let conn = fixture(10, 10);
        let path = LedgerPath::parse("kept").unwrap();
        let ledger = ChangedPathLedger::new(&conn);
        ledger
            .upsert_exact(
                &expected(),
                &path,
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                3,
            )
            .unwrap();
        conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = 'trusted', durable_offset = 10, folded_offset = 10",
            [],
        )
        .unwrap();

        assert!(matches!(
            ledger.acknowledge(
                &expected(),
                &EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence: 3,
                    durable_offset: 9,
                    folded_offset: 9,
                },
                &OwnedEvidence {
                    source: EvidenceSource::Observer,
                    through_sequence: 3,
                    exact_paths: vec![path],
                    prefixes: Vec::new(),
                },
            ),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(ledger.all_exact(&expected()).unwrap().len(), 1);
    }

    #[test]
    fn baseline_advance_rejects_scope_boundary_mismatch() {
        let conn = fixture(10, 10);
        conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = 'trusted', durable_offset = 10, folded_offset = 10",
            [],
        )
        .unwrap();
        let error = ChangedPathLedger::new(&conn)
            .advance_baseline(
                &expected(),
                &BaselineIdentity {
                    ref_name: "refs/branches/main".into(),
                    ref_generation: 8,
                    change_id: ChangeId("change-next".into()),
                    root_id: ObjectId("root-next".into()),
                },
                &EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence: 3,
                    durable_offset: 9,
                    folded_offset: 9,
                },
            )
            .unwrap_err();
        assert!(matches!(error, Error::ChangeLedgerReconcileRequired { .. }));
        assert_eq!(
            conn.query_row(
                "SELECT ref_generation FROM changed_path_scopes",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
            7
        );
    }

    #[test]
    fn acknowledgement_and_baseline_advance_leave_observer_offsets_unchanged() {
        let conn = fixture(10, 10);
        conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = 'trusted', durable_offset = 10, folded_offset = 10",
            [],
        )
        .unwrap();
        let ledger = ChangedPathLedger::new(&conn);
        let cut = EvidenceCut {
            source: EvidenceSource::Intent,
            sequence: 500,
            durable_offset: 10,
            folded_offset: 10,
        };
        ledger
            .acknowledge(
                &expected(),
                &cut,
                &OwnedEvidence {
                    source: EvidenceSource::Intent,
                    through_sequence: 500,
                    exact_paths: Vec::new(),
                    prefixes: Vec::new(),
                },
            )
            .unwrap();
        ledger
            .advance_baseline(
                &expected(),
                &BaselineIdentity {
                    ref_name: "refs/branches/main".into(),
                    ref_generation: 8,
                    change_id: ChangeId("change-next".into()),
                    root_id: ObjectId("root-next".into()),
                },
                &cut,
            )
            .unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT durable_offset, folded_offset FROM changed_path_scopes",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            (10, 10)
        );
    }

    #[test]
    fn candidate_cap_rolls_back_the_over_cap_row_and_rejects_later_writes() {
        let conn = fixture(1, 1);
        let ledger = ChangedPathLedger::new(&conn);
        ledger
            .upsert_exact(
                &expected(),
                &LedgerPath::parse("a").unwrap(),
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                1,
            )
            .unwrap();
        assert!(matches!(
            ledger.upsert_exact(
                &expected(),
                &LedgerPath::parse("b").unwrap(),
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                2,
            ),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(
            conn.query_row("SELECT trust_state FROM changed_path_scopes", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "overflow"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM changed_path_entries", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert!(matches!(
            ledger.upsert_exact(
                &expected(),
                &LedgerPath::parse("c").unwrap(),
                EvidenceFlags::CONTENT,
                EvidenceSource::Observer,
                3,
            ),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM changed_path_entries", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn prefix_cap_rolls_back_the_over_cap_row_and_rejects_later_writes() {
        let prefix_conn = fixture(10, 1);
        let prefix_ledger = ChangedPathLedger::new(&prefix_conn);
        prefix_ledger
            .mark_prefix_dirty(&expected(), &prefix("a", 1))
            .unwrap();
        assert!(matches!(
            prefix_ledger.mark_prefix_dirty(&expected(), &prefix("b", 2)),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(
            prefix_conn
                .query_row("SELECT trust_state FROM changed_path_scopes", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "overflow"
        );
        assert_eq!(
            prefix_conn
                .query_row("SELECT COUNT(*) FROM changed_path_prefixes", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
        assert!(matches!(
            prefix_ledger.mark_prefix_dirty(&expected(), &prefix("c", 3)),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(
            prefix_conn
                .query_row("SELECT COUNT(*) FROM changed_path_prefixes", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn prefix_cap_rollback_restores_rows_deleted_during_coalescing() {
        let conn = fixture(10, 2);
        let ledger = ChangedPathLedger::new(&conn);
        ledger
            .mark_prefix_dirty(&expected(), &prefix("dir/a", 1))
            .unwrap();
        ledger
            .mark_prefix_dirty(&expected(), &prefix("dir/b", 2))
            .unwrap();
        let rows = || {
            conn.prepare(
                "SELECT normalized_prefix, completeness_reason, source_mask,
                        first_sequence, last_sequence, provider_id,
                        provider_sequence, created_at, updated_at
                 FROM changed_path_prefixes
                 ORDER BY normalized_prefix COLLATE BINARY",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
        };
        let before = rows();
        conn.execute("UPDATE changed_path_scopes SET max_prefix_rows = 1", [])
            .unwrap();

        assert!(matches!(
            ledger.mark_prefix_dirty(&expected(), &prefix("dir/a/sub", 3)),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
        assert_eq!(rows(), before);
    }

    #[test]
    fn overflow_and_corrupt_scopes_reject_all_evidence_writes() {
        for state in [TrustState::Overflow, TrustState::Corrupt] {
            let conn = fixture(10, 10);
            set_trust(&conn, state);
            let ledger = ChangedPathLedger::new(&conn);
            assert!(matches!(
                ledger.upsert_exact(
                    &expected(),
                    &LedgerPath::parse("blocked").unwrap(),
                    EvidenceFlags::CONTENT,
                    EvidenceSource::Observer,
                    1,
                ),
                Err(Error::ChangeLedgerReconcileRequired { .. })
            ));
            assert!(matches!(
                ledger.mark_prefix_dirty(&expected(), &prefix("blocked", 1)),
                Err(Error::ChangeLedgerReconcileRequired { .. })
            ));
            assert_eq!(
                conn.query_row(
                    "SELECT
                         (SELECT COUNT(*) FROM changed_path_entries),
                         (SELECT COUNT(*) FROM changed_path_prefixes)",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .unwrap(),
                (0, 0)
            );
        }
    }

    #[test]
    fn trusted_baseline_advance_updates_the_identity_and_fences_the_old_expected_scope() {
        let conn = fixture(10, 10);
        conn.execute(
            "UPDATE changed_path_scopes
             SET trust_state = 'trusted', durable_offset = 34, folded_offset = 34",
            [],
        )
        .unwrap();
        let ledger = ChangedPathLedger::new(&conn);
        let target = BaselineIdentity {
            ref_name: "refs/branches/main".into(),
            ref_generation: 8,
            change_id: ChangeId("change-next".into()),
            root_id: ObjectId("root-next".into()),
        };
        ledger
            .advance_baseline(
                &expected(),
                &target,
                &EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence: 12,
                    durable_offset: 34,
                    folded_offset: 34,
                },
            )
            .unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT ref_generation, change_id, baseline_root_id, durable_offset,
                        folded_offset
                 FROM changed_path_scopes",
                [],
                |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                )),
            )
            .unwrap(),
            (8, "change-next".into(), "root-next".into(), 34, 34)
        );
        assert!(matches!(
            ledger.snapshot_candidates(&expected()),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn exact_event_coalescing_is_order_independent(
            events in prop::collection::vec((0u8..4, 0u8..6, 0u64..100), 1..30)
        ) {
            fn apply(events: &[(u8, u8, u64)]) -> ExactEvidence {
                let conn = fixture(100, 10);
                let ledger = ChangedPathLedger::new(&conn);
                let path = LedgerPath::parse("src/lib.rs").unwrap();
                for &(source, flag, sequence) in events {
                    ledger.upsert_exact(
                        &expected(),
                        &path,
                        EvidenceFlags::from_index(flag),
                        EvidenceSource::from_index(source),
                        sequence,
                    ).unwrap();
                }
                ledger.all_exact(&expected()).unwrap().remove(0)
            }

            let forward = apply(&events);
            let expected_flags = events.iter().fold(EvidenceFlags::default(), |mut flags, event| {
                flags |= EvidenceFlags::from_index(event.1);
                flags
            });
            let expected_sources = events.iter().fold(0, |mask, event| {
                mask | EvidenceSource::from_index(event.0).mask()
            });
            prop_assert_eq!(forward.path.as_str(), "src/lib.rs");
            prop_assert_eq!(forward.flags, expected_flags);
            prop_assert_eq!(forward.source_mask, expected_sources);
            prop_assert_eq!(forward.first_sequence, events.iter().map(|event| event.2).min().unwrap());
            prop_assert_eq!(forward.last_sequence, events.iter().map(|event| event.2).max().unwrap());
            let mut reversed = events.clone();
            reversed.reverse();
            prop_assert_eq!(forward, apply(&reversed));
        }

        #[test]
        fn acknowledgement_never_clears_later_or_other_source_evidence(
            events in prop::collection::vec((0u8..4, 0u64..40), 1..30),
            cut_sequence in 0u64..40,
            acknowledged_source in 0u8..4,
        ) {
            let conn = fixture(100, 10);
            let ledger = ChangedPathLedger::new(&conn);
            let path = LedgerPath::parse("src/lib.rs").unwrap();
            for &(source, sequence) in &events {
                ledger.upsert_exact(
                    &expected(),
                    &path,
                    EvidenceFlags::CONTENT,
                    EvidenceSource::from_index(source),
                    sequence,
                ).unwrap();
            }
            set_trust(&conn, TrustState::Trusted);
            let source = EvidenceSource::from_index(acknowledged_source);
            ledger.acknowledge(
                &expected(),
                &EvidenceCut {
                    source,
                    sequence: cut_sequence,
                    durable_offset: 0,
                    folded_offset: 0,
                },
                &OwnedEvidence {
                    source,
                    through_sequence: cut_sequence,
                    exact_paths: vec![path],
                    prefixes: Vec::new(),
                },
            ).unwrap();

            let remaining = ledger.all_exact(&expected()).unwrap();
            let has_later = events.iter().any(|&(event_source, sequence)| {
                EvidenceSource::from_index(event_source) == source && sequence > cut_sequence
            });
            let has_other = events.iter().any(|&(event_source, _)| {
                EvidenceSource::from_index(event_source) != source
            });
            prop_assert_eq!(remaining.is_empty(), !has_later && !has_other);
        }
    }
}
