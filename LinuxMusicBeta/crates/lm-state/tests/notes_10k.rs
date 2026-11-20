//! THE MEASUREMENT that decides `imbl` (interfaces.md, Parte 4, step 2; C1): an item with 10,000 notes in a note
//! map, edited one note at a time 1,000 times, every edit making a new root that shares with the old one, and all
//! 1,000 roots kept (the undo history).
//!
//! Criteria: under 1 ms per edit on average (in release), and the memory of the history grows with the number of
//! edits (a path per edit), not with copies of the item. Two measures:
//! - exact, at the level of the tree: between two consecutive roots, exactly one note is a different node and
//!   everything off the edited path is the same node ([`Value::same_node`]);
//! - in bytes, including `imbl`'s internal nodes: the resident memory of the process, read from the system
//!   (`VmRSS` in `/proc/self/status` on Linux, `ps` elsewhere). This file is its own test binary with a single
//!   test, so nothing else allocates while it measures. (The workspace takes no test-only crates, so there is no
//!   counting allocator: a `GlobalAlloc` would need `unsafe`.)
//!
//! Run with `cargo test --release -p lm-state --test notes_10k -- --nocapture` to see the numbers.

use std::time::{Duration, Instant};

use lm_state::{PathStep, Value};

const NOTES: u64 = 10_000;
const EDITS: usize = 1_000;
/// The first note number: notes take numbers from the global counter, after the ids
const FIRST_N: u64 = 1_000;

/// The resident memory of this process, in bytes
fn live_bytes() -> i64 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let kb = status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<i64>().ok())
            .expect("VmRSS in /proc/self/status");
        return kb * 1024;
    }
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps, to read the resident memory");
    let kb: i64 = String::from_utf8_lossy(&out.stdout).trim().parse().expect("a number of kB from ps");
    kb * 1024
}

fn k(key: &str) -> PathStep {
    PathStep::key(key)
}

fn note(i: u64) -> Value {
    Value::object([
        ("key", Value::int(i64::try_from(36 + i % 48).unwrap())),
        ("start", Value::int(i64::try_from(i * 120).unwrap())),
        ("length", Value::int(120)),
        ("velocity", Value::real(0.5 + f64::from(u32::try_from(i % 50).unwrap()) / 100.0).unwrap()),
    ])
}

fn project() -> Value {
    let notes = Value::object((0..NOTES).map(|i| (lm_state::note_key(FIRST_N + i), note(i))));
    let item = Value::object([
        ("id", Value::text("c:52")),
        ("kind", Value::text("midi")),
        ("start", Value::int(0)),
        ("length", Value::int(1_200_000)),
        ("notes", notes),
    ]);
    let other_track = Value::object([("id", Value::text("t:18")), ("items", Value::empty_list())]);
    let track = Value::object([("id", Value::text("t:17")), ("items", Value::list([item]))]);
    Value::object([
        ("meta", Value::object([("issued", Value::int(20_000))])),
        ("tracks", Value::list([track, other_track])),
    ])
}

fn notes_of(root: &Value) -> &Value {
    root.at_path(&[k("tracks"), PathStep::Index(0), k("items"), PathStep::Index(0), k("notes")]).unwrap()
}

