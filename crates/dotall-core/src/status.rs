use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectState {
    FreshFastPath,
    FreshAfterHash,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectStatus {
    pub path: String,
    pub format_id: String,
    pub state: ObjectState,
}
