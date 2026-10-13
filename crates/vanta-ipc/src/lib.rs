//! Typed actions shared by GUI, shortcuts, and AION.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    FocusWindow { id: u64 },
    CloseWindow { id: u64 },
    ToggleFloat { id: u64 },
    SwitchWorkspace { index: u32 },
    /// Intent path — executed only after AION mediation
    RequestIntent { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionResult {
    Ok,
    NeedConfirm { reason: String },
    Err { message: String },
}
