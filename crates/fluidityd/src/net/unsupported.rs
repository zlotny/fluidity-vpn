use std::path::Path;

use super::Backend;
use crate::netconfig::NetConfig;

pub struct Unsupported;

impl Unsupported {
    pub fn new(_state_dir: &Path) -> Self {
        Self
    }
}

impl Backend for Unsupported {
    fn apply(&self, _id: &str, config: &NetConfig) -> Result<(), String> {
        if config.is_empty() {
            Ok(())
        } else {
            Err("network settings are not supported on this platform".into())
        }
    }

    fn revert(&self, _id: &str) {}

    fn recover(&self) {}
}
