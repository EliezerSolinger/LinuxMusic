//! The dynamics of a desk channel: compressors modelled on real units, and the SSL E's gate/expander (legacy
//! `DeskDynamics`). The texts the panel shows are the interface's; here are the numbers behind them.

use crate::filters::{Biquad, RampedBiquad};
use crate::math::{amp_to_dbfs, dbfs_to_amp, f32c, log1pf, log10f, logf, one_pole, powf, tanhf};
use crate::oversampling::{Downsampler, Upsampler};
use crate::tolerance::{OutputTransformer, TransformerCore};

/// Levels in dBu, as on the desk, and in dBFS: 0 VU, the desk's nominal +4 dBu, is -18 dBFS RMS, the usual
/// alignment
pub const DBU_TO_DBFS: f32 = -22.0;

/// dB
const SILENCE: f32 = -200.0;
/// dBFS RMS, 0 VU
const NOMINAL_RMS: f32 = 4.0 + DBU_TO_DBFS;
const LN10_OVER_20: f32 = 0.115_129_255;

/// The amplitude of a sine at 0 VU
fn nominal_peak() -> f32 {
    dbfs_to_amp(NOMINAL_RMS) * f32c::SQRT_2
}

fn level_of(amplitude: f32) -> f32 {
    if amplitude > 1e-10 { amp_to_dbfs(amplitude) } else { SILENCE }
}

/// A position 0 to 1 on a logarithmic scale through @p low, @p middle (at 0.5) and @p high
pub fn log_scale(position: f32, low: f32, middle: f32, high: f32) -> f32 {
    let position = position.clamp(0.0, 1.0);
    if position < 0.5 {
        low * powf(middle / low, position * 2.0)
    } else {
        middle * powf(high / middle, (position - 0.5) * 2.0)
    }
}

/// The inverse of [`log_scale`]
pub fn log_position(value: f32, low: f32, middle: f32, high: f32) -> f32 {
    let value = value.clamp(low, high);
    if value < middle {
        logf(value / low) / logf(middle / low) / 2.0
    } else {
        0.5 + logf(value / middle) / logf(high / middle) / 2.0
    }
}

/// A soft knee of width @p width around 0: 0 below, @p over above, quadratic between
fn knee(over: f32, width: f32) -> f32 {
    if width <= 0.0 {
        return over.max(0.0);
    }
    if over <= -width / 2.0 {
        return 0.0;
    }
    if over >= width / 2.0 {
        return over;
    }
    let into = over + width / 2.0;
    into * into / (2.0 * width)
}

/// The API 2500's ratio steps
pub const API_RATIOS: [f32; 7] = [1.5, 2.0, 3.0, 4.0, 6.0, 10.0, f32::INFINITY];
/// The 1176's ratio buttons; 0: all buttons in
pub const UREI_RATIOS: [f32; 5] = [4.0, 8.0, 12.0, 20.0, 0.0];
/// The 1176's fixed thresholds per ratio, dBu (late UREI manual, +-2 dB)
const UREI_THRESHOLDS: [f32; 5] = [-30.0, -26.0, -25.0, -24.0, -24.0];

/// The step of a stepped ratio a position 0 to 1 means
pub fn step(ratio: f32, steps: usize) -> usize {
    ((ratio.clamp(0.0, 1.0) * steps as f32) as usize).min(steps - 1)
}

/// The Fairchild 670's time constant positions (1959 manual; RC values after Raffensperger, DAFx-12)
#[derive(Clone, Copy, Debug)]
pub struct TimeConstant {
    /// s
    pub attack: f32,
    /// uF, of the main capacitor
    pub capacitor: f32,
    /// kOhm, its discharge
    pub resistor: f32,
    /// Series branches: kOhm, uF (0 for none)
    pub branches: [[f32; 2]; 2],
    /// s, the release the manual gives
    pub documented: f32,
}

pub const FAIRCHILD_TIME_CONSTANTS: [TimeConstant; 6] = [
    TimeConstant {
        attack: 0.0002,
        capacitor: 2.0,
        resistor: 51.9,
        branches: [[0.0, 0.0], [0.0, 0.0]],
        documented: 0.3,
    },
    TimeConstant {
        attack: 0.0002,
        capacitor: 2.0,
        resistor: 149.9,
        branches: [[0.0, 0.0], [0.0, 0.0]],
        documented: 0.8,
    },
    TimeConstant {
        attack: 0.0004,
        capacitor: 4.0,
        resistor: 220.0,
        branches: [[0.0, 0.0], [0.0, 0.0]],
        documented: 2.0,
    },
    TimeConstant {
        attack: 0.0004,
        capacitor: 8.0,
        resistor: 220.0,
        branches: [[0.0, 0.0], [0.0, 0.0]],
        documented: 5.0,
    },
    TimeConstant {
        attack: 0.0004,
        capacitor: 4.0,
        resistor: 220.0,
        branches: [[100.0, 8.0], [0.0, 0.0]],
        documented: 2.0,
    },
    TimeConstant {
        attack: 0.0002,
        capacitor: 2.0,
        resistor: 220.0,
        branches: [[100.0, 8.0], [100.0, 20.0]],
        documented: 0.3,
    },
];

/// The position of the Fairchild's time constant switch a release position means (0.5 is position 3, the middle)
pub fn fairchild_position(release: f32) -> usize {
    ((release.clamp(0.0, 1.0) * 6.0 - 0.5).floor() as i32).clamp(0, 5) as usize
}

/// The compressors the strip models. Their keys are saved with projects and never change (U30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompressorModel {
    Ssl,
    Api2500,
    Dbx160,
    Urei1176,
    La2a,
    Fairchild670,
    GatesStaLevel,
    Altec436,
    RcaBa6a,
}

impl CompressorModel {
    /// The order the models are offered in: the VCAs, the FET, the optical, the variable-mu
    pub const DISPLAY_ORDER: [CompressorModel; 9] = [
        CompressorModel::Ssl,
        CompressorModel::Api2500,
        CompressorModel::Dbx160,
        CompressorModel::Urei1176,
        CompressorModel::La2a,
        CompressorModel::Fairchild670,
        CompressorModel::GatesStaLevel,
        CompressorModel::Altec436,
        CompressorModel::RcaBa6a,
    ];

