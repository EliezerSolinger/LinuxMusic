//! No window system on this platform
use std::time::Duration;

use lm_base::{Error, Result};

pub fn init() -> Result<()> {
    Err(Error::new("gui.unsupported"))
}

pub fn pump(timeout: Duration) {
    std::thread::sleep(timeout);
}

#[derive(Debug)]
pub struct Native;

impl Native {
    pub fn new(_: &str, _: u32, _: u32) -> Result<Native> {
        Err(Error::new("gui.unsupported"))
    }
    pub fn handle(&self) -> usize {
        0
    }
    pub fn show(&mut self) {}
    pub fn hide(&mut self) {}
    pub fn set_size(&mut self, _: u32, _: u32) {}
    pub fn take_closed(&mut self) -> bool {
        false
    }
}
