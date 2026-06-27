use super::*;

impl CrabDb {
    pub fn run_lane_test(
        &mut self,
        lane: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<LaneTestReport> {
        self.run_lane_test_with_options(
            lane,
            command,
            turn_id,
            timeout_secs,
            LaneGateOptions::default(),
        )
    }

    pub fn run_lane_test_with_options(
        &mut self,
        lane: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: LaneGateOptions,
    ) -> Result<LaneTestReport> {
        self.run_lane_gate("test", lane, command, turn_id, timeout_secs, options)
    }

    pub fn run_lane_eval(
        &mut self,
        lane: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<LaneTestReport> {
        self.run_lane_eval_with_options(
            lane,
            command,
            turn_id,
            timeout_secs,
            LaneGateOptions::default(),
        )
    }

    pub fn run_lane_eval_with_options(
        &mut self,
        lane: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: LaneGateOptions,
    ) -> Result<LaneTestReport> {
        self.run_lane_gate("eval", lane, command, turn_id, timeout_secs, options)
    }
}
