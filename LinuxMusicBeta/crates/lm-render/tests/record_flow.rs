//! The recording transport of the renderer (F-REC-020, F-REC-023, F-REC-024, F-REC-009, F-REC-026): the pre-roll, the
//! playhead going back, the count-in telemetry, the input meters and the metronome that sounds only while recording
#![cfg(not(loom))]

mod common;

use std::sync::Arc;

use common::*;
use lm_render::*;

const BAR: i64 = 96_000; // 120 bpm, 4/4 at 48 kHz

fn program(pre_roll_bars: u32, count_in: Option<CountIn>, metronome: Option<Metronome>) -> Install {
    let mut b = ProgramBuilder::new(ProgramGen(1), spec(), &limits(0), tempo(120));
    let m = master(&mut b);
    let click = n(1);
    b.node(NodeDesc::sum(click, 1));
    b.feed(Feed::new(click, m, 1).mix(ChannelMix::UpMix { to: 2 }));
    if let Some(metronome) = metronome {
        b.metronome(click, metronome);
    }
    b.transport(TransportSettings { count_in, pre_roll_bars, ..TransportSettings::default() });
    Install::new(b.build().expect("program"))
}

fn sounds(only_recording: bool) -> Metronome {
    let accent = Arc::new(SampleData::from_channels(RATE, vec![vec![1.0; 10]]).expect("sample"));
    let beat = Arc::new(SampleData::from_channels(RATE, vec![vec![0.5; 10]]).expect("sample"));
    Metronome { accent, beat, gain: 1.0, enabled: true, only_recording }
}

#[test]
fn pre_roll_starts_bars_earlier_and_records_from_the_playhead() {
    let (mut control, mut dummy) = make(0);
    install(&mut control, program(1, None, None));
    block(&mut dummy, 64);
    control.commands().send(Command::Locate { sample: 2 * BAR }).expect("locate");
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    let mut first_recorded = None;
    let mut first_played = None;
    let mut done = 0;
    while done < 2 * BAR as usize && first_recorded.is_none() {
        let r = dummy.block(512);
        if first_played.is_none() {
            first_played = Some(control.frame().position);
        }
        if let Some(s) = r.recorded().first() {
            first_recorded = Some(s.sample);
        }
        done += 512;
    }
    assert_eq!(first_recorded, Some(2 * BAR), "recording begins at the playhead, after the pre-roll");
    assert!(control.frame().position >= 2 * BAR);
    // Stopping goes back to where the recording was started from
    control.commands().send(Command::Stop).expect("stop");
    block(&mut dummy, 64);
    assert_eq!(control.frame().position, 2 * BAR);
}

#[test]
fn no_pre_roll_before_zero() {
    let (mut control, mut dummy) = make(0);
    install(&mut control, program(2, None, None));
    block(&mut dummy, 64);
    control.commands().send(Command::Locate { sample: BAR / 2 }).expect("locate");
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    let r = dummy.block(64);
    assert!(r.recorded().is_empty() || r.recorded()[0].sample >= BAR / 2);
    assert!(control.frame().position < BAR / 2, "the pre-roll started from the beginning of the song");
}

#[test]
fn the_count_in_is_reported_and_sounds_with_the_metronome_off() {
    let (mut control, mut dummy) = make(0);
    let mut m = sounds(false);
    m.enabled = false;
    install(&mut control, program(0, Some(CountIn { bars: 1, on_play: true }), Some(m)));
    block(&mut dummy, 64);
    control.commands().send(Command::Play).expect("play");
    let out = render(&mut dummy, 1000, 100);
    assert!(out[0][0].abs() > 0.1, "the first click of the count-in");
    // The frame published while counting says so
    let mut counting = false;
    for _ in 0..40 {
        block(&mut dummy, 256);
        let frame = control.frame();
        counting |= frame.counting_in;
        assert!(!frame.pre_roll, "a count-in is not a pre-roll");
    }
    assert!(counting);
}

#[test]
fn the_metronome_set_to_only_recording_is_silent_while_playing() {
    let (mut control, mut dummy) = make(0);
    install(&mut control, program(0, None, Some(sounds(true))));
    control.commands().send(Command::Play).expect("play");
    let out = render(&mut dummy, 48_000, 256);
    assert!(out[0].iter().all(|x| *x == 0.0));
    control.commands().send(Command::Stop).expect("stop");
    control.commands().send(Command::Locate { sample: 0 }).expect("locate");
    control.commands().send(Command::Record { on: true }).expect("record");
    control.commands().send(Command::Play).expect("play");
    let out = render(&mut dummy, 4_800, 256);
    assert!(out[0].iter().any(|x| x.abs() > 0.1), "it sounds while recording");
}

#[test]
fn the_input_peaks_are_published() {
    let (mut control, mut dummy) = make(0);
    dummy.inputs_mut()[1].iter_mut().for_each(|x| *x = 0.25);
    for _ in 0..40 {
        block(&mut dummy, 256);
    }
    let frame = control.frame();
    assert_eq!(frame.inputs[0], 0.0);
    assert!((frame.inputs[1] - 0.25).abs() < 1e-6);
}
