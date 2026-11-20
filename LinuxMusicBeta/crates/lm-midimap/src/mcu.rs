//! The Mackie Control Universal protocol (interfaces.md, I-21.3): the numbers of its controls, the display SysEx,
//! the meters and the LED rings, and [`Surface`], a simulated MCU for tests (it reads what the host sends, like the
//! hardware, and produces what the hardware would send).
//!
//! Everything a strip does that is a value (fader, V-pot, buttons) is an ordinary mapping (the profile
//! `controllers/mcu.json`); only the display and the meters need code, in the session's MCU module.

use crate::message::Raw;

/// Strips per bank
pub const STRIPS: u8 = 8;
/// Fader `i` (0..8) is pitch bend on channel `i`; the master fader is channel 8
pub const MASTER_FADER_CHANNEL: u8 = 8;
/// Fader touch: note `0x68 + i`; master 0x70
pub const TOUCH: u8 = 0x68;
pub const MASTER_TOUCH: u8 = 0x70;
/// V-pot rotation: CC `0x10 + i`, sign-magnitude steps
pub const VPOT: u8 = 0x10;
/// V-pot LED ring: CC `0x30 + i`
pub const RING: u8 = 0x30;
/// Strip buttons: note + i
pub const REC: u8 = 0x00;
pub const SOLO: u8 = 0x08;
pub const MUTE: u8 = 0x10;
pub const SELECT: u8 = 0x18;
pub const VPOT_PRESS: u8 = 0x20;
/// Global buttons (notes)
pub const BANK_LEFT: u8 = 0x2E;
pub const BANK_RIGHT: u8 = 0x2F;
pub const CHANNEL_LEFT: u8 = 0x30;
pub const CHANNEL_RIGHT: u8 = 0x31;
pub const FLIP: u8 = 0x32;
pub const AUTOMATION_READ: u8 = 0x4A;
pub const AUTOMATION_WRITE: u8 = 0x4B;
pub const AUTOMATION_TRIM: u8 = 0x4C;
pub const AUTOMATION_TOUCH: u8 = 0x4D;
pub const AUTOMATION_LATCH: u8 = 0x4E;
pub const SAVE: u8 = 0x50;
pub const UNDO: u8 = 0x51;
pub const MARKER: u8 = 0x54;
pub const CYCLE: u8 = 0x56;
pub const CLICK: u8 = 0x59;
pub const REWIND: u8 = 0x5B;
pub const FORWARD: u8 = 0x5C;
pub const STOP: u8 = 0x5D;
pub const PLAY: u8 = 0x5E;
pub const RECORD: u8 = 0x5F;
pub const ZOOM: u8 = 0x64;
/// The jog wheel: CC 0x3C, sign-magnitude
pub const JOG: u8 = 0x3C;
/// The two-digit assignment display: CC 0x4B (left digit), 0x4A (right digit)
pub const ASSIGNMENT_LEFT: u8 = 0x4B;
pub const ASSIGNMENT_RIGHT: u8 = 0x4A;

/// The SysEx header of the main unit (an extender is 0x15)
pub const HEADER: [u8; 5] = [0xF0, 0x00, 0x00, 0x66, 0x14];
/// The LCD: 2 rows × 56 characters, 7 per strip
pub const LCD_WIDTH: usize = 56;
pub const CELL: usize = 7;
/// The largest display message: header, 0x12, offset, 112 characters, 0xF7
pub const MAX_SYSEX: usize = HEADER.len() + 2 + 2 * LCD_WIDTH + 1;

/// The display text of `text` at `row` (0 or 1) and `column` (0..56), cut to the row. Characters outside 0x20..0x7F
/// become spaces. Returns the message length written into `out`
pub fn lcd(row: u8, column: usize, text: &str, out: &mut [u8; MAX_SYSEX]) -> usize {
    let column = column.min(LCD_WIDTH);
    let mut n = 0;
    for b in HEADER {
        out[n] = b;
        n += 1;
    }
    out[n] = 0x12;
    out[n + 1] = (usize::from(row.min(1)) * LCD_WIDTH + column) as u8;
    n += 2;
    for c in text.chars().take(LCD_WIDTH - column) {
        out[n] = if (' '..='\u{7e}').contains(&c) { c as u8 } else { b' ' };
        n += 1;
    }
    out[n] = 0xF7;
    n + 1
}

