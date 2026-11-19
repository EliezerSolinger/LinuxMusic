//! A triode stage, as at the input of a tube preamp (legacy `TubeStage`).

use crate::math::{exp, f64c::PI, log1p, pow};

/// log(1 + e^x), without overflow
fn softplus(x: f64) -> f64 {
    if x > 30.0 { x } else { log1p(exp(x)) }
}
/// The derivative of [`softplus`], the logistic function
fn logistic(x: f64) -> f64 {
    1.0 / (1.0 + exp(-x))
}

/// Finds where an increasing function crosses 0 between @p low and @p high
fn bisect(f: impl Fn(f64) -> f64, mut low: f64, mut high: f64) -> f64 {
    for _ in 0..56 {
        let middle = (low + high) / 2.0;
        if f(middle) < 0.0 { low = middle } else { high = middle }
    }
    (low + high) / 2.0
}

// Dempwolf and Zölzer's grid current, with their parameters for the 12AX7 (its constant leakage left out: it would
// only offset the grid)
const GRID_CONDUCTANCE: f64 = 6.06e-4;
const GRID_EXPONENT: f64 = 1.354;
const GRID_SHARPNESS: f64 = 13.9;

/// A triode, after Norman Koren ("Improved VT models for SPICE simulations", 1996) for its plate current, and
/// Kristjan Dempwolf and Udo Zölzer ("A physically-motivated triode model for circuit simulations", DAFx 2011) for
/// its grid current, with its interelectrode capacitances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triode {
    /// Koren's parameters
    pub mu: f64,
    pub ex: f64,
    pub kg1: f64,
    pub kp: f64,
    pub kvb: f64,
    /// F
    pub grid_to_plate: f64,
    /// F
    pub grid_to_cathode: f64,
}

/// A current and its derivatives by the grid and the plate voltages
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Current {
    pub value: f64,
    pub by_grid: f64,
    pub by_plate: f64,
}

impl Triode {
    /// The plate current for the grid and plate voltages to the cathode, and its derivatives
    pub fn plate_current(&self, vgk: f64, vpk: f64) -> Current {
        if vpk <= 0.0 {
            return Current::default();
        }
        let root = (self.kvb + vpk * vpk).sqrt();
        let x = self.kp * (1.0 / self.mu + vgk / root);
        let soft = softplus(x);
        let e1 = vpk / self.kp * soft;
        if e1 <= 1e-12 {
            return Current::default();
        }
        let sigma = logistic(x);
        let power = pow(e1, self.ex - 1.0);
        let value = 2.0 * power * e1 / self.kg1;
        let by_e1 = 2.0 * self.ex * power / self.kg1;
        let e1_by_grid = vpk * sigma / root;
        let e1_by_plate = soft / self.kp - vpk * vpk * vgk * sigma / (root * root * root);
        Current { value, by_grid: by_e1 * e1_by_grid, by_plate: by_e1 * e1_by_plate }
    }

    /// The grid current for the grid voltage to the cathode, and its derivative
    pub fn grid_current(&self, vgk: f64) -> Current {
        let x = GRID_SHARPNESS * vgk;
        let soft = softplus(x) / GRID_SHARPNESS;
        if soft <= 1e-300 {
            return Current::default();
        }
        let power = pow(soft, GRID_EXPONENT - 1.0);
        Current {
            value: GRID_CONDUCTANCE * power * soft,
            by_grid: GRID_CONDUCTANCE * GRID_EXPONENT * power * logistic(x),
            by_plate: 0.0,
        }
    }
}

/// A common cathode stage, as at the input of a tube preamp:
///
/// ```text
///   in -- Ci --+-- Rg --- grid      plate --- Rp --- B+ --- Rs --- supply
///              |                    |                |
///             Rgl       cathode     +-- Co --+       Cs
///              |        |   |                |       |
///             gnd      Rk   Ck              RL      gnd
///                       |   |                |
///                      gnd gnd              gnd (out)
/// ```
///
/// Ci and Rgl couple the input (Ci charges through the grid current: blocking), Rg is the grid stopper, which with
/// the Miller capacitance rolls off the treble, Ck bypasses the cathode resistor, Co and RL couple the output, and Rs
/// and Cs filter the supply, which sags under load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TubeCircuit {
    pub tube: Triode,
    /// V at the plate resistor, idle
    pub b_plus: f64,
    pub plate_resistor: f64,
    pub cathode_resistor: f64,
    pub cathode_capacitor: f64,
    pub grid_stopper: f64,
    pub input_capacitor: f64,
    pub grid_leak: f64,
    pub output_capacitor: f64,
    pub load: f64,
    pub supply_resistor: f64,
    pub supply_capacitor: f64,
}

