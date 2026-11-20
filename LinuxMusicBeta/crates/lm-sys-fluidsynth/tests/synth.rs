//! FluidSynth renders, with a SoundFont made by the test

mod common;
use common::sf2;
use lm_sys_fluidsynth::{ChannelType, MAX_OUTPUTS, Synth, SynthBuilder, SynthConfig, version};
use std::path::PathBuf;

struct Font(PathBuf);
impl Font {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("lm-sys-fluidsynth-{}-{name}.sf2", std::process::id()));
        sf2::standard(&path).unwrap();
        Self(path)
    }
}
impl Drop for Font {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn synth(name: &str, config: SynthConfig) -> Synth {
    let font = Font::new(name);
    let mut b = SynthBuilder::new(config).unwrap();
    let id = b.load_soundfont(&font.0).unwrap();
    for channel in 0..16 {
        b.program_select(channel, id, 0, 0).unwrap();
    }
    b.build()
}

/// Renders `frames` in blocks of 64, returns each output channel
fn render(s: &mut Synth, frames: usize) -> Vec<Vec<f32>> {
    let n = s.outputs() * 2;
    let mut all = vec![Vec::new(); n];
    let mut bufs = vec![vec![0.0_f32; 64]; n];
    for _ in 0..frames / 64 {
        let mut refs: Vec<&mut [f32]> = bufs.iter_mut().map(|b| b.as_mut_slice()).collect();
        s.render(&mut refs).unwrap();
        for (a, b) in all.iter_mut().zip(&bufs) {
            a.extend_from_slice(b);
        }
    }
    all
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// The frequency from the zero crossings going up
fn frequency(x: &[f32], rate: f64) -> f64 {
    let ups: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0).collect();
    (ups.len() - 1) as f64 * rate / (ups[ups.len() - 1] - ups[0]) as f64
}

#[test]
fn the_library_is_2_2_or_later() {
    let (major, minor, _) = version();
    assert!(major > 2 || (major == 2 && minor >= 2), "{:?}", version());
}

#[test]
fn a_note_sounds_at_its_pitch_and_stops() {
    let mut s = synth("note", SynthConfig::default());
    assert_eq!(s.midi_channels(), 16);
    let silent = render(&mut s, 4_800);
    assert!(silent.iter().all(|c| c.iter().all(|v| *v == 0.0)), "silence before any note");

    s.note_on(0, 69, 127).unwrap();
    let out = render(&mut s, 48_000);
    let (left, right) = (&out[0][4_800..], &out[1][4_800..]);
    assert!(rms(left) > 0.05 && rms(right) > 0.05, "rms {} {}", rms(left), rms(right));
    let f = frequency(left, 48_000.0);
    assert!((f - 440.0).abs() < 3.0, "plays at {f} Hz");
    // An octave up
    s.note_on(0, 81, 127).unwrap();
    s.all_sounds_off(Some(0)).unwrap();
    s.note_on(0, 81, 127).unwrap();
    let out = render(&mut s, 9_600);
    let f = frequency(&out[0][960..], 48_000.0);
    assert!((f - 880.0).abs() < 6.0, "octave at {f} Hz");

    s.note_off(0, 81).unwrap();
    let out = render(&mut s, 48_000);
    assert!(rms(&out[0][24_000..]) < 1e-6, "silent after the release");
    assert!(s.note_off(0, 81).is_err(), "a note that is not playing is rejected");
}

#[test]
fn all_sounds_off_cuts_at_once() {
    let mut s = synth("cut", SynthConfig::default());
    for key in [60, 64, 67] {
        s.note_on(3, key, 100).unwrap();
    }
    assert!(rms(&render(&mut s, 4_800)[0]) > 0.01);
    s.all_sounds_off(None).unwrap();
    let out = render(&mut s, 640);
    assert!(rms(&out[0][128..]) < 1e-6);
    // All notes off releases instead
    s.note_on(1, 69, 100).unwrap();
    render(&mut s, 640);
    s.all_notes_off(Some(1)).unwrap();
    let out = render(&mut s, 48_000);
    assert!(rms(&out[0][24_000..]) < 1e-6);
}

#[test]
fn each_midi_channel_plays_into_its_own_output() {
    let config = SynthConfig { outputs: 9, ..SynthConfig::default() };
    let mut s = synth("multi", config);
    assert_eq!(s.outputs(), 9);
    s.note_on(0, 69, 127).unwrap();
    s.note_on(4, 57, 127).unwrap();
    s.note_on(13, 45, 127).unwrap(); // 13 % 9 = output 4 as well
    let out = render(&mut s, 9_600);
    for (o, pair) in out.chunks(2).enumerate() {
        let level = rms(&pair[0]) + rms(&pair[1]);
        if o == 0 || o == 4 {
            assert!(level > 0.05, "output {o} is silent");
        } else {
            assert_eq!(level, 0.0, "output {o} has sound");
        }
    }
    let f = frequency(&out[0][960..], 48_000.0);
    assert!((f - 440.0).abs() < 3.0, "output 0 has only channel 0 ({f} Hz)");
}

#[test]
fn programs_banks_controllers_and_drum_channels() {
    let font = Font::new("programs");
    let mut b = SynthBuilder::new(SynthConfig::default()).unwrap();
    let id = b.load_soundfont(&font.0).unwrap();
    let presets = b.presets(id).unwrap();
    let names: Vec<_> = presets.iter().map(|p| (p.bank, p.program, p.name.as_str())).collect();
    assert_eq!(names, [(0, 0, "Sine"), (0, 1, "Noise")]);
    b.program_select(0, id, 0, 1).unwrap();
    assert!(b.program_select(0, id, 5, 77).is_err(), "no such preset");
    b.set_channel_type(9, ChannelType::Melodic).unwrap();
    b.program_select(9, id, 0, 0).unwrap();
    b.set_channel_type(2, ChannelType::Drum).unwrap();
    b.cc(0, 10, 0).unwrap(); // hard left
    let mut s = b.build();

    s.note_on(0, 69, 127).unwrap();
    let out = render(&mut s, 4_800);
    assert!(rms(&out[0]) > 0.05 && rms(&out[1]) < rms(&out[0]) * 0.05, "panned left");
    let ups = (1..out[0].len()).filter(|&i| out[0][i - 1] < 0.0 && out[0][i] >= 0.0).count();
    assert!(ups > 300, "noise crosses zero far more often than a 440 Hz sine ({ups})");

    s.all_sounds_off(None).unwrap();
    s.program_change(0, 0).unwrap();
    s.bank_select(0, 0).unwrap();
    s.cc(0, 10, 64).unwrap();
    s.pitch_bend(0, 8192).unwrap();
    s.note_on(0, 69, 127).unwrap();
    let out = render(&mut s, 9_600);
    let f = frequency(&out[0][960..], 48_000.0);
    assert!((f - 440.0).abs() < 3.0, "back on the sine ({f} Hz)");
    assert!(s.note_on(99, 60, 100).is_err(), "no channel 99");
    // Channel 9, made melodic, plays the sine
    s.note_on(9, 69, 100).unwrap();
}

#[test]
fn bad_configurations_and_fonts_are_errors() {
    let e = SynthBuilder::new(SynthConfig { outputs: 0, ..SynthConfig::default() }).unwrap_err();
    assert_eq!(e.key(), "fluidsynth.bad_config");
    let e = SynthBuilder::new(SynthConfig { outputs: MAX_OUTPUTS + 1, ..SynthConfig::default() }).unwrap_err();
    assert_eq!(e.key(), "fluidsynth.bad_config");
    let e = SynthBuilder::new(SynthConfig { midi_channels: 20, ..SynthConfig::default() }).unwrap_err();
    assert_eq!(e.key(), "fluidsynth.bad_config");
    let e = SynthBuilder::new(SynthConfig { sample_rate: 1_000_000.0, ..SynthConfig::default() }).unwrap_err();
    assert_eq!(e.key(), "fluidsynth.bad_config");

    let mut b = SynthBuilder::new(SynthConfig::default()).unwrap();
    let missing = std::env::temp_dir().join("lm-sys-fluidsynth-no-such-font.sf2");
    let e = b.load_soundfont(&missing).unwrap_err();
    assert_eq!(e.key(), "fluidsynth.load_failed");
    let garbage = std::env::temp_dir().join(format!("lm-sys-fluidsynth-{}-garbage.sf2", std::process::id()));
    std::fs::write(&garbage, b"RIFF\x10\0\0\0sfbkthis is no font").unwrap();
    let e = b.load_soundfont(&garbage).unwrap_err();
    std::fs::remove_file(&garbage).unwrap();
    assert_eq!(e.key(), "fluidsynth.load_failed");
    assert!(e.label.args.iter().any(|(name, _)| *name == "message"), "FluidSynth's reason is kept: {e}");

    let mut s = b.build();
    let mut wrong = [vec![0.0_f32; 64], vec![0.0_f32; 32]];
    let mut refs: Vec<&mut [f32]> = wrong.iter_mut().map(|b| b.as_mut_slice()).collect();
    assert_eq!(s.render(&mut refs).unwrap_err().key(), "fluidsynth.bad_buffers");
    assert_eq!(s.render(&mut []).unwrap_err().key(), "fluidsynth.bad_buffers");
}

#[test]
fn the_synth_moves_to_the_audio_thread_and_back() {
    let mut s = synth("moves", SynthConfig::default());
    s.note_on(0, 69, 127).unwrap();
    let (s, level) = std::thread::spawn(move || {
        let out = render(&mut s, 4_800);
        (s, rms(&out[0]))
    })
    .join()
    .unwrap();
    assert!(level > 0.05);
    drop(s);
}

#[cfg(target_os = "macos")]
fn thread_count() -> usize {
    // The threads of this process, from the kernel
    let out = std::process::Command::new("ps").args(["-M", "-p", &std::process::id().to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count() - 1
}

#[cfg(target_os = "linux")]
fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().count()
}

#[test]
fn no_thread_is_created() {
    // Other tests of this binary run in parallel threads: measure in a child process, alone
    if std::env::var_os("LM_FLUID_THREADS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "no_thread_is_created", "--test-threads=1", "--nocapture"])
            .env("LM_FLUID_THREADS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let font = Font::new("threads");
    // On Linux, nothing at all. On macOS, the first fluid_settings of the process registers FluidSynth's audio
    // drivers (CoreAudio, PortAudio), and the system starts two idle libdispatch workers then: once per process,
    // not per instance, and not threads of FluidSynth. So on macOS the count starts after a first instance
    let warm = if cfg!(target_os = "macos") { Some(SynthBuilder::new(SynthConfig::default()).unwrap()) } else { None };
    let before = thread_count();
    let mut synths = Vec::new();
    for outputs in [1, 9] {
        let mut b = SynthBuilder::new(SynthConfig { outputs, ..SynthConfig::default() }).unwrap();
        let id = b.load_soundfont(&font.0).unwrap();
        b.program_select(0, id, 0, 0).unwrap();
        let mut s = b.build();
        s.note_on(0, 69, 127).unwrap();
        render(&mut s, 48_000);
        synths.push(s);
    }
    assert_eq!(thread_count(), before, "FluidSynth made threads");
    drop(synths);
    drop(warm);
}

/// Renders `frames` of the one stereo output with the effects, in blocks of 100 (not a multiple of FluidSynth's 64)
fn render_stereo(s: &mut Synth, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0_f32; frames], vec![0.0_f32; frames]);
    for (a, b) in l.chunks_mut(100).zip(r.chunks_mut(100)) {
        s.render_stereo(a, b).unwrap();
    }
    (l, r)
}

#[test]
fn stereo_render_with_reverb_and_chorus() {
    use lm_sys_fluidsynth::{Chorus, Reverb};
    let mut dry = synth("stereo-dry", SynthConfig::default());
    let mut wet = synth("stereo-wet", SynthConfig::default());
    assert_eq!(dry.internal_block(), 64);
    wet.set_reverb_on(true).unwrap();
    wet.set_reverb(Reverb { room_size: 0.9, damping: 0.1, width: 1.0, level: 1.0 }).unwrap();
    wet.set_chorus_on(true).unwrap();
    wet.set_chorus(Chorus { voices: 3, level: 2.0, speed: 0.3, depth: 8.0 }).unwrap();
    for s in [&mut dry, &mut wet] {
        // (the reverb and chorus sends of the channel, which the test font's zone leaves at their defaults)
        s.cc(0, 91, 127).unwrap();
        s.cc(0, 93, 127).unwrap();
        s.note_on(0, 69, 127).unwrap();
    }
    let (dl, _) = render_stereo(&mut dry, 9_600);
    let (wl, _) = render_stereo(&mut wet, 9_600);
    for s in [&mut dry, &mut wet] {
        s.note_off(0, 69).unwrap();
    }
    // After the note: the dry one is silent at once, the reverb rings on
    let (dt, _) = render_stereo(&mut dry, 24_000);
    let (wt, _) = render_stereo(&mut wet, 24_000);
    assert!(rms(&dl[4_800..]) > 0.05 && rms(&wl[4_800..]) > 0.05);
    assert!(rms(&dt[12_000..]) < 1.0e-6, "the dry tail {}", rms(&dt[12_000..]));
    assert!(rms(&wt[2_400..12_000]) > 1.0e-4, "the reverb's tail {}", rms(&wt[2_400..12_000]));
    // Gain: half the level
    let mut half = synth("stereo-gain", SynthConfig::default());
    half.set_gain(0.5);
    half.note_on(0, 69, 127).unwrap();
    let (hl, _) = render_stereo(&mut half, 9_600);
    let ratio = rms(&hl[4_800..]) / rms(&dl[4_800..]);
    assert!((ratio - 0.5).abs() < 0.01, "gain 0.5 gives {ratio}");
    // A multi-output instance cannot render stereo
    let mut multi = synth("stereo-multi", SynthConfig { outputs: 2, ..SynthConfig::default() });
    let (mut a, mut b) = ([0.0; 8], [0.0; 8]);
    assert!(multi.render_stereo(&mut a, &mut b).is_err());
}

#[test]
fn a_playing_voice_is_retuned_and_a_stale_reference_is_ignored() {
    use lm_sys_fluidsynth::{Generator, VoiceRef};
    let mut s = synth("voices", SynthConfig::default());
    s.set_interpolation(lm_sys_fluidsynth::Interpolation::FourthOrder).unwrap();
    s.note_on(0, 69, 127).unwrap();
    let mut voices = [VoiceRef::default(); 4];
    let n = s.voices_of(0, 69, &mut voices);
    assert_eq!(n, 1, "the test font has one zone");
    assert_eq!(s.voices_of(0, 70, &mut voices[1..]), 0);
    assert_eq!(s.voices_of(1, 69, &mut voices[1..]), 0);
    let coarse = s.voice_gen(&voices[0], Generator::CoarseTune).unwrap();
    assert!(s.set_voice_gen(&voices[0], Generator::CoarseTune, coarse + 12.0));
    let (l, _) = render_stereo(&mut s, 24_000);
    let f = frequency(&l[4_800..], 48_000.0);
    assert!((f - 880.0).abs() < 6.0, "retuned an octave up, plays at {f} Hz");
    // Half a semitone, as the legacy's per-note detune
    assert!(s.set_voice_gen(&voices[0], Generator::CoarseTune, coarse + 0.5));
    let (l, _) = render_stereo(&mut s, 24_000);
    let want = 440.0 * 2f64.powf(0.5 / 12.0);
    let f = frequency(&l[4_800..], 48_000.0);
    assert!((f - want).abs() < 3.0, "a fractional coarse tune plays at {f} Hz, not {want}");
    // The note ends: the reference names a voice that is gone
    s.all_sounds_off(None).unwrap();
    render_stereo(&mut s, 640);
    assert!(!s.set_voice_gen(&voices[0], Generator::CoarseTune, 0.0));
    assert_eq!(s.voice_gen(&voices[0], Generator::CoarseTune), None);
    // A reference from another instance is never followed
    let mut other = synth("voices-other", SynthConfig::default());
    other.note_on(0, 69, 127).unwrap();
    assert!(!other.set_voice_gen(&voices[0], Generator::CoarseTune, 0.0));
    // The bend range
    s.set_bend_range(0, 12).unwrap();
    s.note_on(0, 69, 127).unwrap();
    s.pitch_bend(0, 16_383).unwrap();
    let (l, _) = render_stereo(&mut s, 24_000);
    let f = frequency(&l[4_800..], 48_000.0);
    assert!((f - 880.0).abs() < 8.0, "a full bend of 12 semitones plays at {f} Hz");
}
