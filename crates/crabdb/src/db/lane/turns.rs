use super::*;

impl CrabDb {
    pub fn apply_lane_turn_patch(
        &mut self,
        turn_id: &str,
        patch: PatchDocument,
    ) -> Result<LanePatchReport> {
        let _lock = self.acquire_write_lock()?;
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        self.apply_lane_patch_locked(&turn.lane_id, patch, Some(&turn))
    }

    pub fn end_lane_turn(&mut self, turn_id: &str, status: &str) -> Result<LaneTurnEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Ok(LaneTurnEndReport { turn });
        }
        let after_change = turn
            .after_change
            .as_ref()
            .unwrap_or(&turn.before_change)
            .clone();
        self.finish_lane_turn(turn_id, status, Some(&after_change))?;
        self.insert_lane_event_with_context(
            &turn.lane_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "turn_ended",
            Some(&after_change),
            None,
            &serde_json::json!({
                "turn_id": turn_id,
                "status": status
            }),
        )?;
        Ok(LaneTurnEndReport {
            turn: self.lane_turn(turn_id)?,
        })
    }
}
