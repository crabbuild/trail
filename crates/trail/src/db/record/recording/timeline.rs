use super::*;

impl Trail {
    pub fn timeline(&self, branch: Option<&str>, limit: usize) -> Result<Vec<TimelineEntry>> {
        let mut sql = String::from(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations",
        );
        if let Some(branch) = branch {
            let (branch_ref, bare_branch) = self.timeline_branch_terms(branch)?;
            if let Some(bare_branch) = bare_branch {
                sql.push_str(" WHERE branch = ?1 OR branch = ?2");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?3");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows =
                    stmt.query_map(params![branch_ref, bare_branch, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            } else {
                sql.push_str(" WHERE branch = ?1");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?2");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params![branch_ref, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        } else {
            sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?1");
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit as i64], timeline_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn timeline_query(
        &self,
        branch: Option<&str>,
        session: Option<&str>,
        lane: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TimelineEntry>> {
        let scoped = [branch.is_some(), session.is_some(), lane.is_some()]
            .into_iter()
            .filter(|set| *set)
            .count();
        if scoped > 1 {
            return Err(Error::InvalidInput(
                "timeline accepts only one of branch, session, or lane".to_string(),
            ));
        }
        if let Some(session_id) = session {
            return self.session_timeline(session_id, limit);
        }
        if let Some(lane) = lane {
            return self.lane_timeline(lane, limit);
        }
        self.timeline(branch, limit)
    }

    pub(crate) fn timeline_branch_terms(&self, branch: &str) -> Result<(String, Option<String>)> {
        let record = self.resolve_refish(branch)?;
        if record.name.starts_with(MAIN_REF_PREFIX) {
            let bare_branch = record
                .name
                .strip_prefix(MAIN_REF_PREFIX)
                .map(str::to_string);
            Ok((record.name, bare_branch))
        } else if record.name.starts_with(LANE_REF_PREFIX) {
            Ok((record.name, None))
        } else {
            Err(Error::InvalidInput(format!(
                "timeline --branch expects a branch or lane ref, got `{branch}`"
            )))
        }
    }

    pub fn session_timeline(&self, session_id: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        self.lane_session(session_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
