//! A temporary folder per test, removed at the end (no `tempfile` crate: it is not in the workspace list)

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "lm-sys-sndfile-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A deterministic test signal: a stereo sweep-ish mix in [-0.9, 0.9], interleaved
pub fn signal(frames: usize, channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for n in 0..frames {
        for c in 0..channels {
            let t = n as f64 / 48_000.0;
            let v = 0.6 * (std::f64::consts::TAU * (220.0 + 110.0 * c as f64) * t).sin()
                + 0.3 * (std::f64::consts::TAU * 3_000.0 * t).sin();
            out.push(v as f32);
        }
    }
    out
}

/// Every value an int16 can hold, scaled for a float file (k / 32768), repeated to `len`
pub fn int16_values(len: usize) -> Vec<i16> {
    (0..len).map(|i| (i as i64 % 65_536 - 32_768) as i16).collect()
}
