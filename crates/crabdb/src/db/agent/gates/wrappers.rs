use super::*;

impl CrabDb {
    pub fn run_agent_test(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_test_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_test_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("test", agent, command, turn_id, timeout_secs, options)
    }

    pub fn run_agent_eval(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_eval_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_eval_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("eval", agent, command, turn_id, timeout_secs, options)
    }
}
