use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Stable identifier for a connected casting device / session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(Uuid);

impl DeviceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Connecting,
    Active,
    Ending,
    Ended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: DeviceId,
    pub device_name: String,
    pub protocol: String,
    pub state: SessionState,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

impl SessionInfo {
    pub fn new(device_name: impl Into<String>, protocol: impl Into<String>) -> Self {
        Self {
            id: DeviceId::new(),
            device_name: device_name.into(),
            protocol: protocol.into(),
            state: SessionState::Connecting,
            width: 1080,
            height: 1920,
            fps: 0.0,
        }
    }
}
