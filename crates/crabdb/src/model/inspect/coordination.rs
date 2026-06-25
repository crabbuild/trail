#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub agent_id: String,
    pub ref_name: String,
    pub path: Option<String>,
    pub file_id: Option<String>,
    pub mode: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentClaimReport {
    pub agent_id: String,
    pub ref_name: String,
    pub path: String,
    pub mode: String,
    pub ttl_secs: u64,
    pub claimed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<LeaseRecord>,
    #[serde(default)]
    pub conflicts: Vec<LeaseRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseAcquireReport {
    pub lease: LeaseRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseReleaseReport {
    pub lease_id: String,
    pub released: bool,
}