impl TubeCircuit {
    /// A stage with the usual coupling, grid stopper, load and supply filter
    pub const fn new(
        tube: Triode,
        b_plus: f64,
        plate_resistor: f64,
        cathode_resistor: f64,
        cathode_capacitor: f64,
    ) -> Self {
        Self {
            tube,
            b_plus,
            plate_resistor,
            cathode_resistor,
            cathode_capacitor,
            grid_stopper: 68e3,
            input_capacitor: 22e-9,
            grid_leak: 1e6,
            output_capacitor: 22e-9,
            load: 1e6,
            supply_resistor: 10e3,
            supply_capacitor: 22e-6,
        }
    }
}

/// A point of the static curves for an input voltage: the plate's AC voltage, the plate current and the grid current
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TubePoint {
    pub plate: f32,
    pub plate_current: f32,
    pub grid_current: f32,
}

const TABLE_SIZE: usize = 8193;
/// V, either side
const TABLE_RANGE: f32 = 60.0;

/// What a [`TubeCircuit`] does, worked out once: its operating point, its gain, where it starts to clip, and its
/// static curves (with the cathode at its idle voltage, as its capacitor holds it, and the plate on its AC load line).
/// Making one allocates and takes time: do it when preparing, never on the audio thread.
#[derive(Clone, Debug)]
pub struct TubeModel {
    pub circuit: TubeCircuit,
    /// Idle voltages and current
    pub cathode: f64,
    pub plate: f64,
    pub current: f64,
    /// Before the supply resistor
    pub supply: f64,
    /// Midband, from the input to the output (negative: the stage inverts)
    pub gain: f64,
    /// The input peak at which it starts to clip, on whichever side comes first (V)
    pub headroom: f64,
    table: Vec<[f32; 3]>,
}

impl TubeModel {
    pub fn new(circuit: TubeCircuit) -> Self {
        let c = circuit;
        // The operating point: the current through the cathode resistor biases the grid
        let current = bisect(
            |i| {
                i - c
                    .tube
                    .plate_current(-i * c.cathode_resistor, c.b_plus - i * (c.plate_resistor + c.cathode_resistor))
                    .value
            },
            0.0,
            c.b_plus / (c.plate_resistor + c.cathode_resistor),
        );
        let mut model = TubeModel {
            circuit,
            cathode: current * c.cathode_resistor,
            plate: c.b_plus - current * c.plate_resistor,
            current,
            supply: c.b_plus + current * c.supply_resistor,
            gain: 0.0,
            headroom: 0.0,
            table: Vec::with_capacity(TABLE_SIZE),
        };

        for i in 0..TABLE_SIZE {
            let input = f64::from(-TABLE_RANGE) + 2.0 * f64::from(TABLE_RANGE) * i as f64 / (TABLE_SIZE - 1) as f64;
            let point = model.solve(input);
            model.table.push([point.plate, point.plate_current, point.grid_current]);
        }

        let step = 1e-3f32;
        model.gain = f64::from(model.at(step).plate - model.at(-step).plate) / (2.0 * f64::from(step));

        // Where it starts to clip: where its slope has fallen to half its midband slope, on either side
        let clips_at = |direction: f32| -> f32 {
            let step = 2.0 * TABLE_RANGE / (TABLE_SIZE - 1) as f32;
            let mut input = step;
            while input < TABLE_RANGE {
                let slope = (model.at(direction * (input + step)).plate - model.at(direction * input).plate)
                    / (direction * step);
                if f64::from(slope.abs()) < 0.5 * model.gain.abs() {
                    return input;
                }
                input += step;
            }
            TABLE_RANGE
        };
        model.headroom = f64::from(clips_at(1.0).min(clips_at(-1.0)));
        model
    }

    fn solve(&self, input: f64) -> TubePoint {
        let c = &self.circuit;
        let cathode = self.cathode;
        // The grid: the input, less what the grid current drops across the grid stopper (the input capacitor passes
        // the signal, the cathode's capacitor holds it at its idle voltage)
        let grid = bisect(
            |vg| vg + c.grid_stopper * c.tube.grid_current(vg - cathode).value - input,
            input.min(cathode) - 2.0,
            input,
        );
        let vgk = grid - cathode;

        // The plate, on the AC load line: the plate resistor and the load, through the output capacitor
        let load = c.plate_resistor * c.load / (c.plate_resistor + c.load);
        let plate = self.plate;
        let current = self.current;
        let vp = bisect(
            |v| v - plate + load * (c.tube.plate_current(vgk, v - cathode).value - current),
            cathode,
            plate + load * current,
        );

        TubePoint {
            plate: (vp - plate) as f32,
            plate_current: c.tube.plate_current(vgk, vp - cathode).value as f32,
            grid_current: c.tube.grid_current(vgk).value as f32,
        }
    }