    pub fn key(self) -> &'static str {
        match self {
            CompressorModel::Ssl => "ssl",
            CompressorModel::Api2500 => "api2500",
            CompressorModel::Dbx160 => "dbx160",
            CompressorModel::Urei1176 => "urei1176",
            CompressorModel::La2a => "la2a",
            CompressorModel::Fairchild670 => "fairchild670",
            CompressorModel::GatesStaLevel => "gates-sta-level",
            CompressorModel::Altec436 => "altec436",
            CompressorModel::RcaBa6a => "rca-ba6a",
        }
    }

    pub fn from_key(key: &str) -> Option<CompressorModel> {
        Self::DISPLAY_ORDER.into_iter().find(|m| m.key() == key)
    }

    pub fn family(self) -> Family {
        match self {
            CompressorModel::Urei1176 => Family::Fet,
            CompressorModel::La2a => Family::Optical,
            CompressorModel::Fairchild670
            | CompressorModel::GatesStaLevel
            | CompressorModel::Altec436
            | CompressorModel::RcaBa6a => Family::VariMu,
            _ => Family::Vca,
        }
    }

    /// Whether the model has a release to set (the dbx 160's comes out of its detector)
    pub fn has_release(self) -> bool {
        self != CompressorModel::Dbx160
    }

    /// The threshold an amount (COMP, 0 to 1) means, in dBu: from the unit's top (not compressing) down
    pub fn threshold_for(self, amount: f32) -> f32 {
        let amount = amount.clamp(0.0, 1.0);
        match self {
            CompressorModel::Ssl => 4.0 - 30.0 * amount,      // +4 to -26
            CompressorModel::Api2500 => 10.0 - 30.0 * amount, // +10 to -20
            CompressorModel::Dbx160 => 12.0 - 50.0 * amount,  // +12 to -38
            _ => 10.0 - 40.0 * amount,                        // the input driving the unit: ours
        }
    }

    /// Seconds of release a position means (or a factor on the unit's own, see the interface's texts)
    pub fn release_for(self, release: f32) -> f32 {
        match self {
            CompressorModel::Ssl => log_scale(release, 0.1, 0.3, 4.0),
            CompressorModel::Api2500 => log_scale(release, 0.05, 0.3, 3.0),
            CompressorModel::Urei1176 => log_scale(release, 0.05, 0.25, 1.1),
            CompressorModel::La2a => log_scale(release, 0.3, 1.0, 3.0), // a factor on the slow stage
            CompressorModel::Fairchild670 => FAIRCHILD_TIME_CONSTANTS[fairchild_position(release)].documented,
            CompressorModel::GatesStaLevel => log_scale(release, 0.32, 1.0, 1.6), // a factor on the factory recovery
            CompressorModel::Altec436 => log_scale(release, 0.3, 0.7, 1.3),
            CompressorModel::RcaBa6a => log_scale(release, 0.15, 0.33, 0.7),
            CompressorModel::Dbx160 => 0.0,
        }
    }
}

/// The families of compressors
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Vca,
    Fet,
    Optical,
    VariMu,
}

/// The attack, within what the model allows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompressorSpeed {
    Fast,
    Norm,
    Slow,
}

impl CompressorSpeed {
    fn index(self) -> usize {
        self as usize
    }
}

