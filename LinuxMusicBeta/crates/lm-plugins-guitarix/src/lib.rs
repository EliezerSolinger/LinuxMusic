//! Guitarix's units as effect plugins (F-FX: the legacy program's `plugins/Guitarix`): Guitarix's pedals, amp head,
//! cabinets, modulations, delays and reverbs, their DSP Guitarix's own code (`lm-sys-guitarix`), one plugin per unit
//! (the legacy plugin's sub-plugins), each with its own `static Descriptor` in [`units`].
//!
//! - **Ids:** `gx-<the unit's Guitarix id, lower case, every other character a '-'>` (`gx-ts9sim`,
//!   `gx-amp-tonestack`, `gx-gcb-95`). Permanent.
//! - **State, version 1:** the unit's registered parameters, keyed by Guitarix's id with every character other than
//!   a letter, a digit or `_` made `_` (`ts9sim_drive`, `amp_tonestack_Middle`, `ring_modulator_dry_wet`), with
//!   Guitarix's ranges and defaults. A slider is a parameter (logarithmic when Guitarix's is, whole steps when its
//!   step is whole); a switch a stepped parameter of 0 to 1; a selector (a list of named values: the tonestacks, the
//!   amps' tubes, the cabinets, a wah's model, a mode) a `Choice` field keyed by the values' Guitarix ids (U30).
//!   The few settings the unit reads only when it starts (a cabinet's or an amp impulse's response, the amp head's
//!   presence level, the Detune's latency setting) are fields too: changing one builds the unit anew ([`rebuilds`]).
//! - **Outputs** (Guitarix's meters: the amp head's levels, the compressors' and the graphic EQ's bands) are not
//!   parameters: they are the plugin window's live frame ([`outputs`], [`DISPLAY_BYTES`]).
//! - **Wet only:** the dry/wet mix is the host's (the legacy plugin mixed it itself, with the effect's W/D knob).
//!
//! How each unit runs (one instance per channel for a mono unit, one for a pair for a stereo one; pieces on a grid of
//! 32 samples, or a fixed block with its latency for the units whose sound depends on the size of each call) is in
//! [`processor`].

#![forbid(unsafe_code)]

pub mod processor;
mod schema;

pub use schema::{DISPLAY_BYTES, Plan, fixed_block, key_of, name_of, outputs, plan, rebuilds};

macro_rules! units {
    ($($module:ident = $unit:literal => $id:literal $([$($no_adopt:literal),*])?,)*) => {
        /// One module per unit, each with its `DESCRIPTOR`
        pub mod units {
            $(
                pub mod $module {
                    use lm_plugin::{Descriptor, DisplaySpec, PluginKind, TestHints, Velocity};

                    /// Guitarix's id of the unit
                    pub const UNIT: &str = $unit;
                    pub const ID: &str = $id;

                    pub static DESCRIPTOR: Descriptor = Descriptor {
                        id: ID,
                        name: concat!("plugin.", $id, ".name"),
                        kind: PluginKind::Effect,
                        schemas: || crate::schema::schemas(UNIT),
                        ports: crate::schema::no_ports,
                        velocity: Velocity::Ignored,
                        display: Some(DisplaySpec { bytes: crate::DISPLAY_BYTES }),
                        build: |state, cx| crate::processor::build(UNIT, state, cx),
                        tests: TestHints { tuning: None, no_adopt: &[$($($no_adopt),*)?], voices: 16 },
                    };
                }
            )*
        }

        /// Every unit, in the legacy browser's order: (Guitarix's id, descriptor)
        pub static ALL: &[(&str, &lm_plugin::Descriptor)] = &[$(($unit, &units::$module::DESCRIPTOR)),*];
    };
}