    /// The static curves at an input voltage (interpolated in the table)
    #[inline]
    pub fn at(&self, input: f32) -> TubePoint {
        let position = (input + TABLE_RANGE) / (2.0 * TABLE_RANGE) * (TABLE_SIZE - 1) as f32;
        let point = |p: &[f32; 3]| TubePoint { plate: p[0], plate_current: p[1], grid_current: p[2] };
        // (A NaN input goes to the first point, as the comparisons of the legacy program did)
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(position > 0.0) {
            return point(&self.table[0]);
        }
        if position >= (TABLE_SIZE - 1) as f32 {
            return point(&self.table[TABLE_SIZE - 1]);
        }
        let i = position as usize;
        let f = position - i as f32;
        let a = &self.table[i];
        let b = &self.table[i + 1];
        TubePoint {
            plate: a[0] + (b[0] - a[0]) * f,
            plate_current: a[1] + (b[1] - a[1]) * f,
            grid_current: a[2] + (b[2] - a[2]) * f,
        }
    }
}

const NODES: usize = 6; // input coupling, grid, cathode, plate, B+, output
const COUPLING: usize = 0;
const GRID: usize = 1;
const CATHODE: usize = 2;
const PLATE: usize = 3;
const B_PLUS: usize = 4;
const OUTPUT: usize = 5;
/// "Ground": a node that is not one of the six
const GROUND: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Default)]
struct Capacitor {
    a: usize,
    b: usize,
    /// 2C/T
    g: f64,
    /// History
    j: f64,
}

/// A tube stage at a given (oversampled) rate, in either of two modes:
/// - light: the static curve, with what the circuit's memory does most audibly approximated around it: the Miller
///   capacitance as a low-pass at the grid, the input capacitor charged by the grid current (blocking), and the
///   supply sagging with the average plate current;
/// - circuit: the whole circuit solved at each sample (nodal analysis with trapezoidal capacitors, the DK method:
///   Newton's method on the two nonlinear currents only).
///
/// Both take the input in volts and give the stage's output in volts, around 0.
#[derive(Clone, Debug)]
pub struct TubeStage {
    // Light
    miller_coefficient: f32,
    miller: f32,
    /// V the input capacitor holds from the grid current
    blocking: f32,
    blocking_charge: f32,
    blocking_leak: f32,
    average_current: f32,
    sag_coefficient: f32,

    // Circuit
    nonlinear: [[f64; 2]; NODES],
    inverse: [[f64; NODES]; NODES],
    k: [[f64; 2]; 2],
    capacitors: [Capacitor; 5],
    input_conductance: f64,
    input_history: f64,
    voltages: [f64; NODES],
    vgk: f64,
    vpk: f64,
}

fn voltage(v: &[f64; NODES], node: usize) -> f64 {
    if node == GROUND { 0.0 } else { v[node] }
}