/// How each model works. The engines and their parameters, from the research (legado/docs/pesquisa-compressores.md)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    FeedForward,
    LogFeedback,
    VariMu,
    Optical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Coloration {
    /// Polynomial coefficients at no gain reduction, relative to the reference level
    second: f32,
    third: f32,
    /// Added per 10 dB of gain reduction
    second_per10: f32,
    third_per10: f32,
    /// The reference level is the threshold (the tubes' operating level), else 0 VU
    at_threshold: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Path {
    /// Hz, second order, 0 for none
    high_pass: f32,
    /// Hz, first order, 0 for none
    low_pass: f32,
    /// Hz, the low-pass at 20 dB of gain reduction (the RCA's response moves), 0 for fixed
    low_pass_at20: f32,
    transformer: Option<TransformerCore>,
}

fn engine_of(model: CompressorModel) -> Engine {
    match model {
        CompressorModel::Ssl | CompressorModel::Dbx160 => Engine::FeedForward,
        CompressorModel::Api2500 | CompressorModel::Urei1176 => Engine::LogFeedback,
        CompressorModel::La2a => Engine::Optical,
        _ => Engine::VariMu,
    }
}

fn coloration_of(model: CompressorModel) -> Coloration {
    let c = |second, third, second_per10, third_per10| Coloration {
        second,
        third,
        second_per10,
        third_per10,
        at_threshold: false,
    };
    match model {
        // The VCAs are clean (SSL: THD+N < 0.05% at +20 dBu); the API a little more, its discrete stages
        CompressorModel::Ssl => c(0.0, 0.0, 0.0, 0.0),
        CompressorModel::Api2500 => c(0.0004, 0.0002, 0.0, 0.0),
        // The dbx 160's comes from its RMS detector's ripple modulating the VCA, by itself
        CompressorModel::Dbx160 => c(0.0, 0.0, 0.0, 0.0),
        // The 1176's FET: 0.07% at 1 kHz with ~8 dB of reduction (Moore); 2nd and 3rd, the 2nd growing with it
        CompressorModel::Urei1176 => c(0.00005, 0.00005, 0.0003, 0.00028),
        // The LA-2A's 12AX7 output stage: < 0.35% at +10 dBm, single ended, so the 2nd
        CompressorModel::La2a => c(0.0025, 0.0003, 0.0004, 0.0002),
        // Push-pull variable-mu: the 3rd, growing with the reduction. The output sits at the nominal level (the
        // make-up), which the specifications' output levels are compared with
        CompressorModel::Fairchild670 => c(0.0001, 0.0004, 0.0001, 0.0012), // < 1% at 10 dB, +12 dBm
        CompressorModel::GatesStaLevel => c(0.0003, 0.0003, 0.0002, 0.0005), // <= 1% to 30 dB, +10 dBm
        CompressorModel::Altec436 => c(0.001, 0.0005, 0.002, 0.0006),       // < 1.5% at 25 dB
        CompressorModel::RcaBa6a => c(0.0003, 0.0027, 0.0002, 0.0014),      // RCA: < 0.6%, < 1% at 12 dB
    }
}

fn path_of(model: CompressorModel) -> Path {
    // The units' responses, within their specifications: second order high-pass and first order low-pass placed for
    // the tolerance at the band's edges (ours)
    let p = |high_pass, low_pass, low_pass_at20, low_corner, saturation_flux, high_corner| Path {
        high_pass,
        low_pass,
        low_pass_at20,
        transformer: Some(TransformerCore { low_corner, saturation_flux, high_corner }),
    };
    match model {
        CompressorModel::Urei1176 => p(14.3, 39000.0, 0.0, 2.0, 0.015, 60000.0), // 20 Hz-20 kHz +-1 dB
        CompressorModel::La2a => p(0.0, 0.0, 0.0, 3.0, 0.012, 80000.0),          // +-0.1 dB
        CompressorModel::Fairchild670 => p(28.5, 29500.0, 0.0, 5.0, 0.006, 40000.0), // 40 Hz-15 kHz +-1 dB
        CompressorModel::GatesStaLevel => p(21.4, 29500.0, 0.0, 5.0, 0.007, 40000.0), // 30 Hz-15 kHz +-1 dB
        CompressorModel::Altec436 => p(24.0, 23400.0, 0.0, 6.0, 0.006, 35000.0), // 30 Hz-15 kHz +-1.5 dB
        CompressorModel::RcaBa6a => p(21.4, 29500.0, 19600.0, 6.0, 0.005, 30000.0), // -2 dB at 20 dB GR
        _ => Path::default(),
    }
}

/// A variable-mu tube's law: gain reduction in dB for a control voltage, GR = a ln(1 + v / c0) + b v
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct TubeLaw {
    a: f32,
    c0: f32,
    b: f32,
}

impl TubeLaw {
    fn at(&self, v: f32) -> f32 {
        if v > 0.0 { self.a * log1pf(v / self.c0) + self.b * v } else { 0.0 }
    }
}

/// A variable-mu unit's RC network
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Network {
    /// s: charging through the rectifier, discharging
    attack: f32,
    release: f32,
    /// Series branches: R C of the branch, their capacitor over the main one
    branch_tau: [f32; 2],
    branch_ratio: [f32; 2],
    branches: usize,
}

/// The compressor's controls, positions which each model reads as its unit does
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressorSettings {
    pub model: CompressorModel,
    /// COMP, 0 to 1, how much it compresses: a threshold, or the input driving a fixed threshold
    pub amount: f32,
    /// RATIO, 0 (out) to 1: a ratio, its steps, COMP/LIMIT, or the shape of a variable-mu curve
    pub ratio: f32,
    /// FAST, NORM, SLOW: the attack within what the unit allows
    pub speed: CompressorSpeed,
    /// 0 to 1: the release, or the time constant switch; 0.5 is the unit's factory setting
    pub release: f32,
}

impl Default for CompressorSettings {
    fn default() -> Self {
        Self { model: CompressorModel::Ssl, amount: 0.0, ratio: 0.0, speed: CompressorSpeed::Norm, release: 0.5 }
    }
}

/// The channel compressor, of a choice of models of real units, stereo-linked, with automatic make-up gain (a
/// signal at the nominal level comes out where it went in). The engines, from the research:
/// - feed-forward (SSL, dbx 160): an RMS detector, a gain computer with a knee, ballistics;
/// - feedback in the log domain (API 2500 in OLD mode, 1176): the detector reads the output; the loop is solved at
///   each sample, so that ratio and knee come out of it as on the unit;
/// - variable-mu (Fairchild 670, Gates Sta-Level, Altec 436, RCA BA-6A): a rectifier charging the unit's RC network,
///   whose control voltage sets the gain through the tube's law; the loop solved at each sample;
/// - optical (LA-2A): a cell with fast, slow and memory states, in the loop.
///
/// Then the gain element's distortion (oversampled), and the unit's frequency response and transformer.
#[derive(Clone, Debug)]
pub struct ChannelCompressor {
    settings: CompressorSettings,
    sample_rate: f32,
    /// None until the first begin()
    model: Option<CompressorModel>,

    // What begin() works out
    engine: Engine,
    /// dBFS: RMS for the RMS detectors, peak for the others
    threshold: f32,
    /// 1 - 1 / ratio (feed-forward) or the loop gain k = ratio - 1 (feedback)
    slope: f32,
    knee_width: f32,
    /// dB
    makeup: f32,
    rms_coefficient: f32,
    /// dB per frame (slope attacks)
    attack_step: f32,
    attack_coefficient: f32,
    release_coefficient: f32,
    fast_release_coefficient: f32,
    charge_coefficient: f32,
    all_buttons: bool,
    thrust: bool,
    /// The SSL's fixed fast part
    dual_release: bool,

    // Variable-mu: the rectifier, the RC network and the tube's law
    law: TubeLaw,
    /// V per unit of the output's amplitude
    sidechain: f32,
    /// V the rectifier needs before it conducts
    bias: f32,
    period: f32,
    /// Two for the RCA's dual mode
    networks: [Network; 2],
    network_count: usize,

    // Optical: the cell
    cell_gain: f32,
    cell_offset: f32,
    cell_exponent: f32,
    /// The panel's drive, averaged (the panel and the cell integrate)
    cell_envelope: f32,
    envelope_coefficient: f32,
    cell_attack: f32,
    cell_fall: f32,
    cell_charge: f32,
    cell_slow_fall: f32,
    cell_memory_rise: f32,
    cell_memory_fall: f32,
    limit_feed_forward: f32,
    /// The THRUST filter's gain at 1 kHz, divided out
    thrust_normal: f32,

    coloration: Coloration,
    path: Path,
    /// Amplitude of the coloration's reference level
    reference: f32,
    colored: bool,

    // Running state
    mean_square: f32,
    /// dB
    reduction: f32,
    /// dB, the SSL's slow release part
    slow_reduction: f32,
    /// Per network: main node, branches
    voltages: [[f32; 3]; 2],
    cell_fast: f32,
    cell_slow: f32,
    cell_memory: f32,
    dc_second: [f32; 2],
    upsamplers: [Upsampler<2>; 2],
    downsamplers: [Downsampler<2>; 2],
    thrust_filter: RampedBiquad,
    thrust_filter2: RampedBiquad,
    high_pass: RampedBiquad,
    low_pass: RampedBiquad,
    /// The index in `transformers` of this model's, if it has one
    transformer: Option<usize>,
    /// Every model's transformer, made when preparing, so that changing the model allocates nothing
    transformers: [Option<OutputTransformer>; 9],
}

/// Solves h(x) = 0 for a decreasing h on [low, high] (regula falsi, Illinois)
fn solve(h: impl Fn(f32) -> f32, mut low: f32, mut high: f32) -> f32 {
    let mut h_low = h(low);
    if h_low <= 0.0 {
        return low;
    }
    let mut h_high = h(high);
    if h_high >= 0.0 {
        return high;
    }
    let mut side = 0;
    let mut x = low;
    for _ in 0..30 {
        x = (low * h_high - high * h_low) / (h_high - h_low);
        let hx = h(x);
        if hx.abs() < 1e-5 || high - low < 1e-6 {
            break;
        }
        if hx > 0.0 {
            low = x;
            h_low = hx;
            if side == 1 {
                h_high /= 2.0;
            }
            side = 1;
        } else {
            high = x;
            h_high = hx;
            if side == -1 {
                h_low /= 2.0;
            }
            side = -1;
        }
    }
    x
}

/// Bisects the reduction g in [0, 120] where @p reaches(g) stops being above g
fn settle(reaches: impl Fn(f32) -> f32) -> f32 {
    let (mut low, mut high) = (0.0f32, 120.0f32);
    for _ in 0..60 {
        let g = (low + high) / 2.0;
        if reaches(g) > g { low = g } else { high = g }
    }
    (low + high) / 2.0
}

impl ChannelCompressor {
    /// Prepared for @p sample_rate: everything any model needs is made here
    pub fn new(sample_rate: f32) -> Self {
        let transformers = CompressorModel::DISPLAY_ORDER
            .map(|m| path_of(m).transformer.map(|core| OutputTransformer::new(core, f64::from(sample_rate))));
        Self {
            settings: CompressorSettings::default(),
            sample_rate,
            model: None,
            engine: Engine::FeedForward,
            threshold: 0.0,
            slope: 0.0,
            knee_width: 0.0,
            makeup: 0.0,
            rms_coefficient: 0.0,
            attack_step: 0.0,
            attack_coefficient: 0.0,
            release_coefficient: 0.0,
            fast_release_coefficient: 0.0,
            charge_coefficient: 0.0,
            all_buttons: false,
            thrust: false,
            dual_release: false,
            law: TubeLaw::default(),
            sidechain: 1.0,
            bias: 0.0,
            period: 0.0,
            networks: [Network::default(); 2],
            network_count: 1,
            cell_gain: 1.0,
            cell_offset: 0.0,
            cell_exponent: 1.0,
            cell_envelope: 0.0,
            envelope_coefficient: 0.0,
            cell_attack: 0.0,
            cell_fall: 0.0,
            cell_charge: 0.0,
            cell_slow_fall: 0.0,
            cell_memory_rise: 0.0,
            cell_memory_fall: 0.0,
            limit_feed_forward: 0.0,
            thrust_normal: 1.0,
            coloration: Coloration::default(),
            path: Path::default(),
            reference: 1.0,
            colored: false,
            mean_square: 0.0,
            reduction: 0.0,
            slow_reduction: 0.0,
            voltages: [[0.0; 3]; 2],
            cell_fast: 0.0,
            cell_slow: 0.0,
            cell_memory: 0.0,
            dc_second: [0.0; 2],
            upsamplers: [Upsampler::new(sample_rate), Upsampler::new(sample_rate)],
            downsamplers: [Downsampler::new(sample_rate), Downsampler::new(sample_rate)],
            thrust_filter: RampedBiquad::default(),
            thrust_filter2: RampedBiquad::default(),
            high_pass: RampedBiquad::default(),
            low_pass: RampedBiquad::default(),
            transformer: None,
            transformers,
        }
    }

    pub fn is_active(&self) -> bool {
        self.settings.ratio > 0.0
    }

    /// The gain reduction now, in dB
    pub fn reduction(&self) -> f32 {
        self.reduction
    }

    /// The make-up gain, in dB
    pub fn makeup(&self) -> f32 {
        self.makeup
    }

    /// Sets the controls for what follows; recomputes only what changed
    pub fn begin(&mut self, settings: CompressorSettings) {
        let changed = settings != self.settings;
        self.settings = settings;
        if !changed && self.model == Some(settings.model) {
            return;
        }
        let rate = self.sample_rate;
        let model = settings.model;
        let speed = settings.speed.index();

        if self.model != Some(model) {
            self.model = Some(model);
            self.engine = engine_of(model);
            self.coloration = coloration_of(model);
            self.path = path_of(model);
            let c = &self.coloration;
            self.colored = c.second != 0.0 || c.third != 0.0 || c.second_per10 != 0.0 || c.third_per10 != 0.0;
            let rate64 = f64::from(rate);
            if self.path.high_pass > 0.0 {
                self.high_pass.set_target(Biquad::high_pass(
                    f64::from(self.path.high_pass),
                    core::f64::consts::FRAC_1_SQRT_2,
                    rate64,
                ));
            }
            if self.path.low_pass > 0.0 {
                self.low_pass.set_target(Biquad::low_pass1(f64::from(self.path.low_pass), rate64));
            }
            self.high_pass.finish();
            self.low_pass.finish();
            // This model's transformer, made when preparing (reset below)
            self.transformer = CompressorModel::DISPLAY_ORDER.iter().position(|&m| m == model);
            // API's THRUST (LOUD): +10 dB per decade on the detector, as two first order shelves (ours), 0 dB at
            // 1 kHz
            self.thrust = model == CompressorModel::Api2500;
            self.thrust_filter.set_target(Biquad::high_shelf1(100.0, f64::from(dbfs_to_amp(15.0)), rate64));
            self.thrust_filter2.set_target(Biquad::high_shelf1(2000.0, f64::from(dbfs_to_amp(15.0)), rate64));
            self.thrust_filter.finish();
            self.thrust_filter2.finish();
            self.thrust_normal = (self.thrust_filter.target().magnitude(1000.0, rate64)
                * self.thrust_filter2.target().magnitude(1000.0, rate64)) as f32;
            self.reset();
        }

        self.threshold = model.threshold_for(settings.amount) + DBU_TO_DBFS;
        self.all_buttons = false;
        self.dual_release = false;
        let ratio = settings.ratio.clamp(0.0, 1.0);

        match self.engine {
            Engine::FeedForward => {
                if model == CompressorModel::Ssl {
                    let r = powf(20.0, ratio);
                    self.slope = if r >= 19.99 { 1.0 } else { 1.0 - 1.0 / r };
                    self.knee_width = 10.0;
                    self.rms_coefficient = one_pole(0.005, rate);
                    // 20 dB in 3 / 30 / 60 ms; the release's fixed fast part (40 ms, ours) and the knob's slow one
                    self.attack_step = 20.0 / ([0.003, 0.03, 0.06][speed] * rate);
                    self.release_coefficient = one_pole(model.release_for(settings.release), rate);
                    self.fast_release_coefficient = one_pole(0.04, rate);
                    self.charge_coefficient = one_pole(0.15, rate);
                    self.dual_release = true;
                } else {
                    // dbx 160: one true RMS detector (36 ms gives the manual's attacks and 120 dB/s), a hard knee,
                    // infinity at about 120:1
                    let r = powf(20.0, ratio);
                    self.slope = if r >= 19.99 { 1.0 - 1.0 / 120.0 } else { 1.0 - 1.0 / r };
                    self.knee_width = 0.0;
                    self.rms_coefficient = one_pole([0.018, 0.036, 0.072][speed], rate);
                }
                self.makeup = knee(NOMINAL_RMS - self.threshold, self.knee_width) * self.slope;
            }
            Engine::LogFeedback => {
                if model == CompressorModel::Api2500 {
                    let r = API_RATIOS[step(ratio, API_RATIOS.len())];
                    self.knee_width = 6.0; // MED (ours)
                    self.rms_coefficient = one_pole(0.01, rate);
                    self.attack_coefficient = one_pole([0.0003, 0.003, 0.03][speed], rate);
                    self.slope = if r.is_infinite() { 50.0 } else { r - 1.0 }; // feedback cannot reach infinity
                } else {
                    let index = step(ratio, UREI_RATIOS.len());
                    let r = UREI_RATIOS[index];
                    self.all_buttons = r == 0.0;
                    // The fixed threshold of the ratio, moved by the input (ours: COMP is the input)
                    self.threshold =
                        model.threshold_for(settings.amount) + (UREI_THRESHOLDS[index] + 30.0) + DBU_TO_DBFS;
                    self.knee_width = if self.all_buttons { 1.0 } else { [6.0, 4.0, 3.0, 2.0][index] };
                    self.slope = if self.all_buttons { 30.0 } else { r - 1.0 };
                    // The attack resistor: 20 / 200 / 800 us; all buttons in, the attack lags on the transients
                    let attack: f32 = [0.00002, 0.0002, 0.0008][speed];
                    self.attack_coefficient = one_pole(if self.all_buttons { attack * 3.0 } else { attack }, rate);
                }
                self.release_coefficient = one_pole(model.release_for(settings.release), rate);
                // Make-up: the reduction a signal at the nominal level settles at, solving g = k knee(L - g - T)
                let level = if model == CompressorModel::Api2500 { NOMINAL_RMS } else { level_of(nominal_peak()) };
                let (slope, threshold, width) = (self.slope, self.threshold, self.knee_width);
                self.makeup = settle(|g| slope * knee(level - g - threshold, width));
            }
            Engine::VariMu => {
                self.set_up_vari_mu(model, &settings, rate);
                // Make-up: the reduction of a sine at the nominal level, its peak charging the network
                let (sidechain, bias, law) = (self.sidechain, self.bias, self.law);
                let peak = nominal_peak();
                self.makeup = settle(|g| law.at((sidechain * peak * dbfs_to_amp(-g) - bias).max(0.0)));
            }
            Engine::Optical => {
                // The cell: light from the panel, which grows faster than the drive (EL panels), and a photoresistor
                // dividing the signal, GR = 20 log(1 + L^gamma); COMP ~4:1, LIMIT ~infinity (UA), as the loop's
                // slope
                let limit = ratio >= 0.5;
                self.cell_exponent = if limit { 14.0 } else { 4.3 };
                self.cell_gain = 1.0 / dbfs_to_amp(self.threshold);
                self.cell_offset = 0.5;
                // (A partly feed-forward LIMIT is forum lore, and it runs away: LIMIT comes out of the loop's slope)
                self.limit_feed_forward = 0.0;
                self.envelope_coefficient = one_pole(0.002, rate);
                self.cell_attack = one_pole([0.005, 0.01, 0.02][speed], rate);
                self.cell_fall = one_pole(0.05, rate); // half the reduction gone in about 60 ms
                self.cell_charge = one_pole(2.0, rate); // the slow state builds up while it is lit
                self.cell_slow_fall = one_pole(1.5 * model.release_for(settings.release), rate);
                self.cell_memory_rise = one_pole(5.0, rate);
                self.cell_memory_fall = one_pole(30.0, rate);
                let (gain, feed, offset, exponent) =
                    (self.cell_gain, self.limit_feed_forward, self.cell_offset, self.cell_exponent);
                let peak = nominal_peak();
                self.makeup = settle(|g| {
                    // The average of a rectified sine is 2 / pi of its peak
                    let drive = (gain * peak * 2.0 / f32c::PI * (dbfs_to_amp(-g) + feed) - offset).max(0.0);
                    let light = powf(drive, exponent * 0.7);
                    20.0 * log10f(1.0 + light)
                });
            }
        }
        self.reference = if self.coloration.at_threshold { dbfs_to_amp(self.threshold) } else { nominal_peak() };
    }

    /// The variable-mu units: their tube laws and sidechains, calibrated so that the ratio the loop gives is the
    /// unit's (asymptotically 1 + a ln10 / 20 for a ln law; the linear term makes the Fairchild's grow)
    fn set_up_vari_mu(&mut self, model: CompressorModel, settings: &CompressorSettings, rate: f32) {
        let ratio = settings.ratio.clamp(0.0, 1.0);
        let speed = settings.speed.index();
        let mut threshold = self.threshold; // dBFS of the peak where the rectifier starts to conduct
        let mut bias = 0.0; // V; by default where the law's knee falls
        self.network_count = 1;
        let net = &mut self.networks[0];
        net.branches = 0;
        match model {
            CompressorModel::Fairchild670 => {
                // The DC threshold shapes the curve. The 6386 is a remote cutoff tube: its gain falls exponentially
                // with the bias, so the reduction in dB grows linearly with the control voltage. In the loop the
                // slope is then ln10/20 (GR + beta), beta being the rectifier's bias: the ratio grows with the
                // reduction, and the DC threshold sets where it starts (from about 1.6:1 to about 7:1, growing to
                // 20:1 and more when pushed)
                self.law = TubeLaw { a: 0.0, c0: 1.0, b: 1.0 };
                bias = 5.0 + 50.0 * ratio;
                let tc = &FAIRCHILD_TIME_CONSTANTS[fairchild_position(settings.release)];
                let attack = match speed {
                    0 => 0.0002,
                    2 => 0.0008,
                    _ => tc.attack,
                };
                let c = tc.capacitor;
                net.attack = attack;
                // The documented release is ~2.3 to 2.9 times R C (der): the law's slope near the operating point
                net.release = tc.resistor * c * 1e-3;
                for branch in tc.branches {
                    if branch[0] <= 0.0 {
                        continue;
                    }
                    net.branch_tau[net.branches] = branch[0] * branch[1] * 1e-3;
                    net.branch_ratio[net.branches] = branch[1] / c;
                    net.branches += 1;
                }
            }
            CompressorModel::GatesStaLevel => {
                // The manual's table of bias against reduction: GR = 16.6 ln(1 + V / 8.1); the ratio resistor scales
                // it (1.6:1 to 6:1, 3.3:1 from the factory)
                let target = 1.6 + 4.4 * ratio;
                self.law = TubeLaw { a: (target - 1.0) / LN10_OVER_20, c0: 8.1, b: 0.0 };
                let factor = model.release_for(settings.release);
                if speed == 0 {
                    // Double: 25 ms, a fast recovery (2.35 s from the factory) and a slow one after sustained peaks
                    net.attack = 0.025;
                    net.release = 2.35 / 2.5 * factor;
                    net.branch_tau[0] = 3.1 * factor;
                    net.branch_ratio[0] = 2.0;
                    net.branches = 1;
                } else {
                    // Single: 75 ms (150 ms slow, ours), 7.05 s to 63%
                    net.attack = if speed == 2 { 0.15 } else { 0.075 };
                    net.release = 7.05 / 2.5 * factor;
                }
            }
            CompressorModel::Altec436 => {
                // The threshold, 0 to +16 dBm, raises the ratio from 2:1 to 4:1
                let target = 2.0 + 2.0 * ratio;
                self.law = TubeLaw { a: (target - 1.0) / LN10_OVER_20, c0: 6.0, b: 0.0 };
                threshold += 16.0 * ratio;
                net.attack = match speed {
                    0 => 0.025,
                    2 => 0.1,
                    _ => 0.05,
                };
                net.release = model.release_for(settings.release) / 2.5;
            }
            _ => {
                // RCA BA-6A: about 10:1 with a hard knee; the bias shapes it from 4:1
                let target = 4.0 + 6.0 * ratio;
                self.law = TubeLaw { a: (target - 1.0) / LN10_OVER_20, c0: 1.5, b: 0.0 };
                let fast_release = model.release_for(settings.release) / 2.5;
                // Single: the fast path; dual: both; slow: the slow path only
                if speed == 2 {
                    net.attack = 0.9;
                    net.release = 2.0 / 2.5;
                } else {
                    net.attack = 0.0006;
                    net.release = fast_release;
                }
                if speed == 1 {
                    self.networks[1] = Network {
                        attack: 0.9,
                        release: 2.0 / 2.5,
                        branch_tau: [0.0; 2],
                        branch_ratio: [0.0; 2],
                        branches: 0,
                    };
                    self.network_count = 2;
                }
            }
        }
        // The rectifier's bias: conduction from the threshold up; the sidechain scaled so that the law's knee falls
        // there
        self.bias = if bias > 0.0 { bias } else { self.law.c0 * 2.0 };
        self.sidechain = self.bias / dbfs_to_amp(threshold);
        self.period = 1.0 / rate;
    }

    /// Compresses a frame in place
    #[inline]
    pub fn process(&mut self, left: &mut f32, right: &mut f32) {
        let gain = match self.engine {
            Engine::FeedForward => self.feed_forward(*left, *right),
            Engine::LogFeedback => self.log_feedback(*left, *right),
            Engine::VariMu => self.vari_mu(*left, *right),
            Engine::Optical => self.optical(*left, *right),
        };
        *left *= gain;
        *right *= gain;

        // The gain element's distortion, oversampled: a 2nd (without its offset) and a 3rd, growing with the
        // reduction
        if self.colored {
            let c = self.coloration;
            let per10 = self.reduction.max(0.0) / 10.0;
            let growth = if self.model == Some(CompressorModel::Fairchild670) { per10 * per10.sqrt() } else { per10 };
            let mut second = 2.0 * (c.second + c.second_per10 * growth);
            let mut third = 4.0 * (c.third + c.third_per10 * growth);
            if self.all_buttons {
                second *= 3.0;
                third *= 3.0;
            }
            let reference = self.reference;
            for (ch, sample) in [&mut *left, &mut *right].into_iter().enumerate() {
                let mut oversampled = [0.0f32; 8];
                self.upsamplers[ch].process(&mut oversampled, *sample);
                for x in &mut oversampled[..4] {
                    let u = 4.0 * tanhf(*x / reference / 4.0);
                    let squared = u * u;
                    self.dc_second[ch] += (squared - self.dc_second[ch]) * 0.0002;
                    *x += reference * (second * (squared - self.dc_second[ch]) + third * squared * u);
                }
                *sample = self.downsamplers[ch].process(&mut oversampled);
            }
        }

        self.process_path(left, right);
    }

    /// The unit's response and transformer, after the gain and the distortion
    #[inline]
    fn process_path(&mut self, left: &mut f32, right: &mut f32) {
        if self.path.low_pass_at20 > 0.0 {
            let amount = (self.reduction / 20.0).clamp(0.0, 1.0);
            let corner = self.path.low_pass + (self.path.low_pass_at20 - self.path.low_pass) * amount;
            self.low_pass.set_target(Biquad::low_pass1(f64::from(corner), f64::from(self.sample_rate)));
            self.low_pass.finish();
        }
        if self.path.high_pass > 0.0 {
            *left = self.high_pass.process(f64::from(*left), 0, 1.0) as f32;
            *right = self.high_pass.process(f64::from(*right), 1, 1.0) as f32;
        }
        if self.path.low_pass > 0.0 {
            *left = self.low_pass.process(f64::from(*left), 0, 1.0) as f32;
            *right = self.low_pass.process(f64::from(*right), 1, 1.0) as f32;
        }
        if let Some(transformer) = self.transformer.and_then(|i| self.transformers[i].as_mut()) {
            *left = transformer.process(*left, 0);
            *right = transformer.process(*right, 1);
        }
    }

    fn feed_forward(&mut self, left: f32, right: f32) -> f32 {
        self.mean_square += ((left * left).max(right * right) - self.mean_square) * self.rms_coefficient;
        let target = knee(level_of(self.mean_square.sqrt()) - self.threshold, self.knee_width) * self.slope;
        if self.dual_release {
            // SSL: a slope attack; on release a fast part, and the slow part, which builds up while it compresses
            if target > self.reduction {
                self.reduction = target.min(self.reduction + self.attack_step);
            } else {
                self.reduction += (target - self.reduction) * self.fast_release_coefficient;
            }
            if self.reduction > self.slow_reduction {
                self.slow_reduction += (self.reduction - self.slow_reduction) * self.charge_coefficient;
            } else {
                self.slow_reduction += (target - self.slow_reduction) * self.release_coefficient;
            }
            self.slow_reduction = self.slow_reduction.max(0.0);
            dbfs_to_amp(self.makeup - self.reduction.max(self.slow_reduction))
        } else {
            // dbx 160: the detector is the ballistics
            self.reduction = target;
            dbfs_to_amp(self.makeup - self.reduction)
        }
    }

    fn log_feedback(&mut self, left: f32, right: f32) -> f32 {
        // The detector reads the output: solve for the reduction that the reduction leads to
        let (mut key_left, mut key_right) = (left, right);
        if self.thrust {
            key_left = self.thrust_filter2.process(self.thrust_filter.process(f64::from(left), 0, 1.0), 0, 1.0) as f32
                / self.thrust_normal;
            key_right = self.thrust_filter2.process(self.thrust_filter.process(f64::from(right), 1, 1.0), 1, 1.0)
                as f32
                / self.thrust_normal;
        }
        let key_peak = key_left.abs().max(key_right.abs());
        let key_square = (key_left * key_left).max(key_right * key_right);
        let rms = self.rms_coefficient > 0.0 && self.model == Some(CompressorModel::Api2500);
        let (mean_square, rms_coefficient, slope, threshold, width, reduction, attack, release) = (
            self.mean_square,
            self.rms_coefficient,
            self.slope,
            self.threshold,
            self.knee_width,
            self.reduction,
            self.attack_coefficient,
            self.release_coefficient,
        );
        let next = |g: f32| -> (f32, f32) {
            let through = dbfs_to_amp(-g);
            let (level, ms) = if rms {
                let ms = mean_square + (key_square * through * through - mean_square) * rms_coefficient;
                (level_of(ms.sqrt()), ms)
            } else {
                (level_of(key_peak * through), mean_square)
            };
            let target = slope * knee(level - threshold, width);
            let coefficient = if target > reduction { attack } else { release };
            (reduction + (target - reduction) * coefficient, ms)
        };
        let start = next(reduction).0;
        let g = solve(|x| next(x).0 - x, 0.0, start.max(reduction).max(0.0) + 60.0);
        let (settled, ms) = next(g);
        self.reduction = settled;
        self.mean_square = ms;
        dbfs_to_amp(self.makeup - self.reduction)
    }

    fn vari_mu(&mut self, left: f32, right: f32) -> f32 {
        let peak = left.abs().max(right.abs());
        // Each network: charged through the rectifier from the output, discharged through its resistor, its
        // branches drawing from it; solve the main node for the gain it leads to
        let (networks, count, voltages, sidechain, bias, law, period) =
            (self.networks, self.network_count, self.voltages, self.sidechain, self.bias, self.law, self.period);
        let charge = |n: usize, v: f32, node: f32| -> f32 {
            let net = &networks[n];
            let state = &voltages[n];
            let rectified = (sidechain * peak * dbfs_to_amp(-law.at(v)) - bias).max(0.0);
            let mut dv = -state[0] / net.release;
            for b in 0..net.branches {
                dv -= (state[0] - state[b + 1]) * net.branch_ratio[b] / net.branch_tau[b];
            }
            dv += (rectified - node).max(0.0) / net.attack;
            state[0] + dv * period
        };
        let control = |nodes: [f32; 2]| if count == 1 { nodes[0] } else { nodes[0].max(nodes[1]) };
        let evaluate = |v: f32| -> ([f32; 2], f32) {
            let mut nodes = [0.0f32; 2];
            for (n, node) in nodes.iter_mut().enumerate().take(count) {
                *node = charge(n, v, voltages[n][0]);
            }
            (nodes, control(nodes))
        };
        let high = evaluate(0.0).1.max(0.0) + 1.0;
        let v = solve(|x| evaluate(x).1 - x, 0.0, high);
        let (nodes, _) = evaluate(v);
        for n in 0..count {
            let net = &self.networks[n];
            let state = &mut self.voltages[n];
            for b in 0..net.branches {
                state[b + 1] += (state[0] - state[b + 1]) / net.branch_tau[b] * self.period;
            }
            state[0] = nodes[n].max(0.0);
        }
        self.reduction = self.law.at(control([self.voltages[0][0], self.voltages[1][0]]));
        dbfs_to_amp(self.makeup - self.reduction)
    }

    fn optical(&mut self, left: f32, right: f32) -> f32 {
        let peak = left.abs().max(right.abs());
        // The cell in the loop: its light from the output (and, limiting, a part of the input)
        let s = &*self;
        let cell_step = |g: f32| -> (f32, f32, f32) {
            let through = dbfs_to_amp(-g);
            let envelope =
                s.cell_envelope + (peak * (through + s.limit_feed_forward) - s.cell_envelope) * s.envelope_coefficient;
            let drive = (s.cell_gain * envelope - s.cell_offset).max(0.0);
            let target = powf(drive, s.cell_exponent * 0.7);
            let fast =
                s.cell_fast + (target - s.cell_fast) * if target > s.cell_fast { s.cell_attack } else { s.cell_fall };
            let memory = 1.0 + 2.0 * s.cell_memory;
            let slow = s.cell_slow
                + if fast > s.cell_slow {
                    (fast - s.cell_slow) * s.cell_charge
                } else {
                    (fast - s.cell_slow) * (s.cell_slow_fall / memory)
                };
            (20.0 * log10f(1.0 + fast.max(slow)), fast, slow)
        };
        let high = cell_step(0.0).0 + 1.0;
        let g = solve(|x| cell_step(x).0 - x, 0.0, high);
        let (reduction, fast, slow) = cell_step(g);
        self.reduction = reduction;
        self.cell_envelope += (peak * (dbfs_to_amp(-self.reduction) + self.limit_feed_forward) - self.cell_envelope)
            * self.envelope_coefficient;
        self.cell_fast = fast;
        self.cell_slow = slow;
        // The memory grows while the cell is lit and fades slowly
        let lit = fast > 0.1;
        self.cell_memory += (if lit { 1.0 } else { 0.0 } - self.cell_memory)
            * if lit { self.cell_memory_rise } else { self.cell_memory_fall };
        dbfs_to_amp(self.makeup - self.reduction)
    }

    pub fn reset(&mut self) {
        self.mean_square = 0.0;
        self.reduction = 0.0;
        self.slow_reduction = 0.0;
        self.voltages = [[0.0; 3]; 2];
        self.cell_fast = 0.0;
        self.cell_slow = 0.0;
        self.cell_memory = 0.0;
        self.cell_envelope = 0.0;
        self.dc_second = [0.0; 2];
        for u in &mut self.upsamplers {
            u.reset();
        }
        for d in &mut self.downsamplers {
            d.reset();
        }
        self.thrust_filter.reset();
        self.thrust_filter2.reset();
        self.high_pass.reset();
        self.low_pass.reset();
        if let Some(t) = self.transformer.and_then(|i| self.transformers[i].as_mut()) {
            t.reset();
        }
    }
}

/// The gate's operating modes: what has no knob. Their keys are saved with projects and never change (U30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GateMode {
    Std,
    Kick,
    Snare,
    Toms,
    Vox,
    Guitar,
    Bass,
    Noise,
    Expander,
}

