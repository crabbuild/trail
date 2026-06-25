use super::*;

impl CrabDb {
    pub fn create_branch(&mut self, name: &str, from: Option<&str>) -> Result<BranchReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let ref_name = branch_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{name}` already exists"
            )));
        }
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        Ok(BranchReport {
            name: name.to_string(),
            from: source.change_id,
            root_id: source.root_id,
        })
    }

    pub fn list_branches(&self) -> Result<Vec<BranchListEntry>> {
        let current = self.current_branch()?;
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at \
             FROM refs WHERE name LIKE 'refs/branches/%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        let refs = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(refs
            .into_iter()
            .map(|record| {
                let name = record
                    .name
                    .strip_prefix(MAIN_REF_PREFIX)
                    .unwrap_or(&record.name)
                    .to_string();
                BranchListEntry {
                    is_current: name == current || record.name == current,
                    name,
                    ref_name: record.name,
                    change_id: record.change_id,
                    root_id: record.root_id,
                    generation: record.generation,
                }
            })
            .collect())
    }

    pub fn delete_branch(&mut self, name: &str) -> Result<BranchDeleteReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let current = self.current_branch()?;
        let ref_name = branch_ref(name);
        let short_name = ref_name.strip_prefix(MAIN_REF_PREFIX).unwrap_or(name);
        if short_name == current || ref_name == current {
            return Err(Error::InvalidInput(format!(
                "cannot delete current branch `{short_name}`"
            )));
        }
        self.get_ref(&ref_name)?;
        self.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![ref_name])?;
        remove_ref_file(&self.db_dir, &ref_name)?;
        Ok(BranchDeleteReport {
            name: short_name.to_string(),
            ref_name,
        })
    }

    pub fn rename_branch(&mut self, old_name: &str, new_name: &str) -> Result<BranchRenameReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(old_name)?;
        validate_ref_segment(new_name)?;
        let old_ref = branch_ref(old_name);
        let new_ref = branch_ref(new_name);
        let record = self.get_ref(&old_ref)?;
        if self.try_get_ref(&new_ref)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{new_name}` already exists"
            )));
        }
        self.conn.execute(
            "UPDATE refs SET name = ?1, updated_at = ?2 WHERE name = ?3",
            params![new_ref, now_ts(), old_ref],
        )?;
        remove_ref_file(&self.db_dir, &old_ref)?;
        write_ref_file(
            &self.db_dir,
            &new_ref,
            &record.change_id,
            &record.root_id,
            &record.operation_id,
            record.generation,
        )?;
        let current = self.current_branch()?;
        let old_short = old_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(old_name);
        let new_short = new_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(new_name);
        if current == old_short || current == old_ref {
            fs::write(self.db_dir.join(HEAD_FILE), format!("{new_short}\n"))?;
        }
        Ok(BranchRenameReport {
            old_name: old_short.to_string(),
            new_name: new_short.to_string(),
            change_id: record.change_id,
            root_id: record.root_id,
        })
    }
}
