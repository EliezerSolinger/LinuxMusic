//! The generated suite for AvlDrums, with the kits of the legacy tree's data (`kits_dir`)

/// The suite's keys land on the pieces every kit has (36 to 55)
fn on_the_kit(key: u8) -> u8 {
    36 + key % 20
}

const SETUP: lm_plugin::testing::Setup =
    lm_plugin::testing::Setup { resources: None, playing: None, key_map: Some(on_the_kit) };

lm_plugin::contract_tests!(lm_plugins_fluidsynth::avldrums::DESCRIPTOR, "avldrums", setup = SETUP);