impl GateMode {
    pub const DISPLAY_ORDER: [GateMode; 9] = [
        GateMode::Std,
        GateMode::Kick,
        GateMode::Snare,
        GateMode::Toms,
        GateMode::Vox,
        GateMode::Guitar,
        GateMode::Bass,
        GateMode::Noise,
        GateMode::Expander,
    ];

    pub fn key(self) -> &'static str {
        match self {
            GateMode::Std => "std",
            GateMode::Kick => "kick",
            GateMode::Snare => "snare",
            GateMode::Toms => "toms",
            GateMode::Vox => "vox",
            GateMode::Guitar => "guitar",
            GateMode::Bass => "bass",
            GateMode::Noise => "noise",
            GateMode::Expander => "expander",
        }
    }

    pub fn from_key(key: &str) -> Option<GateMode> {
        Self::DISPLAY_ORDER.into_iter().find(|m| m.key() == key)
    }

    /// What a mode sets (legado/docs/pesquisa-compressores.md, part 2)
    pub fn spec(self) -> GateModeSpec {
        let s = |expander, fast, hold, release, hysteresis, key_high_pass, key_low_pass| GateModeSpec {
            expander,
            fast,
            hold,
            release,
            hysteresis,
            key_high_pass,
            key_low_pass,
        };
        match self {
            GateMode::Kick => s(false, true, 0.05, 0.1, 3.0, 35.0, 250.0),
            GateMode::Snare => s(false, true, 0.05, 0.24, 3.0, 1000.0, 2000.0),
            GateMode::Toms => s(false, true, 0.1, 0.75, 6.0, 60.0, 3000.0),
            GateMode::Vox => s(true, false, 0.0, 0.1, 4.0, 150.0, 0.0),
            GateMode::Guitar => s(false, false, 0.05, 0.3, 6.0, 80.0, 0.0),
            GateMode::Bass => s(false, false, 0.2, 0.2, 3.0, 0.0, 0.0),
            GateMode::Noise => s(true, false, 0.05, 0.28, 3.0, 150.0, 0.0),
            GateMode::Expander => s(true, false, 0.0, 0.1, 0.0, 0.0, 0.0),
            GateMode::Std => s(false, true, 0.0, 0.33, 0.0, 0.0, 0.0),
        }
    }
}