units! {
    gain = "gain" => "gx-gain",
    low_highpass = "low_highpass" => "gx-low-highpass",
    highbooster = "highbooster" => "gx-highbooster",
    eqs = "eqs" => "gx-eqs",
    ampstack = "ampstack" => "gx-ampstack",
    crybaby = "crybaby" => "gx-crybaby",
    poweramp = "poweramp" => "gx-poweramp",
    wah = "wah" => "gx-wah",
    smbpitchshift = "smbPitchShift" => "gx-smbpitchshift",
    gx_distortion = "gx_distortion" => "gx-gx-distortion",
    bitdowner = "bitdowner" => "gx-bitdowner",
    thick_distortion = "thick_distortion" => "gx-thick-distortion",
    ts9sim = "ts9sim" => "gx-ts9sim",
    aclipper = "aclipper" => "gx-aclipper",
    mxrdis = "mxrdis" => "gx-mxrdis",
    bossds1 = "bossds1" => "gx-bossds1",
    bmp = "bmp" => "gx-bmp",
    ir = "IR" => "gx-ir",
    compressor = "compressor" => "gx-compressor",
    expander = "expander" => "gx-expander",
    overdrive = "overdrive" => "gx-overdrive",
    echo = "echo" => "gx-echo",
    delay = "delay" => "gx-delay",
    freeverb = "freeverb" => "gx-freeverb",
    biquad = "biquad" => "gx-biquad",
    tremolo = "tremolo" => "gx-tremolo",
    phaser_mono = "phaser_mono" => "gx-phaser-mono",
    chorus_mono = "chorus_mono" => "gx-chorus-mono",
    flanger_mono = "flanger_mono" => "gx-flanger-mono",
    flanger_mono_gx = "flanger_mono_gx" => "gx-flanger-mono-gx",
    feedback = "feedback" => "gx-feedback",
    eq = "eq" => "gx-eq",
    dide = "dide" => "gx-dide",
    amp_tonestack = "amp.tonestack" => "gx-amp-tonestack",
    cab = "cab" => "gx-cab",
    pre = "pre" => "gx-pre",
    abgate = "abgate" => "gx-abgate",
    ring_modulator = "ring_modulator" => "gx-ring-modulator",
    graphiceq = "graphiceq" => "gx-graphiceq",
    univibe_mono = "univibe_mono" => "gx-univibe-mono",
    mbc = "mbc" => "gx-mbc",
    mbd = "mbd" => "gx-mbd",
    mbe = "mbe" => "gx-mbe",
    mbdel = "mbdel" => "gx-mbdel",
    mbclip = "mbclip" => "gx-mbclip",
    bboom = "bboom" => "gx-bboom",
    mbchor = "mbchor" => "gx-mbchor",
    gcb_95 = "GCB_95" => "gx-gcb-95",
    duckdelay = "duckDelay" => "gx-duckdelay",
    reversedelay = "reversedelay" => "gx-reversedelay",
    baxandall = "baxandall" => "gx-baxandall",
    distortion2 = "distortion2" => "gx-distortion2",
    fuzzface = "fuzzface" => "gx-fuzzface",
    trbuff = "trbuff" => "gx-trbuff",
    fuzzfacefm = "fuzzfacefm" => "gx-fuzzfacefm",
    fuzzfacerm = "fuzzfacerm" => "gx-fuzzfacerm",
    hornet = "hornet" => "gx-hornet",
    susta = "susta" => "gx-susta",
    hfb = "hfb" => "gx-hfb",
    muff = "muff" => "gx-muff",
    scream = "scream" => "gx-scream",
    lpbboost = "lpbboost" => "gx-lpbboost",
    foxeylady = "foxeylady" => "gx-foxeylady",
    hogsfoot = "hogsfoot" => "gx-hogsfoot",
    cstbt1 = "cstbt1" => "gx-cstbt1",
    coverd = "coverd" => "gx-coverd",
    rangem = "rangem" => "gx-rangem",
    ruin = "ruin" => "gx-ruin",
    ffreak = "ffreak" => "gx-ffreak",
    fumaster = "fumaster" => "gx-fumaster",
    fuzzdrive = "fuzzdrive" => "gx-fuzzdrive",
    rolandwah = "rolandwah" => "gx-rolandwah",
    astrofuzz = "astrofuzz" => "gx-astrofuzz",
    buffb = "buffb" => "gx-buffb",
    eldist = "eldist" => "gx-eldist",
    mole = "mole" => "gx-mole",
    buzz = "buzz" => "gx-buzz",
    bfuzz = "bfuzz" => "gx-bfuzz",
    axface = "axface" => "gx-axface",
    chorus = "chorus" => "gx-chorus",
    flanger = "flanger" => "gx-flanger",
    phaser = "phaser" => "gx-phaser",
    stereodelay = "stereodelay" => "gx-stereodelay",
    stereoecho = "stereoecho" => "gx-stereoecho",
    moog = "moog" => "gx-moog",
    ampmodul = "ampmodul" => "gx-ampmodul",
    tonemodul = "tonemodul" => "gx-tonemodul",
    didest = "didest" => "gx-didest",
    stereoverb = "stereoverb" => "gx-stereoverb",
    zita_rev1 = "zita_rev1" => "gx-zita-rev1",
    univibe = "univibe" => "gx-univibe",
    mbcs = "mbcs" => "gx-mbcs",
    dattorros_progenitor = "dattorros_progenitor" => "gx-dattorros-progenitor",
    panoram_enhancer = "panoram_enhancer" => "gx-panoram-enhancer",
    bassenhancer = "bassEnhancer" => "gx-bassenhancer",
    ringmodulatorst = "ringModulatorSt" => "gx-ringmodulatorst",
    duckdelayst = "duckDelaySt" => "gx-duckdelayst",
}

/// Every unit's descriptor, for the registry
pub fn descriptors() -> Vec<&'static lm_plugin::Descriptor> {
    ALL.iter().map(|(_, d)| *d).collect()
}
