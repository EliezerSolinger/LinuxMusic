//! Instantiation, ports and run against the test bundle

mod common;
use common::bundle::{self, GAIN, REFUSES, UNOBTAINIUM, port};

use lm_sys_lilv::{Instance, InstanceConfig};

fn gain(config: &InstanceConfig) -> Instance {
    let mut i = bundle::world().instantiate(GAIN, config).unwrap();
    i.activate();
    i
}

fn run(i: &mut Instance, input: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0_f32; input.len()];
    i.run(input.len(), &[input], &mut [&mut out]).unwrap();
    out
}

#[test]
fn the_gain_runs_with_the_options_the_host_gave() {
    let mut i = gain(&InstanceConfig::new(44_100.0, 128));
    assert_eq!(i.audio_inputs(), &[port::IN]);
    assert_eq!(i.audio_outputs(), &[port::OUT]);
    assert!(i.has_worker() && i.has_state());
    assert_eq!(i.control(port::GAIN), Some(1.0), "starts at the default");
    let input: Vec<f32> = (0..128).map(|n| (n as f32 * 0.1).sin()).collect();
    assert_eq!(run(&mut i, &input), input);
    i.set_control(port::GAIN, 0.5).unwrap();
    let out = run(&mut i, &input);
    assert!(out.iter().zip(&input).all(|(o, x)| *o == x * 0.5));
    assert_eq!(i.control(port::RATE), Some(44_100.0), "the plugin saw param:sampleRate");
    assert_eq!(i.control(port::MAX_BLOCK), Some(128.0), "the plugin saw bufsz:maxBlockLength");
    assert_eq!(i.latency(), Some(7));
    assert_eq!(i.control(port::END_RUNS), Some(1.0), "end_run is called after every run");
}

#[test]
fn controls_are_clamped_and_checked() {
    let mut i = gain(&bundle::config());
    i.set_control(port::GAIN, 5.0).unwrap();
    assert_eq!(i.control(port::GAIN), Some(2.0));
    i.set_control(port::GAIN, -1.0).unwrap();
    assert_eq!(i.control(port::GAIN), Some(0.0));
    assert_eq!(i.set_control(port::LATENCY, 1.0).unwrap_err().key(), "lilv.not_control_input", "an output");
    assert_eq!(i.set_control(port::IN, 1.0).unwrap_err().key(), "lilv.not_control_input", "an audio port");
    assert_eq!(i.set_control(99, 1.0).unwrap_err().key(), "lilv.not_control_input");
    assert_eq!(i.set_control(port::GAIN, f32::NAN).unwrap_err().key(), "lilv.not_control_input");
    assert_eq!(i.control(port::IN), None);
}

#[test]
fn midi_goes_in_and_events_come_out() {
    let mut i = gain(&bundle::config());
    i.push_midi(port::MIDI_IN, 0, &[0x90, 60, 100]).unwrap();
    i.push_midi(port::MIDI_IN, 10, &[0x80, 60, 0]).unwrap();
    i.push_midi(port::MIDI_IN, 200, &[0x90, 64, 1]).unwrap();
    let sysex = [0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7];
    i.push_midi(port::MIDI_IN, 200, &sysex).unwrap();
    assert_eq!(i.push_midi(port::MIDI_IN, 5, &[0x90, 1, 1]).unwrap_err().key(), "lilv.event_order");
    let input = vec![0.0_f32; 256];
    run(&mut i, &input);
    assert_eq!(i.control(port::NOTES), Some(2.0));
    let events: Vec<(i64, Vec<u8>)> = i
        .output_events(port::EVENTS_OUT)
        .unwrap()
        .inspect(|e| assert_eq!(e.type_urid, i.urids().midi_event))
        .map(|e| (e.frame, e.data.to_vec()))
        .collect();
    assert_eq!(
        events,
        vec![(0, vec![0x90, 60, 100]), (10, vec![0x80, 60, 0]), (200, vec![0x90, 64, 1]), (200, sysex.to_vec())]
    );
    // The input is emptied after each run: the next run sees nothing new
    run(&mut i, &input);
    assert_eq!(i.control(port::NOTES), Some(2.0));
    assert_eq!(i.output_events(port::EVENTS_OUT).unwrap().count(), 0);

    assert_eq!(i.push_midi(port::EVENTS_OUT, 0, &[0x90, 1, 1]).unwrap_err().key(), "lilv.not_atom_input");
    assert_eq!(i.output_events(port::MIDI_IN).unwrap_err().key(), "lilv.not_atom_output");
}

