use super::*;

impl Trail {
    pub(crate) fn set_ref(
        &self,
        name: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let now = now_ts();
        let generation = self
            .try_get_ref(name)?
            .map(|record| record.generation + 1)
            .unwrap_or(1);
        self.conn.execute(
            "INSERT INTO refs (name, change_id, root_id, operation_id, generation, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(name) DO UPDATE SET \
                change_id = excluded.change_id, root_id = excluded.root_id, \
                operation_id = excluded.operation_id, generation = excluded.generation, \
                updated_at = excluded.updated_at",
            params![
                name,
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now
            ],
        )?;
        write_ref_file(
            &self.db_dir,
            name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    pub(crate) fn advance_ref_cas(
        &self,
        expected: &RefRecord,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        self.advance_ref_cas_in_transaction(expected, change_id, root_id, operation_id)?;
        self.repair_ref_mirror(expected, change_id, root_id, operation_id)
    }

    pub(crate) fn advance_ref_cas_in_transaction(
        &self,
        expected: &RefRecord,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let generation = expected.generation + 1;
        let updated = self.conn.execute(
            "UPDATE refs SET change_id = ?1, root_id = ?2, operation_id = ?3, generation = ?4, updated_at = ?5 \
             WHERE name = ?6 AND generation = ?7 AND change_id = ?8",
            params![
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now_ts(),
                expected.name.clone(),
                expected.generation,
                expected.change_id.0.clone()
            ],
        )?;
        if updated != 1 {
            return Err(Error::StaleBranch(expected.name.clone()));
        }
        Ok(())
    }

    pub(crate) fn repair_ref_mirror(
        &self,
        expected: &RefRecord,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let generation = expected.generation + 1;
        write_ref_file(
            &self.db_dir,
            &expected.name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    pub(crate) fn get_ref(&self, name: &str) -> Result<RefRecord> {
        self.try_get_ref(name)?
            .ok_or_else(|| Error::RefNotFound(name.to_string()))
    }

    pub(crate) fn try_get_ref(&self, name: &str) -> Result<Option<RefRecord>> {
        self.conn
            .query_row(
                "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs WHERE name = ?1",
                params![name],
                ref_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn all_refs(&self) -> Result<Vec<RefRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn resolve_branch_ref(&self, branch: &str) -> Result<RefRecord> {
        if branch.starts_with("refs/") {
            self.get_ref(branch)
        } else {
            self.get_ref(&branch_ref(branch))
        }
    }

    pub(crate) fn resolve_refish(&self, refish: &str) -> Result<RefRecord> {
        if let Some(branch) = refish.strip_prefix("branch:") {
            return self.resolve_branch_ref(branch);
        }
        if let Some(lane) = refish.strip_prefix("lane:") {
            return self.get_ref(&lane_ref(lane));
        }
        if let Some(root_id) = refish.strip_prefix("root:") {
            return self.ref_from_root(&ObjectId(root_id.to_string()));
        }
        if let Some(change_id) = ChangeId::from_checkpoint_alias(refish) {
            return self.ref_from_change(&change_id);
        }
        if crate::ids::is_change_id(refish) {
            return self.ref_from_change(&ChangeId(refish.to_string()));
        }
        if refish.starts_with("refs/") {
            return self.get_ref(refish);
        }
        if let Ok(record) = self.get_ref(&branch_ref(refish)) {
            return Ok(record);
        }
        if let Ok(record) = self.get_ref(&lane_ref(refish)) {
            return Ok(record);
        }
        if crate::ids::is_object_id(refish) {
            return self.ref_from_root(&ObjectId(refish.to_string()));
        }
        Err(Error::RefNotFound(refish.to_string()))
    }

    pub(crate) fn ref_from_change(&self, change_id: &ChangeId) -> Result<RefRecord> {
        let op = self.operation(change_id)?;
        let operation_id: String = self.conn.query_row(
            "SELECT operation_id FROM operations WHERE change_id = ?1",
            params![change_id.0],
            |row| row.get(0),
        )?;
        Ok(RefRecord {
            name: format!("changes/{}", change_id.0),
            change_id: change_id.clone(),
            root_id: op.after_root,
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: op.created_at,
        })
    }

    pub(crate) fn ref_from_root(&self, root_id: &ObjectId) -> Result<RefRecord> {
        let _: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let row: Option<(String, String, i64)> = self
            .conn
            .query_row(
                "SELECT change_id, operation_id, created_at \
                 FROM operations WHERE after_root = ?1 \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![root_id.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((change_id, operation_id, created_at)) = row else {
            return Err(Error::InvalidInput(format!(
                "root `{}` is not associated with a recorded operation",
                root_id.0
            )));
        };
        Ok(RefRecord {
            name: format!("roots/{}", root_id.0),
            change_id: ChangeId(change_id),
            root_id: root_id.clone(),
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: created_at,
        })
    }

    pub(crate) fn common_parent_hint(
        &self,
        source: &ChangeId,
        target: &ChangeId,
    ) -> Result<ChangeId> {
        let source_ancestors = self.ancestor_set(source)?;
        let mut cursor = Some(target.clone());
        while let Some(change) = cursor {
            if source_ancestors.contains(&change.0) {
                return Ok(change);
            }
            cursor = self.first_parent(&change)?;
        }
        Err(Error::Conflict(
            "branches do not have a recorded common ancestor".to_string(),
        ))
    }

    pub(crate) fn ancestor_set(&self, change_id: &ChangeId) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        let mut stack = vec![change_id.clone()];
        while let Some(change) = stack.pop() {
            if !out.insert(change.0.clone()) {
                continue;
            }
            let parents = self.parents(&change)?;
            stack.extend(parents);
        }
        Ok(out)
    }

    pub(crate) fn first_parent(&self, change_id: &ChangeId) -> Result<Option<ChangeId>> {
        self.conn
            .query_row(
                "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position LIMIT 1",
                params![change_id.0],
                |row| Ok(ChangeId(row.get(0)?)),
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn first_parent_distance(
        &self,
        from: &ChangeId,
        ancestor: &ChangeId,
    ) -> Result<Option<u64>> {
        let mut distance = 0u64;
        let mut cursor = Some(from.clone());
        while let Some(change) = cursor {
            if &change == ancestor {
                return Ok(Some(distance));
            }
            cursor = self.first_parent(&change)?;
            distance = distance.saturating_add(1);
        }
        Ok(None)
    }

    pub(crate) fn parents(&self, change_id: &ChangeId) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
