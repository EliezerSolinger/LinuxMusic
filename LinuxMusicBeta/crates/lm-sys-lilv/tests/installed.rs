//! The third-party plugins installed on this machine, each probed in a child process as the scan helper does (H15):
//! instantiate, activate, run, save, restore, drop. A plugin that crashes or hangs its child is what the real scan
//! blocklists; it does not fail this test. What fails it is this crate misbehaving with a real plugin (a panic in the
//! child, which exits with 101).
//!
//! Folders: `LV2_PATH` if set, plus the usual system and package-manager folders that exist (on macOS that includes
//! Homebrew's, where `brew install carla` puts 21 LV2 plugins). With nothing installed, nothing is probed.

use lm_sys_lilv::{InstanceConfig, World};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROBE: &str = "LM_LILV_PROBE";

fn lv2_path() -> OsString {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("LV2_PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if cfg!(target_os = "macos") {
        dirs.extend([
            home.join("Library/Audio/Plug-Ins/LV2"),
            PathBuf::from("/Library/Audio/Plug-Ins/LV2"),
            PathBuf::from("/opt/homebrew/lib/lv2"),
            PathBuf::from("/usr/local/lib/lv2"),
        ]);
    } else {
        dirs.extend([
            home.join(".lv2"),
            PathBuf::from("/usr/local/lib/lv2"),
            PathBuf::from("/usr/lib/lv2"),
            PathBuf::from("/usr/lib/x86_64-linux-gnu/lv2"),
            PathBuf::from("/usr/lib/aarch64-linux-gnu/lv2"),
        ]);
    }
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    std::env::join_paths(dirs).unwrap_or_default()
}

fn config() -> InstanceConfig {
    InstanceConfig::new(48_000.0, 256)
}

/// The child: probes the plugin named by `LM_LILV_PROBE` (nothing when the variable is not set, as in a normal run)
#[test]
fn probe_child() {
    let Some(uri) = std::env::var_os(PROBE) else {
        return;
    };
    let uri = uri.to_string_lossy().into_owned();
    let world = World::new().unwrap();
    world.load_path(&lv2_path()).unwrap();
    // A refusal (missing feature, failed instantiate) is a keyed error: a normal outcome for a scan
    let Ok(mut instance) = world.instantiate(&uri, &config()) else {
        return;
    };
    instance.activate();
    let inputs: Vec<Vec<f32>> =
        instance.audio_inputs().iter().map(|_| (0..256).map(|n| (n as f32 * 0.05).sin() * 0.5).collect()).collect();
    let mut outputs: Vec<Vec<f32>> = instance.audio_outputs().iter().map(|_| vec![0.0; 256]).collect();
    for block in 0..8 {
        let ins: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();
        let mut outs: Vec<&mut [f32]> = outputs.iter_mut().map(Vec::as_mut_slice).collect();
        let frames = if block % 2 == 0 { 256 } else { 100 };
        instance.run(frames, &ins, &mut outs).unwrap();
    }
    let _ = instance.latency();
    if let Ok(Some(chunk)) = instance.save_state() {
        // The plugin's own state goes back in without complaint from this host
        if let Err(e) = instance.restore_state(&chunk) {
            assert_eq!(e.key(), "lilv.state_restore_failed", "{uri}: {e}");
        }
        let ins: Vec<&[f32]> = inputs.iter().map(Vec::as_slice).collect();
        let mut outs: Vec<&mut [f32]> = outputs.iter_mut().map(Vec::as_mut_slice).collect();
        instance.run(256, &ins, &mut outs).unwrap();
    }
    instance.deactivate();
    drop(instance);
}

#[test]
fn installed_plugins_are_probed_in_child_processes() {
    if std::env::var_os(PROBE).is_some() {
        return; // inside a child
    }
    let world = World::new().unwrap();
    world.load_path(&lv2_path()).unwrap();
    let candidates: Vec<String> =
        world.plugins().into_iter().filter(|p| p.missing(&config()).is_empty()).map(|p| p.uri).collect();
    let exe = std::env::current_exe().unwrap();
    let mut failures = Vec::new();
    for uri in &candidates {
        let mut child = Command::new(&exe)
            .args(["--exact", "probe_child", "--test-threads=1", "--quiet"])
            .env(PROBE, uri)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None; // hung: the scan would blocklist it
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // Exit 101 is a panic in the child: this crate failed with a real plugin. A signal or a hang is the
        // plugin's crash, which the scan blocklists
        if let Some(status) = status
            && status.code() == Some(101)
        {
            failures.push(uri.clone());
        }
    }
    assert!(failures.is_empty(), "the binding failed with these installed plugins: {failures:?}");
}
