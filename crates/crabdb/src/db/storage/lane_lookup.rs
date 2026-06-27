use super::*;

impl CrabDb {
    pub(crate) fn lane_branch(&self, lane: &str) -> Result<LaneBranch> {
        self.conn
            .query_row(
                "SELECT lane_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at \
                 FROM lane_branches WHERE lane_id = ?1 OR ref_name = ?2 OR lane_id IN (SELECT lane_id FROM lanes WHERE name = ?1)",
                params![lane, lane_ref(lane)],
                |row| {
                    Ok(LaneBranch {
                        lane_id: row.get(0)?,
                        ref_name: row.get(1)?,
                        base_change: ChangeId(row.get(2)?),
                        head_change: ChangeId(row.get(3)?),
                        base_root: ObjectId(row.get(4)?),
                        head_root: ObjectId(row.get(5)?),
                        session_id: row.get(6)?,
                        workdir: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(lane_ref(lane)))
    }

    pub(crate) fn lane_record(&self, lane_id: &str) -> Result<LaneRecord> {
        self.conn
            .query_row(
                "SELECT lane_id, name, kind, provider, model, created_at, metadata_json \
                 FROM lanes WHERE lane_id = ?1 OR name = ?1",
                params![lane_id],
                |row| {
                    Ok(LaneRecord {
                        lane_id: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        provider: row.get(3)?,
                        model: row.get(4)?,
                        created_at: row.get(5)?,
                        metadata_json: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(lane_id.to_string()))
    }

    pub(crate) fn lease(&self, lease_id: &str) -> Result<LeaseRecord> {
        self.conn
            .query_row(
                "SELECT lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE lease_id = ?1",
                params![lease_id],
                lease_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lease `{lease_id}` not found")))
    }

    pub(crate) fn existing_active_lease(
        &self,
        lane_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Option<LeaseRecord>> {
        self.conn
            .query_row(
                "SELECT lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE lane_id = ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
                   AND mode = ?3 AND expires_at > ?4 ORDER BY expires_at DESC LIMIT 1",
                params![lane_id, path, mode, now_ts()],
                lease_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn conflicting_active_leases(
        &self,
        lane_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Vec<LeaseRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at \
             FROM leases WHERE lane_id != ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
               AND expires_at > ?3 ORDER BY expires_at ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![lane_id, path, now_ts()], lease_row)?;
        let leases = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(leases
            .into_iter()
            .filter(|lease| mode == "write" || lease.mode == "write")
            .collect())
    }
}
