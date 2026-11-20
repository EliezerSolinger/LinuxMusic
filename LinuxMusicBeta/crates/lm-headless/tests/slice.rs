//! `lm-headless slice` on the Dummy: the report on the standard output, the same export after reopening

use lm_state::{Value, from_json};

#[test]
fn slice_prints_its_report_and_the_export_is_identical() {
    let dir = std::env::temp_dir().join(format!("lm-headless-slice-{}", lm_io::new_uuid()));
    std::fs::create_dir_all(&dir).expect("dir");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_lm-headless"))
        .args(["slice", dir.join("song").to_str().expect("utf-8"), "--seconds", "1", "--block", "128"])
        .output()
        .expect("lm-headless");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    let report = from_json(text.trim()).expect("a JSON report");
    assert_eq!(report.get("identicalAfterReopen").and_then(Value::as_bool), Some(true));
    assert_eq!(report.get("device").and_then(|d| d.get("period")).and_then(Value::as_int), Some(128));
    assert!(report.get("takeSeconds").and_then(Value::as_real).unwrap_or(0.0) > 0.5, "{text}");
}