/// What a gate mode sets
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GateModeSpec {
    pub expander: bool,
    pub fast: bool,
    /// s
    pub hold: f32,
    /// s
    pub release: f32,
    /// dB; 0 for the SSL's, which depends on the threshold
    pub hysteresis: f32,
    /// Hz, 0 for none
    pub key_high_pass: f32,
    /// Hz, 0 for none
    pub key_low_pass: f32,
}

/// The gate's controls
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GateSettings {
    /// dBu
    pub threshold: f32,
    /// dB, 0 is out
    pub range: f32,
    pub mode: GateMode,
}

impl Default for GateSettings {
    fn default() -> Self {
        Self { threshold: -10.0, range: 0.0, mode: GateMode::Std }
    }
}

/// s for 40 dB
const GATE_ATTACK: f32 = 0.0015;
const GATE_FAST_ATTACK: f32 = 0.0001;
const GATE_RATIO: f32 = 20.0;
const EXPANDER_RATIO: f32 = 2.0;
/// The key filters' Q: the legacy program's literal, a hair under 1/sqrt(2)
#[allow(clippy::approx_constant)]
const KEY_Q: f64 = 0.7071;
/// s, of the key's peak envelope, so it does not chatter
const GATE_KEY_RELEASE: f32 = 0.01;