impl TubeStage {
    /// Prepared for @p model at @p sample_rate (the oversampled rate it runs at)
    pub fn new(model: &TubeModel, sample_rate: f64) -> Self {
        let c = &model.circuit;
        let t = 1.0 / sample_rate;

        // Light
        let miller = c.tube.grid_to_cathode + c.tube.grid_to_plate * (1.0 + model.gain.abs());
        let miller_cutoff = 1.0 / (2.0 * PI * c.grid_stopper * miller);

        // Circuit: the linear part's matrix, with the capacitors as conductances (trapezoidal rule)
        let g = |capacitance: f64| 2.0 * capacitance / t;
        let input_conductance = g(c.input_capacitor);
        let capacitors = [
            Capacitor { a: GRID, b: CATHODE, g: g(c.tube.grid_to_cathode), j: 0.0 },
            Capacitor { a: GRID, b: PLATE, g: g(c.tube.grid_to_plate), j: 0.0 },
            Capacitor { a: CATHODE, b: GROUND, g: g(c.cathode_capacitor), j: 0.0 },
            Capacitor { a: B_PLUS, b: GROUND, g: g(c.supply_capacitor), j: 0.0 },
            Capacitor { a: PLATE, b: OUTPUT, g: g(c.output_capacitor), j: 0.0 },
        ];
        let mut a = [[0.0f64; NODES]; NODES];
        let mut conductance = |n1: usize, n2: usize, value: f64| {
            if n1 != GROUND {
                a[n1][n1] += value;
            }
            if n2 != GROUND {
                a[n2][n2] += value;
            }
            if n1 != GROUND && n2 != GROUND {
                a[n1][n2] -= value;
                a[n2][n1] -= value;
            }
        };
        conductance(COUPLING, GROUND, input_conductance); // to the input, a known voltage
        conductance(COUPLING, GROUND, 1.0 / c.grid_leak);
        conductance(COUPLING, GRID, 1.0 / c.grid_stopper);
        conductance(CATHODE, GROUND, 1.0 / c.cathode_resistor);
        conductance(PLATE, B_PLUS, 1.0 / c.plate_resistor);
        conductance(B_PLUS, GROUND, 1.0 / c.supply_resistor); // to the supply, a known voltage
        conductance(OUTPUT, GROUND, 1.0 / c.load);
        for capacitor in &capacitors {
            conductance(capacitor.a, capacitor.b, capacitor.g);
        }

        // Its inverse (Gauss-Jordan with partial pivoting)
        let mut inverse = [[0.0f64; NODES]; NODES];
        for (i, row) in inverse.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        for col in 0..NODES {
            let mut pivot = col;
            for row in col + 1..NODES {
                if a[row][col].abs() > a[pivot][col].abs() {
                    pivot = row;
                }
            }
            a.swap(col, pivot);
            inverse.swap(col, pivot);
            let scale = 1.0 / a[col][col];
            for k in 0..NODES {
                a[col][k] *= scale;
                inverse[col][k] *= scale;
            }
            for row in 0..NODES {
                if row == col {
                    continue;
                }
                let factor = a[row][col];
                for k in 0..NODES {
                    a[row][k] -= factor * a[col][k];
                    inverse[row][k] -= factor * inverse[col][k];
                }
            }
        }

        // The nonlinear currents: the grid current from the grid to the cathode, the plate current from the plate
        // to the cathode
        let mut nonlinear = [[0.0f64; 2]; NODES];
        for n in 0..NODES {
            nonlinear[n][0] = inverse[n][CATHODE] - inverse[n][GRID];
            nonlinear[n][1] = inverse[n][CATHODE] - inverse[n][PLATE];
        }
        let mut k = [[0.0f64; 2]; 2];
        for i in 0..2 {
            k[0][i] = nonlinear[GRID][i] - nonlinear[CATHODE][i];
            k[1][i] = nonlinear[PLATE][i] - nonlinear[CATHODE][i];
        }

        let mut stage = TubeStage {
            miller_coefficient: (1.0 - exp(-2.0 * PI * miller_cutoff * t)) as f32,
            miller: 0.0,
            blocking: 0.0,
            blocking_charge: (t / c.input_capacitor) as f32,
            blocking_leak: (t / (c.grid_leak * c.input_capacitor)) as f32,
            average_current: 0.0,
            sag_coefficient: (1.0 - exp(-t / (c.supply_resistor * c.supply_capacitor))) as f32,
            nonlinear,
            inverse,
            k,
            capacitors,
            input_conductance,
            input_history: 0.0,
            voltages: [0.0; NODES],
            vgk: 0.0,
            vpk: 0.0,
        };
        stage.reset(model);
        stage
    }

    /// Back to the operating point
    pub fn reset(&mut self, model: &TubeModel) {
        self.miller = 0.0;
        self.blocking = 0.0;
        self.average_current = model.current as f32;
        self.voltages = [0.0, 0.0, model.cathode, model.plate, model.circuit.b_plus, 0.0];
        for capacitor in &mut self.capacitors {
            capacitor.j = capacitor.g * (voltage(&self.voltages, capacitor.a) - voltage(&self.voltages, capacitor.b));
        }
        self.input_history = 0.0;
        self.vgk = -model.cathode;
        self.vpk = model.plate - model.cathode;
    }

    #[inline]
    pub fn process_light(&mut self, model: &TubeModel, input: f32) -> f32 {
        self.miller += (input - self.miller) * self.miller_coefficient;
        // The supply sags as the average plate current rises; the stage's curves scale with it
        let sag = (model.circuit.supply_resistor / model.circuit.b_plus) as f32
            * (self.average_current - model.current as f32);
        let supply = (1.0 - sag).clamp(0.5, 1.5);
        let point = model.at((self.miller - self.blocking) / supply);
        self.blocking += point.grid_current * self.blocking_charge - self.blocking * self.blocking_leak;
        self.average_current += (point.plate_current - self.average_current) * self.sag_coefficient;
        point.plate * supply
    }

