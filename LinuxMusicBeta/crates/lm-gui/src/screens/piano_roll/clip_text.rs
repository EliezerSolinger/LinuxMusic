//! The notes on the system clipboard (F-NOT-019): a line of text per note, so a copy in one project or instance
//! pastes in another. The first line says what it is; anything else on the clipboard is not notes.

use super::music::{Bend, Note};

const HEAD: &str = "linuxmusic-notes/1";

/// The notes as text: `key start length velocity pan at:semitones,...`, starts from the first note
pub fn to_text(notes: &[Note]) -> String {
    let mut out = String::from(HEAD);
    for n in notes {
        let bend: Vec<String> = n.bend.iter().map(|b| format!("{}:{}", b.at, b.semitones)).collect();
        out.push_str(&format!(
            "\n{} {} {} {} {} {}",
            n.key,
            n.start,
            n.length,
            n.velocity,
            n.pan,
            if bend.is_empty() { "-".to_string() } else { bend.join(",") }
        ));
    }
    out
}

/// The notes of a text made by `to_text`; none when it is anything else
pub fn from_text(text: &str) -> Option<Vec<Note>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != HEAD {
        return None;
    }
    let mut out = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [key, start, length, velocity, pan, bend] = f.as_slice() else { return None };
        let mut note = Note::new(
            key.parse().ok().filter(|k| *k <= 127)?,
            start.parse().ok()?,
            length.parse().ok()?,
            velocity.parse().ok()?,
        );
        note.pan = pan.parse().ok()?;
        if *bend != "-" {
            for p in bend.split(',') {
                let (at, st) = p.split_once(':')?;
                note.bend.push(Bend { at: at.parse().ok()?, semitones: st.parse().ok()? });
            }
        }
        out.push(note);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_go_through_text_and_back() {
        let mut a = Note::new(60, 0, 240, 0.8);
        a.pan = -0.5;
        a.bend = vec![Bend { at: 0, semitones: 0.0 }, Bend { at: 120, semitones: 1.5 }];
        let b = Note::new(64, 480, 960, 1.0);
        let text = to_text(&[a.clone(), b.clone()]);
        assert_eq!(from_text(&text), Some(vec![a, b]));
    }

    #[test]
    fn other_text_is_not_notes() {
        assert_eq!(from_text("hello"), None);
        assert_eq!(from_text("linuxmusic-notes/1\n200 0 1 1 0 -"), None);
    }
}