/// Text cut or padded to one 7-character cell (the last character stays a space, as on the hardware, so cells
/// do not run together)
pub fn cell(text: &str) -> [u8; CELL] {
    let mut c = [b' '; CELL];
    for (i, ch) in text.chars().filter(|c| !c.is_control()).take(CELL - 1).enumerate() {
        c[i] = if ch.is_ascii() { ch as u8 } else { b'?' };
    }
    c
}

/// A meter level: 0..=12 (0 = −∞ … 12 = 0 dBFS), from a peak in dBFS. The MCU's scale has steps of about 5 dB at the
/// bottom and 2 dB near the top; this uses the common table
pub fn meter_level(dbfs: f32) -> u8 {
    const STEPS: [f32; 12] = [-60.0, -50.0, -40.0, -30.0, -20.0, -14.0, -10.0, -8.0, -6.0, -4.0, -2.0, 0.0];
    STEPS.iter().take_while(|s| dbfs >= **s).count() as u8
}

/// The meter message of strip `i` at `level` (channel pressure on channel 0)
pub fn meter(strip: u8, level: u8) -> [u8; 2] {
    [0xD0, ((strip & 7) << 4) | level.min(0x0F)]
}

/// A 7-segment character for the assignment display (CC value): letters and digits as the MCU encodes them
pub fn segment(c: char) -> u8 {
    let c = c.to_ascii_uppercase();
    match c {
        '@'..='_' => c as u8 - 0x40,
        ' '..='?' => c as u8,
        _ => 0x20,
    }
}

/// The device query the host sends, and the reply of a main unit starts with `HEADER, 0x01`
pub const QUERY: [u8; 7] = [0xF0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xF7];

pub fn is_mcu_sysex(data: &[u8]) -> bool {
    data.len() > HEADER.len() && data[..4] == HEADER[..4] && (data[4] == 0x14 || data[4] == 0x15)
}

/// A simulated Mackie Control: what the host sent to it (faders moved by the motors, LEDs, rings, the LCD, meters),
/// and the messages a hand on it would produce
#[derive(Clone, Debug)]
pub struct Surface {
    /// Fader positions 0..=16383 (8 strips + master), as the motors put them
    pub faders: [u16; 9],
    /// LED state of every note (velocity > 0 = on)
    pub leds: [bool; 128],
    /// Ring CC values per strip
    pub rings: [u8; 8],
    /// The LCD, 2 rows
    pub lcd: [[u8; LCD_WIDTH]; 2],
    pub meters: [u8; 8],
    /// The assignment display characters (left, right)
    pub assignment: [u8; 2],
    /// Messages received that the model did not understand
    pub unknown: usize,
}

impl Default for Surface {
    fn default() -> Self {
        Surface {
            faders: [0; 9],
            leds: [false; 128],
            rings: [0; 8],
            lcd: [[b' '; LCD_WIDTH]; 2],
            meters: [0; 8],
            assignment: [b' '; 2],
            unknown: 0,
        }
    }
}

