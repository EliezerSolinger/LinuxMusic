//! The first slice (step 10) on the Dummy with a noise like the fan's: record with monitoring, a synth through an
//! effect, the analog rack and the master, save, close, reopen, and the same export bit for bit

mod common;

use common::*;
use lm_session::Session;
use lm_session::slice::{SliceOptions, run_slice};
use lm_state::Value;

#[test]
fn the_first_slice_end_to_end_on_the_dummy() {
    let dir = TempDir::new("slice");
    let mut config = config(&dir);
    config.dummy_source = Some(|| Box::new(lm_session::FanNoise::default()));
    let mut s = Session::new(config).expect("session");
    let mut o = SliceOptions::new(dir.join("slice"));
    o.record_seconds = 2.0;
    let report = run_slice(&mut s, &o).expect("slice");
    let real = |k: &str| report.get(k).and_then(Value::as_real).unwrap_or(f64::NAN);
    assert!(real("takeSeconds") > 1.5, "{report:?}");
    assert!(real("takeRmsDb") > -30.0 && real("takeRmsDb") < -10.0, "{report:?}");
    assert_eq!(report.get("identicalAfterReopen").and_then(Value::as_bool), Some(true), "{report:?}");
    assert_eq!(report.get("sameTreeAfterReopen").and_then(Value::as_bool), Some(true), "{report:?}");
    let peak = report.get("exportBefore").and_then(|e| e.get("peakDb")).and_then(Value::as_real).unwrap_or(-200.0);
    assert!(peak > -40.0, "the song sounds: {report:?}");
}

#[test]
fn the_export_is_identical_after_reopening_at_any_device_block() {
    // The device's block decides when builds and installs land in the live engine; the export must not depend on it
    for block in [32, 64, 128, 1024] {
        let dir = TempDir::new("slice-block");
        let mut config = config(&dir);
        config.block = block;
        config.dummy_source = Some(|| Box::new(lm_session::FanNoise::default()));
        let mut s = Session::new(config).expect("session");
        let mut o = SliceOptions::new(dir.join("slice"));
        o.record_seconds = 1.0;
        let report = run_slice(&mut s, &o).expect("slice");
        assert_eq!(report.get("identicalAfterReopen").and_then(Value::as_bool), Some(true), "{block}: {report:?}");
        let period = report.get("device").and_then(|d| d.get("period")).and_then(Value::as_int);
        assert_eq!(period, Some(i64::from(block)), "{report:?}");
    }
}
