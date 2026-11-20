//! The hot path of the compiled table (feeding messages, feedback) never allocates nor frees (Parte 2, rule 5)

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use lm_midimap::{
    ANY_PORT, Button, Encoding, Entry, Feedback, MapTable, Message, Mode, Ring, Source, Takeover, Timing, Transform,
};

#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

#[test]
fn feeding_and_feedback_do_not_allocate() {
    let mut entries = Vec::new();
    for i in 0..64u8 {
        let message = match i % 6 {
            0 => Message::Cc { number: i },
            1 => Message::Cc14 { msb: i % 32 },
            2 => Message::Nrpn { number: u16::from(i) },
            3 => Message::Note { key: i },
            4 => Message::PitchBend,
            _ => Message::ProgramChange,
        };
        let mode = if i % 5 == 0 { Mode::Relative(Encoding::SignMagnitude) } else { Mode::Absolute };
        entries.push(Entry {
            id: u64::from(i),
            source: Source { port: if i % 7 == 0 { ANY_PORT } else { u16::from(i % 3) }, channel: None, message },
            transform: Transform { mode, takeover: Takeover::Scale, button: Button::Toggle, ..Transform::default() },
            switch: message.is_note() || message == Message::ProgramChange,
            consume: true,
            feedback: Some(Feedback { message, channel: 0, ring: if i % 9 == 0 { Ring::Wrap } else { Ring::None } }),
            touch: Some(Source { port: 0, channel: Some(0), message: Message::Note { key: 0x68 } }),
            target: u32::from(i),
        });
    }
    let mut table = MapTable::new(entries, 4, Timing::samples(48000));
    let values: [std::cell::Cell<f32>; 64] = std::array::from_fn(|_| std::cell::Cell::new(0.5));
    let mut out = 0usize;
    assert_no_alloc(|| {
        for t in 0..20_000u64 {
            let b = (t % 128) as u8;
            let msgs: [[u8; 3]; 6] =
                [[0xB0, b % 64, b], [0xB0, 99, 0], [0xB0, 98, b % 64], [0xB0, 6, b], [0x90, b, 100], [0xE0, b, b]];
            for m in &msgs {
                table.feed(
                    (t % 4) as u16,
                    m,
                    t * 64,
                    |tg| values[*tg as usize].get(),
                    |ev, tg| {
                        if let lm_midimap::Event::Value { out, .. } = ev {
                            values[*tg as usize].set(out.value);
                        }
                    },
                );
            }
            table.feed(0, &[0xC0, b], t * 64, |_| 0.0, |_, _| {});
            for (e, v) in values.iter().enumerate() {
                table.offer(e, v.get());
            }
            table.poll_feedback(t * 64, |_, _, n| out += usize::from(n));
        }
    });
    assert!(out > 0);
}
