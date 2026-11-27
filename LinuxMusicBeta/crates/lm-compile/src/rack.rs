//! The desk's rack as program nodes (I-3.9, M5; P-17, P-20, P-21).
//!
//! The rack of a channel is a chain of **stages**, in the signal order of `lm-dsp`'s `ChannelStrip`: the gain stage
//! (`tube`: the GAIN pot, the low cut, the op-amp or tube), the filters (`eq.highPass`, `eq.lowPass`, the mono bass),
//! the dynamics (gate and compressor) and the EQ (the four bands, then the transformer). The inserts sit at the
//! legacy's six points between them (`lm-dsp`'s `InsertPoint`): `pre-gain`, `post-gain`, `post-filter`,
//! `post-dynamics`, `post-eq` (the default: the end of the rack) and `pre-pan` (after the fader).
//!
//! The rack is one processor node per run of stages with no insert between them (one node when every insert is at
//! the same end), so the inserts can go between the sections without the rack knowing about them. Every segment
//! is built by the same rack processor, told which stages it runs, and indexes its parameters in **one** space: the
//! position of the field in [`rack_params`]. The tolerances are drawn from the channel's id (P-21), as the seed of
//! the build. In the digital desk mode (P-17) there is no rack node at all: the channel is clean.
//!
//! The master's rack follows the legacy's order (F-MIX-060): DRIVE, the master's inserts, STEREO and BUS COMP, the
//! fader (POST), then XFMR.

use lm_state::{FieldKind, SchemaRegistry};

/// A stage of a rack, in signal order
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// Channel: the GAIN pot, the low cut and the gain stage (`tube`)
    Gain,
    /// Channel: high-pass, low-pass, mono bass (`eq.highPass`, `eq.lowPass`, `eq.monoBass*`)
    Filter,
    /// Channel: gate and compressor (`dynamics`)
    Dynamics,
    /// Channel: the four EQ bands, then the transformer (`eq.low`..`eq.high`, `transformer`)
    Eq,
    /// Master: the drive (`tube`)
    MasterDrive,
    /// Master: stereo section and bus compressor (`stereo`, `dynamics`)
    MasterBus,
    /// Master: the output transformer, after the fader (`transformer`), then the monitoring (LEVEL, DIM, CUT, MONO,
    /// SIDE: the machine's, `Environment::monitor`, never in an export)
    MasterTransformer,
    /// Channel: the legacy strip's input, TRIM and MONO (and the channel's stereo WIDTH), before the rack and every
    /// insert. Built in both desk modes, when the channel has a `trim` or `width` or `mono` is on
    Input,
    /// Channel: phase invert and swap, after the effects and before the fader (after the pre-fader tap). Built in both
    /// desk modes, when `phaseInvert` or `swap` is on
    Polarity,
}

impl Stage {
    /// The stages of a channel's rack, in order
    pub const CHANNEL: [Stage; 4] = [Stage::Gain, Stage::Filter, Stage::Dynamics, Stage::Eq];

    pub fn key(self) -> &'static str {
        match self {
            Stage::Gain => "gain",
            Stage::Filter => "filter",
            Stage::Dynamics => "dynamics",
            Stage::Eq => "eq",
            Stage::MasterDrive => "master-drive",
            Stage::MasterBus => "master-bus",
            Stage::MasterTransformer => "master-transformer",
            Stage::Input => "input",
            Stage::Polarity => "polarity",
        }
    }
}

/// Where an insert sits (the document's `"point"`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InsertPoint {
    PreGain,
    PostGain,
    PostFilter,
    PostDynamics,
    PostEq,
    PrePan,
}

impl InsertPoint {
    pub fn parse(key: Option<&str>) -> Self {
        match key {
            Some("pre-gain") => InsertPoint::PreGain,
            Some("post-gain") => InsertPoint::PostGain,
            Some("post-filter") => InsertPoint::PostFilter,
            Some("post-dynamics") => InsertPoint::PostDynamics,
            Some("pre-pan") => InsertPoint::PrePan,
            _ => InsertPoint::PostEq,
        }
    }

