//! The sounding notes and their expressions, for the formats whose plugins take expression values at points in time
//! (CLAP, VST3): the core's expression events carry a ramp; these send the ramp as a value every [`RAMP_STEP`]
//! frames until it ends, starting from the note's current value. Fixed capacity: nothing is allocated per block.

use lm_render::Expression;

/// The frames between two values of a ramp
pub const RAMP_STEP: u32 = 32;
const MAX_RAMPS: usize = 256;
const MAX_NOTES: usize = 512;

/// A sounding note
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub id: u32,
    pub channel: i16,
    pub key: i16,
    pan: f64,
    tuning: f64,
    pressure: f64,
    timbre: f64,
}

#[derive(Clone, Copy, Debug)]
struct Ramp {
    note: u32,
    expression: Expression,
    from: f64,
    to: f64,
    done: u32,
    length: u32,
    offset: u32,
}

/// One expression value to send: (frame in the block, the note, which expression, the core's value)
pub type Emit<'a> = &'a mut dyn FnMut(u32, &Note, Expression, f64);

pub struct Expressions {
    notes: Vec<Note>,
    ramps: Vec<Ramp>,
}

impl Expressions {
    pub fn new() -> Expressions {
        Expressions { notes: Vec::with_capacity(MAX_NOTES), ramps: Vec::with_capacity(MAX_RAMPS) }
    }

    pub fn note_on(&mut self, id: u32, channel: i16, key: i16) {
        if self.notes.len() == self.notes.capacity() {
            self.notes.remove(0);
        }
        self.notes.push(Note { id, channel, key, pan: 0.0, tuning: 0.0, pressure: 0.0, timbre: 0.0 });
    }

    pub fn note_off(&mut self, id: u32) {
        self.notes.retain(|n| n.id != id);
        self.ramps.retain(|r| r.note != id);
    }

    pub fn clear(&mut self) {
        self.notes.clear();
        self.ramps.clear();
    }

    fn send(&mut self, time: u32, note: u32, expression: Expression, value: f64, emit: Emit<'_>) {
        let unknown = Note { id: note, channel: -1, key: -1, pan: 0.0, tuning: 0.0, pressure: 0.0, timbre: 0.0 };
        let n = match self.notes.iter_mut().find(|n| n.id == note) {
            Some(n) => {
                match expression {
                    Expression::Pan => n.pan = value,
                    Expression::Tuning => n.tuning = value,
                    Expression::Pressure => n.pressure = value,
                    Expression::Timbre => n.timbre = value,
                }
                *n
            }
            None => unknown,
        };
        emit(time, &n, expression, value);
    }

    /// An expression event of the core: at once, or a ramp from the note's current value
    pub fn set(&mut self, time: u32, note: u32, expression: Expression, value: f64, ramp: u32, emit: Emit<'_>) {
        self.ramps.retain(|r| !(r.note == note && r.expression == expression));
        if ramp == 0 {
            self.send(time, note, expression, value, emit);
        } else if self.ramps.len() < self.ramps.capacity() {
            let from = self.notes.iter().find(|n| n.id == note).map_or(0.0, |n| match expression {
                Expression::Pan => n.pan,
                Expression::Tuning => n.tuning,
                Expression::Pressure => n.pressure,
                Expression::Timbre => n.timbre,
            });
            self.ramps.push(Ramp { note, expression, from, to: value, done: 0, length: ramp, offset: time });
        }
    }

    /// The ramps' values in this block; the ones that end are dropped
    pub fn run(&mut self, frames: u32, emit: Emit<'_>) {
        let mut k = 0;
        while k < self.ramps.len() {
            let r = self.ramps[k];
            let mut t = r.offset;
            let mut done = r.done;
            while t < frames && done < r.length {
                done = (done + RAMP_STEP).min(r.length);
                let x = r.from + (r.to - r.from) * f64::from(done) / f64::from(r.length);
                self.send(t, r.note, r.expression, x, emit);
                t += RAMP_STEP;
            }
            if done >= r.length {
                self.ramps.swap_remove(k);
            } else {
                self.ramps[k].done = done;
                self.ramps[k].offset = t.saturating_sub(frames);
                k += 1;
            }
        }
    }
}

/// Sorts a block's events by time, stably and without allocating (insertion sort: the lists are short and nearly
/// in order)
pub fn sort_by_time<T>(events: &mut [T], time: impl Fn(&T) -> u32) {
    for i in 1..events.len() {
        let mut j = i;
        while j > 0 && time(&events[j - 1]) > time(&events[j]) {
            events.swap(j - 1, j);
            j -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ramp_goes_from_the_current_value_in_steps_across_blocks() {
        let mut e = Expressions::new();
        e.note_on(1, 0, 60);
        let mut got = Vec::new();
        e.set(0, 1, Expression::Tuning, 2.0, 0, &mut |t, _, _, v| got.push((t, v)));
        e.set(16, 1, Expression::Tuning, 4.0, 64, &mut |t, _, _, v| got.push((t, v)));
        e.run(64, &mut |t, n, _, v| got.push((t + 1000 * u32::from(n.key == 60), v)));
        e.run(64, &mut |t, _, _, v| got.push((t, v)));
        assert_eq!(got, [(0, 2.0), (1016, 3.0), (1048, 4.0)]);
    }

    #[test]
    fn sorting_keeps_the_order_of_equal_times() {
        let mut v = [(3, 'a'), (1, 'b'), (3, 'c'), (0, 'd')];
        sort_by_time(&mut v, |x| x.0);
        assert_eq!(v, [(0, 'd'), (1, 'b'), (3, 'a'), (3, 'c')]);
    }
}
