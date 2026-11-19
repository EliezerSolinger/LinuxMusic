//! The real-time calls of an instance neither allocate nor free on the host side: under `assert_no_alloc` (Rust's
//! allocator, this thread) and the C heap trap (every thread on macOS, so the worker thread is counted too). The test
//! plugin's own run, work and work_response do not allocate either (no report id: it writes no log). The worker is
//! kept busy: requests are scheduled from run, worked on the worker thread, and their responses delivered after
//! later runs. `harness = false`: measured on the main thread.

mod common;
use common::bundle::{self, GAIN, port};
use common::heap;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_sys_lilv::{Instance, WorkerMode};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn exercise(i: &mut Instance, input: &[f32], output: &mut [f32], blocks: usize) {
    for b in 0..blocks {
        i.set_control(port::GAIN, 0.5 + (b % 3) as f32 * 0.25).unwrap();
        i.set_control(port::TRIGGER, if b % 4 == 0 { 1.0 } else { 0.0 }).unwrap();
        i.push_midi(port::MIDI_IN, 0, &[0x90, 60, 100]).unwrap();
        i.push_midi(port::MIDI_IN, 32, &[0x80, 60, 0]).unwrap();
        i.run(input.len(), &[input], &mut [output]).unwrap();
        let _ = i.control(port::WORKED);
        let _ = i.latency();
        let events = i.output_events(port::EVENTS_OUT).unwrap().count();
        assert_eq!(events, 2);
    }
}

fn main() {
    heap::install();
    heap::self_test();
    let input = vec![0.25_f32; 128];
    let mut output = vec![0.0_f32; 128];
    for mode in [WorkerMode::Threaded, WorkerMode::Synchronous] {
        let mut config = bundle::config();
        config.worker = mode;
        let mut i = bundle::world().instantiate(GAIN, &config).unwrap();
        i.activate();
        // Warm up outside the measure until the worker has really answered: the worker thread's first park, wake-up
        // and lock make the system set up thread-local state once (measured: 3 C heap events on macOS, on the worker
        // thread, never again). Without waiting, those land inside the measure
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while i.control(port::WORKED).unwrap() < 4.0 {
            assert!(std::time::Instant::now() < deadline, "{mode:?}: the worker never answered");
            exercise(&mut i, &input, &mut output, 4);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Several rounds, every one clean: nothing is allocated per call, nor once in a while
        for round in 0..3 {
            let events = heap::measure(|| assert_no_alloc(|| exercise(&mut i, &input, &mut output, 2000)));
            assert_eq!(events, 0, "{mode:?} round {round}: the C heap was touched {events} times");
        }
        assert!(i.control(port::WORKED).unwrap() > 100.0, "{mode:?}: the worker really worked");
    }
    // The test harness is off: success is the exit status
}