impl Surface {
    /// Receives a message from the host
    pub fn receive(&mut self, bytes: &[u8]) {
        if bytes.first() == Some(&0xF0) {
            if is_mcu_sysex(bytes) && bytes.get(5) == Some(&0x12) && bytes.len() >= 8 {
                let start = usize::from(bytes[6]);
                for (i, &b) in bytes[7..bytes.len() - 1].iter().enumerate() {
                    let pos = start + i;
                    if pos < 2 * LCD_WIDTH {
                        self.lcd[pos / LCD_WIDTH][pos % LCD_WIDTH] = b;
                    }
                }
            } else {
                self.unknown += 1;
            }
            return;
        }
        if bytes.first().is_some_and(|b| b & 0xF0 == 0xD0) && bytes.len() >= 2 {
            let strip = usize::from(bytes[1] >> 4);
            if strip < 8 {
                let level = bytes[1] & 0x0F;
                if level <= 0x0C {
                    self.meters[strip] = level;
                }
            }
            return;
        }
        match Raw::parse(bytes) {
            Some(Raw::PitchBend { channel, value }) if channel <= MASTER_FADER_CHANNEL => {
                self.faders[usize::from(channel)] = value
            }
            Some(Raw::NoteOn { key, velocity, .. }) => self.leds[usize::from(key)] = velocity > 0,
            Some(Raw::NoteOff { key, .. }) => self.leds[usize::from(key)] = false,
            Some(Raw::Cc { number, value, .. }) if (RING..RING + 8).contains(&number) => {
                self.rings[usize::from(number - RING)] = value
            }
            Some(Raw::Cc { number, value, .. }) if number == ASSIGNMENT_LEFT || number == ASSIGNMENT_RIGHT => {
                self.assignment[usize::from(number == ASSIGNMENT_RIGHT)] = value
            }
            _ => self.unknown += 1,
        }
    }

    /// The text of strip `i`'s cell on `row`
    pub fn cell_text(&self, row: usize, strip: usize) -> String {
        let s = &self.lcd[row][strip * CELL..(strip + 1) * CELL];
        String::from_utf8_lossy(s).trim_end().to_owned()
    }

    /// A hand moves fader `i` to `value` (0..=16383)
    pub fn move_fader(i: u8, value: u16) -> [u8; 3] {
        Raw::PitchBend { channel: i, value }.encode().0
    }

    pub fn touch(i: u8, on: bool) -> [u8; 3] {
        [0x90, TOUCH + i, if on { 0x7F } else { 0 }]
    }

    /// V-pot `i` turned by `steps` (sign-magnitude, at most 63)
    pub fn turn(i: u8, steps: i32) -> [u8; 3] {
        let m = steps.unsigned_abs().min(63) as u8;
        [0xB0, VPOT + i, if steps < 0 { 0x40 | m } else { m }]
    }

    /// A button down or up
    pub fn press(note: u8, down: bool) -> [u8; 3] {
        [0x90, note, if down { 0x7F } else { 0 }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcd_message() {
        let mut out = [0u8; MAX_SYSEX];
        let n = lcd(1, 7, "Bass", &mut out);
        assert_eq!(&out[..n], &[0xF0, 0, 0, 0x66, 0x14, 0x12, 63, b'B', b'a', b's', b's', 0xF7]);
        let mut s = Surface::default();
        s.receive(&out[..n]);
        assert_eq!(s.cell_text(1, 1), "Bass");
        let long: String = "x".repeat(200);
        let n = lcd(0, 50, &long, &mut out);
        assert_eq!(n, HEADER.len() + 2 + 6 + 1);
        assert_eq!(cell("Electric Guitar"), *b"Electr ");
    }

    #[test]
    fn meters_and_leds() {
        assert_eq!(meter_level(-100.0), 0);
        assert_eq!(meter_level(-3.0), 10);
        assert_eq!(meter_level(0.0), 12);
        let mut s = Surface::default();
        s.receive(&meter(3, 9));
        assert_eq!(s.meters[3], 9);
        s.receive(&[0x90, MUTE + 2, 0x7F]);
        assert!(s.leds[usize::from(MUTE + 2)]);
        s.receive(&[0x90, MUTE + 2, 0]);
        assert!(!s.leds[usize::from(MUTE + 2)]);
        s.receive(&Surface::move_fader(4, 1234));
        assert_eq!(s.faders[4], 1234);
        assert_eq!(Surface::turn(1, -3), [0xB0, 0x11, 0x43]);
        assert_eq!(segment('a'), 0x01);
        assert_eq!(segment('3'), b'3');
    }
}