    #[inline]
    pub fn process_circuit(&mut self, model: &TubeModel, input: f32) -> f32 {
        let c = &model.circuit;
        let input = f64::from(input);
        // The linear part's sources: the input through its capacitor, the supply, the capacitors' histories
        let mut b = [0.0f64; NODES];
        b[COUPLING] += self.input_conductance * input - self.input_history;
        b[B_PLUS] += model.supply / c.supply_resistor;
        for capacitor in &self.capacitors {
            if capacitor.a != GROUND {
                b[capacitor.a] += capacitor.j;
            }
            if capacitor.b != GROUND {
                b[capacitor.b] -= capacitor.j;
            }
        }
        let mut v0 = [0.0f64; NODES];
        for n in 0..NODES {
            let mut sum = 0.0;
            for k in 0..NODES {
                sum += self.inverse[n][k] * b[k];
            }
            v0[n] = sum;
        }
        let p0g = v0[GRID] - v0[CATHODE];
        let p0p = v0[PLATE] - v0[CATHODE];

        // Newton's method on the controlling voltages
        let k = &self.k;
        let mut vgk = self.vgk;
        let mut vpk = self.vpk;
        for _ in 0..40 {
            let ig = c.tube.grid_current(vgk);
            let ip = c.tube.plate_current(vgk, vpk);
            let f0 = vgk - p0g - (k[0][0] * ig.value + k[0][1] * ip.value);
            let f1 = vpk - p0p - (k[1][0] * ig.value + k[1][1] * ip.value);
            let j00 = 1.0 - (k[0][0] * ig.by_grid + k[0][1] * ip.by_grid);
            let j01 = -k[0][1] * ip.by_plate;
            let j10 = -(k[1][0] * ig.by_grid + k[1][1] * ip.by_grid);
            let j11 = 1.0 - k[1][1] * ip.by_plate;
            let determinant = j00 * j11 - j01 * j10;
            if determinant == 0.0 {
                break;
            }
            let dg = ((f0 * j11 - f1 * j01) / determinant).clamp(-1.0, 1.0);
            let dp = ((f1 * j00 - f0 * j10) / determinant).clamp(-50.0, 50.0);
            vgk -= dg;
            vpk -= dp;
            if dg.abs() < 1e-9 && dp.abs() < 1e-7 {
                break;
            }
        }
        let ig = c.tube.grid_current(vgk);
        let ip = c.tube.plate_current(vgk, vpk);
        self.vgk = vgk;
        self.vpk = vpk;

        for n in 0..NODES {
            self.voltages[n] = v0[n] + self.nonlinear[n][0] * ig.value + self.nonlinear[n][1] * ip.value;
        }

        // The capacitors' histories for the next sample
        for capacitor in &mut self.capacitors {
            let across = voltage(&self.voltages, capacitor.a) - voltage(&self.voltages, capacitor.b);
            capacitor.j = 2.0 * capacitor.g * across - capacitor.j;
        }
        self.input_history = 2.0 * self.input_conductance * (input - self.voltages[COUPLING]) - self.input_history;

        self.voltages[OUTPUT] as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ax7() -> TubeCircuit {
        let t = Triode {
            mu: 100.0,
            ex: 1.4,
            kg1: 1060.0,
            kp: 600.0,
            kvb: 300.0,
            grid_to_plate: 1.7e-12,
            grid_to_cathode: 1.6e-12,
        };
        TubeCircuit::new(t, 250.0, 100e3, 1.5e3, 22e-6)
    }

    #[test]
    fn the_12ax7_is_biased_and_amplifies() {
        let m = TubeModel::new(ax7());
        assert!(m.current > 0.3e-3 && m.current < 1.5e-3, "{}", m.current);
        assert!(m.plate > 100.0 && m.plate < 220.0, "{}", m.plate);
        assert!(m.gain < -30.0 && m.gain > -80.0, "{}", m.gain);
        assert!(m.headroom > 0.5 && m.headroom < 5.0, "{}", m.headroom);
    }

    #[test]
    fn light_and_circuit_agree_for_a_small_signal() {
        let m = TubeModel::new(ax7());
        let rate = 192000.0;
        let mut light = TubeStage::new(&m, rate);
        let mut circuit = TubeStage::new(&m, rate);
        let (mut pl, mut pc) = (0.0f32, 0.0f32);
        for n in 0..19200 {
            let x = 0.05 * crate::math::sin(2.0 * PI * 1000.0 * f64::from(n) / rate) as f32;
            let yl = light.process_light(&m, x);
            let yc = circuit.process_circuit(&m, x);
            if n > 9600 {
                pl = pl.max(yl.abs());
                pc = pc.max(yc.abs());
            }
        }
        assert!((pl / pc - 1.0).abs() < 0.1, "{pl} {pc}");
    }
}
