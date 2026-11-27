//! Every unit is listed, made, started at the common rates and blocks, processes to finite output, takes its
//! parameters, and no unit starts a thread (zita-convolver runs its one partition level in the calling thread)

use lm_sys_guitarix::{Instance, MIN_CONVOLVER_BLOCK, units};

fn threads() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // (the process's threads, from the system's own count)
        let out = std::process::Command::new("ps").args(["-M", "-p", &std::process::id().to_string()]).output();
        out.map(|o| String::from_utf8_lossy(&o.stdout).lines().count().saturating_sub(1)).unwrap_or(0)
    }
}

#[test]
fn the_legacy_list_of_units() {
    let all = units();
    assert_eq!(all.len(), 97);
    assert_eq!(all[0].id, "gain");
    assert!(all.iter().any(|u| u.id == "ampstack" && u.stereo && u.has_update));
    assert!(all.iter().any(|u| u.id == "cab" && !u.stereo && u.has_update));
    let tonestack = all.iter().find(|u| u.id == "amp.tonestack").unwrap();
    let select = &tonestack.params[tonestack.param("amp.tonestack.select").unwrap()];
    assert_eq!(select.values.len(), 27);
    assert_eq!((select.low, select.up), (0.0, 26.0));
}

#[test]
fn every_unit_plays_at_every_rate_and_starts_no_thread() {
    let before = threads();
    for unit in units() {
        for rate in [44_100u32, 48_000, 96_000, 192_000] {
            let block = if unit.id == "smbPitchShift" { 256 } else { MIN_CONVOLVER_BLOCK as usize };
            let mut i = Instance::new(unit.index).unwrap();
            i.start(rate, block as u32).unwrap();
            let mut input = [vec![0.0f32; block], vec![0.0f32; block]];
            let mut output = [vec![0.0f32; block], vec![0.0f32; block]];
            let mut n = 0u64;
            for _ in 0..40 {
                for ch in &mut input {
                    for x in ch.iter_mut() {
                        *x = (0.3 * libm_sin(n as f64 * 220.0 / f64::from(rate))) as f32;
                        n += 1;
                    }
                }
                let [in0, in1] = &mut input;
                let [out0, out1] = &mut output;
                if unit.stereo {
                    i.process_stereo([in0, in1], [out0, out1]);
                } else {
                    i.process_mono(in0, out0);
                }
                for x in output.iter().flatten() {
                    assert!(x.is_finite(), "{} at {rate}: {x}", unit.id);
                }
            }
        }
    }
    assert!(threads() <= before + 1, "the units started threads: {before} then {}", threads());
}

fn libm_sin(cycles: f64) -> f64 {
    (cycles * std::f64::consts::TAU).sin()
}

#[test]
fn parameters_reach_the_unit() {
    // The gain unit's volume: its output follows it
    let unit = units().iter().find(|u| u.id == "gain").unwrap();
    let p = unit.param("gain.gain").unwrap();
    let level = |db: f32| {
        let mut i = Instance::new(unit.index).unwrap();
        i.set(p, db);
        i.start(48_000, 256).unwrap();
        let mut input = vec![0.5f32; 256];
        let mut output = vec![0.0f32; 256];
        for _ in 0..200 {
            input.fill(0.5);
            i.process_mono(&mut input, &mut output);
        }
        output[255]
    };
    let (a, b) = (level(0.0), level(-6.0));
    assert!((20.0 * (b / a).log10() + 6.0).abs() < 0.1, "{a} {b}");
}

#[test]
#[should_panic]
fn a_block_larger_than_the_started_one_is_refused() {
    // The units size their buffers at start: more frames than that would write past them
    let unit = units().iter().find(|u| u.id == "gain").unwrap();
    let mut i = Instance::new(unit.index).unwrap();
    i.start(48_000, 64).unwrap();
    let mut input = vec![0.0f32; 128];
    let mut output = vec![0.0f32; 128];
    i.process_mono(&mut input, &mut output);
}

#[test]
fn an_unknown_unit_is_an_error_not_a_panic() {
    assert!(Instance::new(usize::MAX).is_err());
}
