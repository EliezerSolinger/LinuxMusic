//! The generated contract suite for the Sampler, playing an audio-file source (the suite makes the audio resource)

fn playing() -> lm_state::Value {
    let key = "7a".repeat(32);
    lm_state::Value::empty_object()
        .with("format", lm_state::Value::text("audio"))
        .with("sample", lm_state::Value::resource(lm_state::Resource::new("audio", &key).expect("a valid key")))
        .with("loop", lm_state::Value::text("on"))
}

const SETUP: lm_plugin::testing::Setup =
    lm_plugin::testing::Setup { resources: None, playing: Some(playing), key_map: None };

lm_plugin::contract_tests!(lm_plugins_sampler::DESCRIPTOR, "sampler", setup = SETUP);