#[test]
fn an_atom_input_that_is_full_refuses_more() {
    let mut config = bundle::config();
    config.atom_capacity = 64;
    let mut i = bundle::world().instantiate(GAIN, &config).unwrap();
    // The port's rsz:minimumSize (16 KiB) wins over the configured 64 bytes
    let mut pushed = 0;
    while i.push_midi(port::MIDI_IN, 0, &[0x90, 60, 100]).is_ok() {
        pushed += 1;
    }
    assert_eq!(pushed, (16384 - 16) / 24, "16-byte header, 24 bytes per 3-byte event");
    assert_eq!(i.push_midi(port::MIDI_IN, 0, &[0x90, 60, 100]).unwrap_err().key(), "lilv.atom_full");
}

#[test]
fn run_checks_the_block_and_the_buffers() {
    let mut i = bundle::world().instantiate(GAIN, &bundle::config()).unwrap();
    let input = vec![0.0_f32; 512];
    let mut out = vec![0.0_f32; 512];
    assert_eq!(i.run(64, &[&input], &mut [&mut out]).unwrap_err().key(), "lilv.not_active");
    i.activate();
    assert!(i.is_active());
    assert_eq!(i.run(257, &[&input], &mut [&mut out]).unwrap_err().key(), "lilv.bad_block");
    assert_eq!(i.run(0, &[&input], &mut [&mut out]).unwrap_err().key(), "lilv.bad_block", "below min_block");
    assert_eq!(i.run(64, &[], &mut [&mut out]).unwrap_err().key(), "lilv.bad_buffers");
    assert_eq!(i.run(64, &[&input[..10]], &mut [&mut out]).unwrap_err().key(), "lilv.bad_buffers");
    i.run(64, &[&input], &mut [&mut out]).unwrap();
    i.deactivate();
    assert!(!i.is_active());

    let mut fixed = bundle::config();
    fixed.fixed_block = true;
    fixed.power_of_two_block = true;
    let mut i = gain(&fixed);
    assert_eq!(i.run(128, &[&input], &mut [&mut out]).unwrap_err().key(), "lilv.bad_block", "fixed at 256");
    i.run(256, &[&input], &mut [&mut out]).unwrap();
}

#[test]
fn plugins_that_cannot_be_made_are_errors() {
    let world = bundle::world();
    let err = world.instantiate(UNOBTAINIUM, &bundle::config()).unwrap_err();
    assert_eq!(err.key(), "lilv.missing_feature");
    assert!(err.to_string().contains("urn:linuxmusic:test#unobtainium"), "{err}");
    assert_eq!(world.instantiate(REFUSES, &bundle::config()).unwrap_err().key(), "lilv.instantiate_failed");
    assert_eq!(world.instantiate("urn:nothing", &bundle::config()).unwrap_err().key(), "lilv.no_plugin");
    let mut bad = bundle::config();
    bad.sample_rate = 0.0;
    assert_eq!(world.instantiate(GAIN, &bad).unwrap_err().key(), "lilv.bad_config");
    let mut bad = bundle::config();
    bad.power_of_two_block = true;
    bad.max_block = 100;
    assert_eq!(world.instantiate(GAIN, &bad).unwrap_err().key(), "lilv.bad_config");
    let mut bad = bundle::config();
    bad.power_of_two_block = false;
    bad.max_block = (1 << 20) + 1;
    assert_eq!(world.instantiate(GAIN, &bad).unwrap_err().key(), "lilv.bad_config");
}

#[test]
fn an_instance_moves_to_another_thread_and_outlives_its_world_handle() {
    let mut i = {
        let world = bundle::world();
        world.instantiate(GAIN, &bundle::config()).unwrap()
        // The World handle is dropped here; the instance keeps the world alive
    };
    i.activate();
    let i = std::thread::spawn(move || {
        let input = vec![0.25_f32; 256];
        assert_eq!(run(&mut i, &input), input);
        i
    })
    .join()
    .unwrap();
    drop(i);
}

#[test]
fn many_instances_of_one_world() {
    let world = bundle::world();
    let mut all: Vec<Instance> = (0..8).map(|_| world.instantiate(GAIN, &bundle::config()).unwrap()).collect();
    for (n, i) in all.iter_mut().enumerate() {
        i.activate();
        i.set_control(port::GAIN, n as f32 / 4.0).unwrap();
    }
    let input = vec![1.0_f32; 32];
    for (n, i) in all.iter_mut().enumerate() {
        assert_eq!(run(i, &input)[0], n as f32 / 4.0);
    }
}
