use std::sync::Arc;

use super::*;
use crate::model::{Instrument, OutputGroup, Sequence};
use crate::pcm::Pcm;

const RATE: u32 = 48_000;

fn constant(frames: usize) -> Arc<Pcm> {
    Arc::new(Pcm::mono(RATE, vec![1.0; frames]))
}

fn engine_for(zones: Vec<Zone>, multi: bool, outputs: Vec<OutputGroup>) -> Engine {
    let mut i = Instrument::new("test");
    i.zones = zones;
    if !outputs.is_empty() {
        i.outputs = outputs;
    }
    i.validate().expect("valid");
    let mut cfg = EngineConfig::new(RATE, 256);
    cfg.multi = multi;
    Engine::new(Arc::new(i), cfg)
}

fn zone(sample: Arc<Pcm>) -> Zone {
    let mut z = Zone::new(sample);
    z.root_key = 60;
    z
}

fn peak(e: &mut Engine, frames: usize) -> f32 {
    let mut p = 0.0f32;
    let mut left = frames;
    while left > 0 {
        let n = left.min(256);
        e.render(n);
        let (l, r) = e.bus(0);
        p = l.iter().chain(r.iter()).fold(p, |a, x| a.max(x.abs()));
        left -= n;
    }
    p
}

#[test]
fn a_note_sounds_and_a_one_shot_ignores_the_note_off() {
    let mut z = zone(constant(4800));
    z.loop_mode = LoopMode::OneShot;
    let mut e = engine_for(vec![z], false, Vec::new());
    e.note_on(1, 60, 1.0);
    e.note_off(1);
    assert!(peak(&mut e, 480) > 0.1);
    assert_eq!(e.active_voices(), 1);
    // To its end, then gone
    peak(&mut e, 6000);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn a_note_off_releases_a_looped_note() {
    let mut z = zone(constant(1000));
    z.loop_mode = LoopMode::Forward;
    z.loop_start = 100;
    z.loop_end = 900;
    z.amp.release = 0.01;
    let mut e = engine_for(vec![z], false, Vec::new());
    e.note_on(1, 60, 1.0);
    assert!(peak(&mut e, 4800) > 0.1);
    assert_eq!(e.active_voices(), 1);
    e.note_off(1);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn key_and_velocity_ranges_select_zones() {
    let mut a = zone(constant(4800));
    a.keys = (0, 59);
    let mut b = zone(constant(4800));
    b.keys = (60, 127);
    b.vels = (64, 127);
    let mut e = engine_for(vec![a, b], false, Vec::new());
    e.note_on(1, 60, 0.2);
    assert_eq!(e.active_voices(), 0);
    e.note_on(2, 60, 1.0);
    assert_eq!(e.active_voices(), 1);
    e.note_on(3, 10, 1.0);
    assert_eq!(e.active_voices(), 2);
}

#[test]
fn round_robin_cycles_through_the_positions() {
    let zones: Vec<Zone> = (1..=3)
        .map(|p| {
            let mut z = zone(Arc::new(Pcm::mono(RATE, vec![p as f32; 4800])));
            z.sequence = Some(Sequence { group: 0, length: 3, position: p });
            z.vel_track = 0.0;
            z
        })
        .collect();
    let mut e = engine_for(zones, false, Vec::new());
    let mut seen = Vec::new();
    for n in 0..6 {
        e.all_sound_off();
        e.note_on(n + 1, 60, 1.0);
        e.render(64);
        let (l, _) = e.bus(0);
        seen.push((l[63] * 100.0).round() as i32);
    }
    // Constant power pan: the center is 0.707 of the sample's value; the ratio between turns is what counts
    assert!(seen[0] < seen[1] && seen[1] < seen[2], "{seen:?}");
    assert_eq!(seen[0], seen[3]);
    assert_eq!(seen[1], seen[4]);
}

#[test]
fn random_ranges_are_deterministic_for_a_seed() {
    let mk = || {
        let mut lo = zone(constant(4800));
        lo.random = Some((0.0, 0.5));
        let mut hi = zone(constant(4800));
        hi.random = Some((0.5, 1.0));
        hi.pan = 1.0;
        engine_for(vec![lo, hi], false, Vec::new())
    };
    let (mut a, mut b) = (mk(), mk());
    for n in 0..20 {
        a.note_on(n + 1, 60, 1.0);
        b.note_on(n + 1, 60, 1.0);
        a.render(32);
        b.render(32);
        assert_eq!(a.bus(0).0, b.bus(0).0);
        // Exactly one of the two zones plays each time
    }
    assert_eq!(a.active_voices(), 20);
}

#[test]
fn an_exclusive_group_chokes() {
    let mut open = zone(constant(48_000));
    open.group = 1;
    open.off_by = 1;
    open.off_fast = true;
    open.loop_mode = LoopMode::OneShot;
    let mut e = engine_for(vec![open], false, Vec::new());
    e.note_on(1, 60, 1.0);
    peak(&mut e, 256);
    e.note_on(2, 60, 1.0);
    peak(&mut e, 2400);
    // The first fell away in its few milliseconds; the second goes on
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn the_polyphony_steals_without_exceeding_it() {
    let mut z = zone(constant(480_000));
    z.loop_mode = LoopMode::Forward;
    z.loop_start = 0;
    z.loop_end = 1000;
    let mut e = engine_for(vec![z], false, Vec::new());
    let mut p = *e.params();
    p.polyphony = 4;
    e.set_params(p);
    for n in 0..16 {
        e.note_on(n + 1, 60 + (n % 12) as u8, 1.0);
        e.render(64);
    }
    // Let the stolen ones finish their short fade
    for _ in 0..4 {
        e.render(256);
    }
    assert!(e.active_voices() <= 4, "{}", e.active_voices());
}

#[test]
fn release_triggers_play_on_the_note_off() {
    let mut attack = zone(constant(4800));
    attack.loop_mode = LoopMode::Off;
    let mut release = zone(Arc::new(Pcm::mono(RATE, vec![0.5; 4800])));
    release.trigger = Trigger::Release;
    let mut e = engine_for(vec![attack, release], false, Vec::new());
    e.note_on(1, 60, 1.0);
    assert_eq!(e.active_voices(), 1);
    e.note_off(1);
    // The attack voice fades; the release zone's voice is new
    assert_eq!(e.voices.iter().filter(|v| v.active && v.zone == 1).count(), 1);
}

#[test]
fn the_pedal_holds_the_release() {
    let mut z = zone(constant(48_000));
    z.loop_mode = LoopMode::Forward;
    z.loop_end = 1000;
    z.amp.release = 0.005;
    let mut e = engine_for(vec![z], false, Vec::new());
    e.cc(64, 127);
    e.note_on(1, 60, 1.0);
    e.note_off(1);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 1);
    e.cc(64, 0);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn an_octave_up_plays_twice_as_fast() {
    // A ramp: the position after 100 frames at key 72 is 200 frames
    let ramp: Vec<f32> = (0..10_000).map(|i| i as f32 / 10_000.0).collect();
    let z = zone(Arc::new(Pcm::mono(RATE, ramp)));
    let mut e = engine_for(vec![z], false, Vec::new());
    e.note_on(1, 72, 1.0);
    e.render(100);
    let v = e.voices.iter().find(|v| v.active).expect("a voice");
    assert!((v.pos - 200.0).abs() < 1.0e-6, "{}", v.pos);
}

#[test]
fn multi_output_separates_the_groups() {
    let mut a = zone(constant(4800));
    a.output = 0;
    let mut b = zone(constant(4800));
    b.output = 1;
    b.keys = (61, 61);
    a.keys = (60, 60);
    let outs = vec![OutputGroup { name: "a".into(), stereo: true }, OutputGroup { name: "b".into(), stereo: false }];
    let mut e = engine_for(vec![a, b], true, outs);
    assert_eq!(e.groups(), 2);
    e.note_on(1, 61, 1.0);
    e.render(64);
    let (l0, _) = e.bus(0);
    let (l1, _) = e.bus(1);
    assert!(l0.iter().all(|x| *x == 0.0));
    assert!(l1.iter().any(|x| *x != 0.0));
}

#[test]
fn rendering_does_not_allocate() {
    let mut z = zone(constant(4800));
    z.loop_mode = LoopMode::PingPong;
    z.loop_end = 2000;
    z.filter = Some(crate::model::Filter { cutoff: 2000.0, ..Default::default() });
    let mut e = engine_for(vec![z], false, Vec::new());
    assert_no_alloc::assert_no_alloc(|| {
        for n in 0..8 {
            e.note_on(n + 1, 50 + n as u8, 0.8);
            e.render(256);
        }
        e.note_off(1);
        e.cc(64, 127);
        e.render(256);
        e.all_notes_release();
        e.render(256);
    });
}

#[test]
fn ping_pong_stays_inside_the_loop() {
    let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
    let mut z = zone(Arc::new(Pcm::mono(RATE, ramp)));
    z.loop_mode = LoopMode::PingPong;
    z.loop_start = 100;
    z.loop_end = 200;
    z.start = 150;
    let mut e = engine_for(vec![z], false, Vec::new());
    e.note_on(1, 60, 1.0);
    for _ in 0..40 {
        e.render(256);
        let v = e.voices.iter().find(|v| v.active).expect("still playing");
        assert!(v.pos >= 99.0 && v.pos <= 201.0, "{}", v.pos);
    }
}

#[test]
fn window_bounds_follow_the_audio_file_processors_rule() {
    use crate::model::Window;
    assert_eq!(window_bounds(1000, &Window { start: 0.2, end: 0.8, loop_start: 0.5 }), (200, 800, 500));
    // Reversed knobs are put in order, a frame apart at least
    let (s, e, _) = window_bounds(1000, &Window { start: 0.9, end: 0.1, loop_start: 0.0 });
    assert!(s < e);
    let (s, e, _) = window_bounds(1000, &Window { start: 0.5, end: 0.5, loop_start: 0.0 });
    assert_eq!(e - s, 1);
}

fn looped() -> Zone {
    let mut z = zone(constant(1000));
    z.loop_mode = LoopMode::Forward;
    z.loop_start = 100;
    z.loop_end = 900;
    z.amp.release = 0.01;
    z
}

fn mono_params(legato: bool, glide: f32) -> Params {
    Params { mono: true, legato, glide, ..Params::default() }
}

#[test]
fn mono_ends_the_sounding_note_and_brings_the_held_one_back() {
    let mut e = engine_for(vec![looped()], false, Vec::new());
    e.set_params(mono_params(false, 0.0));
    e.note_on(1, 60, 1.0);
    peak(&mut e, 480);
    e.note_on(2, 64, 1.0);
    // The first voice falls fast; only the second stays
    peak(&mut e, 960);
    assert_eq!(e.active_voices(), 1);
    // Releasing the second brings the first back, as a new note
    e.note_off(2);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 1, "the held note sounds again");
    e.note_off(1);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn legato_keeps_the_voice_and_moves_its_pitch() {
    let mut e = engine_for(vec![looped()], false, Vec::new());
    e.set_params(mono_params(true, 0.0));
    e.note_on(1, 60, 1.0);
    peak(&mut e, 480);
    e.note_on(2, 72, 1.0);
    peak(&mut e, 480);
    // One voice, an octave up, with no new attack
    assert_eq!(e.active_voices(), 1);
    assert!((e.voice_cents(0).expect("a voice") - 1200.0).abs() < 1.0e-3);
    // The note-off of the first (no longer sounding) changes nothing; the second's ends the voice
    e.note_off(1);
    peak(&mut e, 480);
    assert_eq!(e.active_voices(), 1);
    e.note_off(2);
    peak(&mut e, 4800);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn glide_closes_the_distance_in_the_time_asked() {
    let mut e = engine_for(vec![looped()], false, Vec::new());
    e.set_params(mono_params(true, 0.1));
    e.note_on(1, 60, 1.0);
    peak(&mut e, 480);
    e.note_on(2, 72, 1.0);
    // Just after the move the voice is still near the first pitch
    e.render(32);
    let early = e.voice_glide(0).expect("a voice");
    assert!(early < -1000.0, "the glide starts an octave below: {early}");
    // 0.1 s later it has arrived
    peak(&mut e, 5000);
    assert_eq!(e.voice_glide(0), Some(0.0));
}

#[test]
fn poly_with_no_glide_is_what_it_was() {
    let mut e = engine_for(vec![looped()], false, Vec::new());
    e.note_on(1, 60, 1.0);
    e.note_on(2, 64, 1.0);
    peak(&mut e, 480);
    assert_eq!(e.active_voices(), 2);
}

/// One scripted run: events at absolute frames, the stream cut into the given block lengths (repeating)
fn scripted(cuts: &[usize], total: usize) -> Vec<f32> {
    let mut saw: Vec<f32> = (0..4000).map(|i| ((i % 100) as f32 / 50.0) - 1.0).collect();
    saw.extend((0..4000).map(|i| ((i % 37) as f32 / 18.0) - 1.0));
    let mut a = zone(Arc::new(Pcm::mono(RATE, saw.clone())));
    a.loop_mode = LoopMode::Forward;
    a.loop_start = 500;
    a.loop_end = 7000;
    a.pan = -0.4;
    a.filter = Some(crate::model::Filter {
        cutoff: 2000.0,
        resonance: 6.0,
        key_track: 50.0,
        env: Ahdsr { attack: 0.01, decay: 0.2, sustain: 0.3, ..Ahdsr::default() },
        env_depth: 2400.0,
        ..crate::model::Filter::default()
    });
    let mut b = zone(Arc::new(Pcm::stereo(RATE, saw.clone(), saw.iter().rev().copied().collect())));
    b.keys = (64, 127);
    b.loop_mode = LoopMode::Forward;
    b.loop_start = 100;
    b.loop_end = 7900;
    let mut e = engine_for(vec![a, b], false, Vec::new());
    e.set_params(Params { glide: 0.05, mono: false, pan: 0.2, ..Params::default() });
    // (frame, action)
    let script: [(usize, u8); 6] = [(7, 0), (45, 1), (300, 2), (1000, 3), (1500, 4), (2501, 5)];
    let mut out = Vec::with_capacity(total);
    let (mut at, mut c) = (0usize, 0usize);
    while at < total {
        // Cut also at the events (a host renders up to each event), then at the host's own cuts
        let mut n = cuts[c % cuts.len()].min(total - at);
        c += 1;
        let mut done = 0;
        while done < n {
            let abs = at + done;
            for (f, act) in script {
                if f == abs {
                    match act {
                        0 => e.note_on(1, 60, 0.8),
                        1 => e.note_on(2, 67, 1.0),
                        2 => e.pitch_bend(0.5),
                        3 => e.note_off(1),
                        4 => e.note_on(3, 72, 0.5),
                        _ => e.note_off(2),
                    }
                }
            }
            let next_event = script.iter().map(|(f, _)| *f).filter(|f| *f > abs).min().unwrap_or(usize::MAX);
            let m = (n - done).min(next_event - abs);
            e.render(m);
            let (l, r) = e.bus(0);
            out.extend(l.iter().zip(r).flat_map(|(l, r)| [*l, *r]));
            done += m;
        }
        n = done;
        at += n;
    }
    out
}

/// M1: the output is the same for any way the host cuts the stream (the grid of per-voice updates is the engine's
/// absolute frame, not the start of each call)
#[test]
fn the_output_does_not_depend_on_the_host_block_size() {
    let a = scripted(&[64], 4096);
    let b = scripted(&[33, 31], 4096);
    let c = scripted(&[1], 4096);
    let d = scripted(&[256, 7, 100, 3], 4096);
    assert!(a.iter().any(|x| x.abs() > 0.01), "it sounds");
    assert_eq!(a, b);
    assert_eq!(a, c);
    assert_eq!(a, d);
}
