//! The generated suite for the GigPlayer, playing the test `.gig`

mod common;

fn playing() -> lm_state::Value {
    lm_state::Value::empty_object().with("gig", lm_state::Value::resource(lm_plugins_gigplayer::test_gig()))
}

const SETUP: lm_plugin::testing::Setup =
    lm_plugin::testing::Setup { resources: Some(common::test_resources), playing: Some(playing), key_map: None };

lm_plugin::contract_tests!(lm_plugins_gigplayer::DESCRIPTOR, "gigplayer", setup = SETUP);
