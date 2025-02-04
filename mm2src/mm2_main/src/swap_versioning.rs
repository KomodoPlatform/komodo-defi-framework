//! Swap Versioning Module
//!
//! This module provides a dedicated type for handling swap versioning

#[derive(Copy, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SwapVersion {
    pub version: u32,
}

impl Default for SwapVersion {
    fn default() -> Self {
        Self {
            version: legacy_swap_version(),
        }
    }
}

impl SwapVersion {
    pub(crate) fn is_legacy(&self) -> bool { self.version == legacy_swap_version() }
}

impl From<u32> for SwapVersion {
    fn from(version: u32) -> Self { Self { version } }
}

pub(crate) const fn legacy_swap_version() -> u32 { 1 }