    /// How many channel stages come before this point
    pub(crate) fn stages_before(self) -> usize {
        match self {
            InsertPoint::PreGain => 0,
            InsertPoint::PostGain => 1,
            InsertPoint::PostFilter => 2,
            InsertPoint::PostDynamics => 3,
            InsertPoint::PostEq | InsertPoint::PrePan => 4,
        }
    }
}

/// One parameter of the rack: a real field of a section
#[derive(Clone, Debug, PartialEq)]
pub struct RackParam {
    /// `"rack.tube.drive"`: the slot table's key (M5)
    pub key: String,
    pub section: &'static str,
    pub field: &'static str,
    /// The stage that uses it
    pub stage: Stage,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

const CHANNEL_SECTIONS: [(&str, &str); 5] = [
    ("tube", lm_doc::schemas::types::TUBE),
    ("eq", lm_doc::schemas::types::EQ),
    ("dynamics", lm_doc::schemas::types::DYNAMICS),
    ("transformer", lm_doc::schemas::types::TRANSFORMER),
    ("tolerance", lm_doc::schemas::types::TOLERANCE),
];

const MASTER_SECTIONS: [(&str, &str); 4] = [
    ("tube", lm_doc::schemas::types::TUBE),
    ("stereo", lm_doc::schemas::types::STEREO),
    ("dynamics", lm_doc::schemas::types::BUS_COMPRESSOR),
    ("transformer", lm_doc::schemas::types::TRANSFORMER),
];

fn stage_of(master: bool, section: &str, field: &str) -> Stage {
    if master {
        return match section {
            "tube" => Stage::MasterDrive,
            "transformer" => Stage::MasterTransformer,
            _ => Stage::MasterBus,
        };
    }
    match (section, field) {
        ("tube", _) => Stage::Gain,
        ("eq", "highPass" | "lowPass" | "monoBass" | "monoBassFrequency") => Stage::Filter,
        ("dynamics", _) => Stage::Dynamics,
        _ => Stage::Eq,
    }
}

/// The channel-level reals the rack's stages run, after the sections' (keys are the document's: `"trim"`, not
/// `"rack.…"`), and the monitor's level on the master
fn extra_params(master: bool) -> Vec<RackParam> {
    let p = |key: &str, section: &'static str, field: &'static str, stage, min, max, default| RackParam {
        key: key.to_owned(),
        section,
        field,
        stage,
        min,
        max,
        default,
    };
    if master {
        vec![
            p("preLeft", "channel", "preLeft", Stage::MasterDrive, 0.0, 4.0, 1.0),
            p("preRight", "channel", "preRight", Stage::MasterDrive, 0.0, 4.0, 1.0),
            p("monitor.level", "monitor", "level", Stage::MasterTransformer, -60.0, 6.0, 0.0),
        ]
    } else {
        vec![
            p("trim", "channel", "trim", Stage::Input, -60.0, 24.0, 0.0),
            p("width", "channel", "width", Stage::Input, 0.0, 200.0, 100.0),
        ]
    }
}

/// The parameters of a channel's rack (`master: false`) or of the master's, in their index order: the real fields
/// of each section, sections and fields in the schema's order, then the channel-level ones ([`extra_params`]). The
/// rack processor uses the same table
pub fn rack_params(registry: &SchemaRegistry, master: bool) -> Vec<RackParam> {
    let sections: &[(&str, &str)] = if master { &MASTER_SECTIONS } else { &CHANNEL_SECTIONS };
    let mut out = Vec::new();
    for (section, type_name) in sections {
        let Some(schema) = registry.find(type_name) else { continue };
        for f in &schema.fields {
            if let FieldKind::Real { min, max } = f.kind {
                let default = f.default.as_ref().and_then(lm_state::Value::as_real).unwrap_or(min);
                out.push(RackParam {
                    key: format!("rack.{section}.{}", f.key),
                    section,
                    field: f.key,
                    stage: stage_of(master, section, f.key),
                    min,
                    max,
                    default,
                });
            }
        }
    }
    out.extend(extra_params(master));
    out
}
