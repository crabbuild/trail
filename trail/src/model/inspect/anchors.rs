#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub version: u16,
    pub id: AnchorId,
    pub label: String,
    pub file_id: FileId,
    pub line_id: LineId,
    pub created_path: String,
    pub created_line: u64,
    pub created_change: ChangeId,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorCreateReport {
    pub anchor: Anchor,
    pub object_id: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorResolveReport {
    pub anchor: Anchor,
    pub branch: String,
    pub status: String,
    pub path: Option<String>,
    pub line_number: Option<u64>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorDeleteReport {
    pub anchor_id: AnchorId,
}