/// The gate/expander of an SSL E series channel, stereo-linked: THRESHOLD (-30 to +10 dBu) and RANGE (0, out, to
/// 40 dB) on knobs; the mode sets the rest:
/// - a 20:1 gate or a 2:1 expander;
/// - the attack, a slope in dB: 40 dB in 1.5 ms (normal) or 100 us (fast);
/// - hold, release (a time constant in dB, so closing further takes longer), hysteresis (the SSL's, growing as the
///   threshold is lowered, or the mode's own);
/// - the key's filters, so that e.g. the kick's gate does not open on the hi-hat.
#[derive(Clone, Debug)]
pub struct ChannelGate {
    settings: GateSettings,
    sample_rate: f32,
    spec: GateModeSpec,
    key_high_pass: RampedBiquad,
    key_low_pass: RampedBiquad,
    key_filtered: bool,
    /// dB per frame
    attack_step: f32,
    release_coefficient: f32,
    hold_frames: u64,
    key_release: f32,
    /// The key's peak envelope
    envelope: f32,
    open: bool,
    holding: u64,
    /// dB
    reduction: f32,
}

impl ChannelGate {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            settings: GateSettings::default(),
            sample_rate,
            spec: GateModeSpec::default(),
            key_high_pass: RampedBiquad::default(),
            key_low_pass: RampedBiquad::default(),
            key_filtered: false,
            attack_step: 0.0,
            release_coefficient: 0.0,
            hold_frames: 0,
            key_release: 0.0,
            envelope: 0.0,
            open: false,
            holding: 0,
            reduction: 0.0,
        }
    }

    /// The SSL's hysteresis at a threshold in dBu
    pub fn hysteresis(threshold: f32) -> f32 {
        2.0 + 6.0 * ((10.0 - threshold) / 40.0).clamp(0.0, 1.0)
    }

    pub fn is_active(&self) -> bool {
        self.settings.range > 0.0
    }

    /// dB
    pub fn reduction(&self) -> f32 {
        self.reduction
    }

    pub fn begin(&mut self, settings: GateSettings) {
        let mode_changed = settings.mode != self.settings.mode;
        self.settings = settings;
        self.spec = settings.mode.spec();
        let rate = self.sample_rate;
        self.attack_step = 40.0 / ((if self.spec.fast { GATE_FAST_ATTACK } else { GATE_ATTACK }) * rate);
        self.release_coefficient = 1.0 - crate::math::expf(-1.0 / (self.spec.release * rate));
        self.hold_frames = (self.spec.hold * rate) as u64;
        self.key_release = crate::math::expf(-1.0 / (GATE_KEY_RELEASE * rate));
        self.key_filtered = self.spec.key_high_pass > 0.0 || self.spec.key_low_pass > 0.0;
        let rate64 = f64::from(rate);
        self.key_high_pass.set_target(if self.spec.key_high_pass > 0.0 {
            Biquad::high_pass(f64::from(self.spec.key_high_pass), KEY_Q, rate64)
        } else {
            Biquad::IDENTITY
        });
        self.key_low_pass.set_target(if self.spec.key_low_pass > 0.0 {
            Biquad::low_pass(f64::from(self.spec.key_low_pass), KEY_Q, rate64)
        } else {
            Biquad::IDENTITY
        });
        self.key_high_pass.finish();
        self.key_low_pass.finish();
        if mode_changed {
            self.key_high_pass.reset();
            self.key_low_pass.reset();
        }
    }

    /// The gain for a frame, linear
    #[inline]
    pub fn gain(&mut self, mut left: f32, mut right: f32) -> f32 {
        if self.key_filtered {
            left = self.key_low_pass.process(self.key_high_pass.process(f64::from(left), 0, 1.0), 0, 1.0) as f32;
            right = self.key_low_pass.process(self.key_high_pass.process(f64::from(right), 1, 1.0), 1, 1.0) as f32;
        }
        // The key: a peak envelope, quick to rise and briefly held, so that zero crossings do not close the gate
        self.envelope = left.abs().max(right.abs()).max(self.envelope * self.key_release);
        let level = level_of(self.envelope);
        let threshold = self.settings.threshold + DBU_TO_DBFS;
        let hysteresis =
            if self.spec.hysteresis > 0.0 { self.spec.hysteresis } else { Self::hysteresis(self.settings.threshold) };

        if level >= threshold {
            self.open = true;
            self.holding = self.hold_frames;
        } else if level < threshold - hysteresis {
            if self.holding > 0 {
                self.holding -= 1;
            } else {
                self.open = false;
            }
        }

        let ratio = if self.spec.expander { EXPANDER_RATIO } else { GATE_RATIO };
        let target =
            if self.open { 0.0 } else { self.settings.range.min((threshold - level).max(0.0) * (ratio - 1.0)) };
        if target < self.reduction {
            self.reduction = target.max(self.reduction - self.attack_step);
        } else {
            self.reduction += (target - self.reduction) * self.release_coefficient;
        }
        dbfs_to_amp(-self.reduction)
    }

    pub fn reset(&mut self) {
        self.open = false;
        self.holding = 0;
        self.envelope = 0.0;
        self.reduction = 0.0;
        self.key_high_pass.reset();
        self.key_low_pass.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(model: CompressorModel, amount: f32, ratio: f32, input_db: f32) -> f32 {
        let rate = 48000.0;
        let mut c = ChannelCompressor::new(rate);
        c.begin(CompressorSettings { model, amount, ratio, speed: CompressorSpeed::Norm, release: 0.5 });
        let amplitude = dbfs_to_amp(input_db);
        let mut peak = 0.0f32;
        for n in 0..48000 {
            let x = amplitude * crate::math::sinf(2.0 * f32c::PI * 1000.0 * n as f32 / rate);
            let (mut l, mut r) = (x, x);
            c.process(&mut l, &mut r);
            if n > 36000 {
                peak = peak.max(l.abs());
            }
        }
        amp_to_dbfs(peak / amplitude)
    }

    #[test]
    fn every_model_keeps_the_nominal_level_and_compresses_loud_signals() {
        let nominal = NOMINAL_RMS + 3.01;
        for model in CompressorModel::DISPLAY_ORDER {
            let at_nominal = run(model, 0.6, 0.7, nominal);
            assert!(at_nominal.abs() < 1.5, "{model:?} at nominal: {at_nominal}");
            let loud = run(model, 0.6, 0.7, nominal + 15.0);
            assert!(loud < at_nominal - 2.0, "{model:?} loud: {loud} vs {at_nominal}");
        }
    }

    #[test]
    fn gate_closes_on_quiet_signals() {
        let rate = 48000.0;
        let mut g = ChannelGate::new(rate);
        g.begin(GateSettings { threshold: -10.0, range: 40.0, mode: GateMode::Std });
        let mut gain = 1.0;
        // Closing takes the release's time constant (0.33 s): 3 s is 9 of them
        for _ in 0..3 * 48000 {
            gain = g.gain(0.001, 0.001);
        }
        assert!(amp_to_dbfs(gain) < -39.9, "{gain}");
    }

    #[test]
    fn keys_round_trip() {
        for m in CompressorModel::DISPLAY_ORDER {
            assert_eq!(CompressorModel::from_key(m.key()), Some(m));
        }
        for m in GateMode::DISPLAY_ORDER {
            assert_eq!(GateMode::from_key(m.key()), Some(m));
        }
    }
}