#[test]
// A measurement of time and resident memory (read from the system through `ps` or /proc): neither means anything
// under miri, which also cannot start a process; the random trees of json_roundtrip and robustness cover `imbl` there
#[cfg_attr(miri, ignore)]
#[allow(clippy::print_stdout, clippy::cast_precision_loss)]
fn ten_thousand_notes_edited_one_at_a_time() {
    // A timing and memory budget means nothing under a sanitizer (its allocator and checks change both)
    if std::env::var_os("LM_TEST_SANITIZER").is_some() {
        return;
    }
    // ---- the item ----
    let before = live_bytes();
    let root = project();
    let item_bytes = live_bytes() - before;
    assert_eq!(notes_of(&root).len(), 10_000);

    // ---- 1,000 edits, every root kept ----
    let mut roots: Vec<Value> = Vec::with_capacity(EDITS + 1);
    let mut edited_n = Vec::with_capacity(EDITS);
    let mut checkpoints = Vec::with_capacity(5);
    let mut slowest = Duration::ZERO;
    roots.push(root);
    let history_start = live_bytes();
    checkpoints.push(0i64);
    let mut elapsed = Duration::ZERO;
    for e in 0..EDITS {
        // A different note each time, spread over the whole map
        let n = FIRST_N + (u64::try_from(e).unwrap() * 7_919) % NOTES;
        let path = [
            k("tracks"),
            PathStep::Index(0),
            k("items"),
            PathStep::Index(0),
            k("notes"),
            PathStep::Note(n),
            k("start"),
        ];
        let t0 = Instant::now();
        let previous = roots.last().unwrap();
        let old_start = previous.at_path(&path).unwrap().as_int().unwrap();
        let next = previous.with_path(&path, Value::int(old_start + 1)).unwrap();
        let took = t0.elapsed();
        elapsed += took;
        slowest = slowest.max(took);
        roots.push(next);
        edited_n.push(n);
        if (e + 1) % 250 == 0 {
            checkpoints.push(live_bytes() - history_start);
        }
    }
    let history_bytes = live_bytes() - history_start;
    let per_edit = elapsed / u32::try_from(EDITS).unwrap();

    // ---- dragging one note at 60 frames per second for 10 s (C1): 600 edits of the same note ----
    let drag_path = [
        k("tracks"),
        PathStep::Index(0),
        k("items"),
        PathStep::Index(0),
        k("notes"),
        PathStep::Note(FIRST_N + 5_000),
        k("start"),
    ];
    let mut dragged = roots.last().unwrap().clone();
    let drag_started = Instant::now();
    let mut drag_slowest = Duration::ZERO;
    for frame in 0..600i64 {
        let t0 = Instant::now();
        dragged = dragged.with_path(&drag_path, Value::int(600_000 + frame * 10)).unwrap();
        drag_slowest = drag_slowest.max(t0.elapsed());
    }
    let drag_per_frame = drag_started.elapsed() / 600;

    // ---- sharing: between two consecutive roots, exactly one note is a different node ----
    let mut shared_checks = 0usize;
    for (e, pair) in roots.windows(2).enumerate() {
        let (a, b) = (notes_of(&pair[0]), notes_of(&pair[1]));
        assert!(!a.same_node(b));
        let mut changed = Vec::new();
        for ((ka, va), (kb, vb)) in a.entries().zip(b.entries()) {
            assert_eq!(ka, kb);
            if !va.same_node(vb) {
                changed.push(lm_state::parse_note_key(ka).unwrap());
            }
            shared_checks += 1;
        }
        assert_eq!(changed, [edited_n[e]], "edit {e}");
        // Everything off the path is the same node
        let other = [k("tracks"), PathStep::Index(1)];
        assert!(pair[0].at_path(&other).unwrap().same_node(pair[1].at_path(&other).unwrap()));
        assert!(pair[0].get("meta").unwrap().same_node(pair[1].get("meta").unwrap()));
    }
    // The first root was never touched
    assert_eq!(roots[0], project());

    // ---- report ----
    let per_edit_bytes = history_bytes as f64 / EDITS as f64;
    let quarters: Vec<i64> = checkpoints.windows(2).map(|w| w[1] - w[0]).collect();
    println!("--- lm-state: 10,000-note measurement (imbl) ---");
    println!("item with 10,000 notes: {:.2} MB", item_bytes as f64 / 1e6);
    println!(
        "1,000 edits: total {:.3} ms, {:.2} µs per edit on average, slowest {:.2} µs",
        elapsed.as_secs_f64() * 1e3,
        per_edit.as_secs_f64() * 1e6,
        slowest.as_secs_f64() * 1e6
    );
    println!(
        "history of 1,000 roots: {:.2} MB, {:.0} bytes per edit ({:.3}% of a copy of the item)",
        history_bytes as f64 / 1e6,
        per_edit_bytes,
        100.0 * per_edit_bytes / item_bytes as f64
    );
    println!("growth per 250 edits: {quarters:?} bytes");
    println!(
        "drag at 60 fps: {:.2} µs per frame, slowest {:.2} µs",
        drag_per_frame.as_secs_f64() * 1e6,
        drag_slowest.as_secs_f64() * 1e6
    );
    println!("sharing checked on {shared_checks} note pairs: one changed node per edit");

    // ---- criteria ----
    assert!(per_edit < Duration::from_millis(1), "{per_edit:?} per edit");
    // A copy per edit would be ~item_bytes per edit; a path is a tiny fraction of it
    assert!(per_edit_bytes < item_bytes as f64 / 100.0, "{per_edit_bytes} bytes per edit, item {item_bytes}");
    // Linear: every quarter of the edits costs about the same (no copies, no growth with history size)
    let mean = quarters.iter().sum::<i64>() as f64 / quarters.len() as f64;
    for q in &quarters {
        let ratio = *q as f64 / mean;
        assert!((0.75..1.25).contains(&ratio), "quarter {q} vs mean {mean}: {quarters:?}");
    }
    assert!(drag_slowest < Duration::from_millis(16), "a frame is 16 ms");
    drop(roots);
}
