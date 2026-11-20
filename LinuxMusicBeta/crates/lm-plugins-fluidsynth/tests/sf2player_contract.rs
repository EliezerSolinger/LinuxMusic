//! The generated suite for the Sf2 Player, playing the test SoundFont

mod common;

fn playing() -> lm_state::Value {
    lm_state::Value::empty_object()
        .with("soundfont", lm_state::Value::resource(lm_plugins_fluidsynth::sf2player::test_soundfont()))
}

const SETUP: lm_plugin::testing::Setup =
    lm_plugin::testing::Setup { resources: Some(common::test_resources), playing: Some(playing), key_map: None };

lm_plugin::contract_tests!(lm_plugins_fluidsynth::sf2player::DESCRIPTOR, "sf2player", setup = SETUP);
