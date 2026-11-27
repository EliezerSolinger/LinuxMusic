//! SFZ: the common opcodes of the text format. Headers (`<control>`, `<global>`, `<master>`, `<group>`, `<region>`)
//! nest their opcodes down to the region; `#define` variables, `#include`, `default_path` and note names are
//! resolved; the samples are read through a [`SampleLoader`].
//!
//! Supported: `sample`, `key`, `lokey`, `hikey`, `lovel`, `hivel`, `pitch_keycenter`, `pitch_keytrack`, `tune`,
//! `transpose`, `offset`, `end`, `loop_mode` (`no_loop`, `one_shot`, `loop_continuous`, `loop_sustain`),
//! `loop_start`, `loop_end`, `direction`, `volume`, `pan`, `amp_veltrack`, the `ampeg_*` envelope (`delay`, `attack`,
//! `hold`, `decay`, `sustain` in percent, `release`), `fil_type`, `cutoff`, `resonance`, `fil_keytrack`,
//! `fil_veltrack`, the `fileg_*` envelope and `fileg_depth`, `group`, `off_by`, `off_mode`, `trigger`, `lorand`,
//! `hirand`, `seq_length`, `seq_position`, `rt_decay` and `output`. The keyswitches, the controller conditions, the
//! LFOs and the effects are skipped.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lm_base::{Error, Result};

use crate::model::{Ahdsr, Filter, FilterKind, Instrument, LoopMode, OutputGroup, Sequence, Trigger, VelCurve, Zone};
use crate::pcm::{Pcm, SampleLoader};

type Opcodes = HashMap<String, String>;

/// A note name (`c4`, `f#3`, `eb2`; middle C is `c4` = 60) or a number
pub fn parse_key(text: &str) -> Option<u8> {
    let t = text.trim().to_ascii_lowercase();
    if let Ok(n) = t.parse::<i32>() {
        return (0..=127).contains(&n).then_some(n as u8);
    }
    let b = t.as_bytes();
    let base = match *b.first()? {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    let mut i = 1;
    let mut semis = base;
    match b.get(i) {
        Some(b'#') => {
            semis += 1;
            i += 1;
        }
        Some(b'b') => {
            semis -= 1;
            i += 1;
        }
        _ => {}
    }
    let octave: i32 = t.get(i..)?.parse().ok()?;
    let n = octave.checked_add(1)?.checked_mul(12)?.checked_add(semis)?;
    (0..=127).contains(&n).then_some(n as u8)
}

/// Splits the text into (header, opcodes) in order. `#define`s are applied, comments (`//`, `/* */`) dropped
/// and `#include` lines read through `include`
/// The most `#include`s one file may expand to in total, so a file that includes itself cannot multiply forever
const MAX_INCLUDES: usize = 64;

fn tokenize(
    text: &str,
    include: &mut dyn FnMut(&str) -> Option<String>,
    depth: usize,
    budget: &mut usize,
) -> Vec<(String, Opcodes)> {
    let mut defines: BTreeMap<String, String> = BTreeMap::new();
    let mut out: Vec<(String, Opcodes)> = Vec::new();
    let text = strip_comments(text);
    let mut current: Option<(String, Opcodes)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#define") {
            let mut it = rest.split_whitespace();
            if let (Some(name), Some(value)) = (it.next(), it.next()) {
                defines.insert(name.to_owned(), value.to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("#include") {
            if depth < 8 && *budget > 0 {
                *budget -= 1;
                let name = rest.trim().trim_matches('"');
                if let Some(inner) = include(name) {
                    let sub = tokenize(&inner, include, depth + 1, budget);
                    out.extend(current.take());
                    out.extend(sub);
                }
            }
            continue;
        }
        let mut line = line.to_owned();
        for (k, v) in &defines {
            line = line.replace(k.as_str(), v);
        }
        // Split on whitespace; a token with `=` opens an opcode, the others continue the last value
        let mut rest = line.as_str();
        while !rest.is_empty() {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            if rest.starts_with('<') {
                if let Some(end) = rest.find('>') {
                    out.extend(current.take());
                    current = Some((rest[1..end].to_ascii_lowercase(), Opcodes::new()));
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            // An opcode: name=value, the value up to the next ` name=` or header
            if let Some(eq) = rest.find('=') {
                let name = rest[..eq].trim();
                if !name.contains(char::is_whitespace) && !name.is_empty() {
                    let after = &rest[eq + 1..];
                    let end = next_opcode_start(after);
                    let value = &after[..end];
                    if let Some((_, ops)) = current.as_mut() {
                        ops.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                    }
                    rest = &after[end..];
                    continue;
                }
            }
            // Something else on the line: skipped up to the next space
            rest = rest.find(char::is_whitespace).map_or("", |i| &rest[i..]);
        }
    }
    out.extend(current.take());
    out
}

/// Where the next opcode (or header) of `s` starts: after a space, a word of `[a-z0-9_$]` and `=`
fn next_opcode_start(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            return i;
        }
        if b[i] == b' ' || b[i] == b'\t' {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'$') {
                j += 1;
            }
            if j > i + 1 && b.get(j) == Some(&b'=') {
                return i;
            }
        }
        i += 1;
    }
    b.len()
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_block = false;
    for line in text.lines() {
        let mut l = line;
        let mut kept = String::new();
        loop {
            if in_block {
                match l.find("*/") {
                    Some(i) => {
                        l = &l[i + 2..];
                        in_block = false;
                    }
                    None => break,
                }
            } else {
                let line_comment = l.find("//");
                let block = l.find("/*");
                match (line_comment, block) {
                    (Some(a), Some(b)) if b < a => {
                        kept.push_str(&l[..b]);
                        l = &l[b + 2..];
                        in_block = true;
                    }
                    (Some(a), _) => {
                        kept.push_str(&l[..a]);
                        break;
                    }
                    (None, Some(b)) => {
                        kept.push_str(&l[..b]);
                        l = &l[b + 2..];
                        in_block = true;
                    }
                    (None, None) => {
                        kept.push_str(l);
                        break;
                    }
                }
            }
        }
        out.push_str(&kept);
        out.push('\n');
    }
    out
}

fn num(ops: &Opcodes, key: &str) -> Option<f32> {
    ops.get(key)?.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

fn int(ops: &Opcodes, key: &str) -> Option<i64> {
    let v = ops.get(key)?.trim();
    v.parse::<i64>().ok().or_else(|| v.parse::<f64>().ok().map(|f| f as i64))
}

/// What reads an SFZ: its text, its folder (for the samples and `#include`) and the loader of the samples
pub fn import(text: &str, folder: &Path, loader: &mut dyn SampleLoader) -> Result<Instrument> {
    let mut include = |name: &str| std::fs::read_to_string(folder.join(name.replace('\\', "/"))).ok();
    let tokens = tokenize(text, &mut include, 0, &mut MAX_INCLUDES.clone());
    let mut control: Opcodes = Opcodes::new();
    let mut global: Opcodes = Opcodes::new();
    let mut master: Opcodes = Opcodes::new();
    let mut group: Opcodes = Opcodes::new();
    let mut regions: Vec<Opcodes> = Vec::new();
    for (header, ops) in tokens {
        match header.as_str() {
            "control" => control.extend(ops),
            "global" => {
                global = ops;
                master.clear();
                group.clear();
            }
            "master" => {
                master = ops;
                group.clear();
            }
            "group" => group = ops,
            "region" => {
                let mut r = Opcodes::new();
                for layer in [&global, &master, &group] {
                    r.extend(layer.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
                r.extend(ops);
                regions.push(r);
            }
            _ => {}
        }
    }
    let default_path = control.get("default_path").cloned().unwrap_or_default().replace('\\', "/");
    let mut out = Instrument::new("sfz");
    let mut cache: HashMap<PathBuf, Arc<Pcm>> = HashMap::new();
    let mut max_output = 0u8;
    for r in &regions {
        let Some(sample) = r.get("sample") else {
            // (a generator region, `sample=*sine`: not a sample)
            continue;
        };
        if sample.starts_with('*') {
            continue;
        }
        let path = folder.join(&default_path).join(sample.replace('\\', "/"));
        let pcm = match cache.get(&path) {
            Some(p) => Arc::clone(p),
            None => {
                let p = Arc::new(loader.load(&path)?);
                if p.frames() == 0 {
                    return Err(Error::new("sampler.sfz.empty_sample").arg("path", path.display().to_string()));
                }
                cache.insert(path.clone(), Arc::clone(&p));
                p
            }
        };
        let zone = region(r, pcm);
        max_output = max_output.max(zone.output);
        out.zones.push(zone);
    }
    if out.zones.is_empty() {
        return Err(Error::new("sampler.sfz.empty"));
    }
    if max_output > 0 {
        out.outputs = (0..=max_output).map(|i| OutputGroup { name: format!("out{}", i + 1), stereo: true }).collect();
    }
    out.validate()?;
    Ok(out)
}

fn region(r: &Opcodes, pcm: Arc<Pcm>) -> Zone {
    let mut z = Zone::new(pcm);
    let key = |k: &str| r.get(k).and_then(|v| parse_key(v));
    if let Some(k) = key("key") {
        z.keys = (k, k);
        z.root_key = k;
    }
    if let Some(k) = key("lokey") {
        z.keys.0 = k;
    }
    if let Some(k) = key("hikey") {
        z.keys.1 = k;
    }
    if let Some(k) = key("pitch_keycenter") {
        z.root_key = k;
    }
    if let Some(v) = int(r, "lovel") {
        z.vels.0 = v.clamp(0, 127) as u8;
    }
    if let Some(v) = int(r, "hivel") {
        z.vels.1 = v.clamp(0, 127) as u8;
    }
    z.vels.0 = z.vels.0.max(1);
    z.vels.1 = z.vels.1.max(z.vels.0);
    if let Some(v) = num(r, "pitch_keytrack") {
        z.key_track = v;
    }
    z.tune = num(r, "tune").or_else(|| num(r, "cents")).unwrap_or(0.0) + num(r, "transpose").unwrap_or(0.0) * 100.0;
    let frames = z.sample.frames() as i64;
    z.start = int(r, "offset").unwrap_or(0).clamp(0, frames - 1) as u32;
    // `end` is the last frame played (inclusive); -1 silences the region in the format: not supported, plays whole
    z.end = int(r, "end").filter(|e| *e >= 0).map_or(0, |e| e.saturating_add(1).clamp(1, frames) as u32);
    z.loop_start = int(r, "loop_start").unwrap_or(0).clamp(0, frames) as u32;
    // `loop_end` is the last frame of the loop (inclusive)
    z.loop_end = int(r, "loop_end").map_or(0, |e| e.saturating_add(1).clamp(0, frames) as u32);
    z.loop_mode = match r.get("loop_mode").or_else(|| r.get("loopmode")).map(String::as_str) {
        Some("no_loop") => LoopMode::Off,
        Some("one_shot") => LoopMode::OneShot,
        Some("loop_continuous") => LoopMode::Forward,
        Some("loop_sustain") => LoopMode::UntilRelease,
        // (without the opcode a sample whose file has loop points loops in the format; only the opcodes count here)
        _ => LoopMode::Off,
    };
    z.reverse = r.get("direction").is_some_and(|d| d == "reverse");
    z.gain = num(r, "volume").unwrap_or(0.0);
    z.pan = (num(r, "pan").unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0);
    z.vel_track = (num(r, "amp_veltrack").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0);
    z.vel_curve = VelCurve::Concave;
    z.amp = Ahdsr {
        delay: num(r, "ampeg_delay").unwrap_or(0.0).max(0.0),
        attack: num(r, "ampeg_attack").unwrap_or(0.0).max(0.0),
        hold: num(r, "ampeg_hold").unwrap_or(0.0).max(0.0),
        decay: num(r, "ampeg_decay").unwrap_or(0.0).max(0.0),
        sustain: (num(r, "ampeg_sustain").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
        release: num(r, "ampeg_release").unwrap_or(0.001).max(0.002),
    };
    if let Some(kind) = r.get("fil_type").or_else(|| r.get("filtype")) {
        let kind = match kind.as_str() {
            "hpf_1p" | "hpf_2p" | "hpf_4p" | "hpf_6p" => Some(FilterKind::HighPass),
            "bpf_1p" | "bpf_2p" => Some(FilterKind::BandPass),
            "lpf_1p" | "lpf_2p" | "lpf_4p" | "lpf_6p" => Some(FilterKind::LowPass),
            _ => None,
        };
        if let (Some(kind), Some(cutoff)) = (kind, num(r, "cutoff")) {
            z.filter = Some(Filter {
                kind,
                cutoff: cutoff.max(1.0),
                resonance: num(r, "resonance").unwrap_or(0.0),
                key_track: num(r, "fil_keytrack").unwrap_or(0.0),
                vel_cents: -num(r, "fil_veltrack").unwrap_or(0.0),
                env: Ahdsr {
                    delay: num(r, "fileg_delay").unwrap_or(0.0).max(0.0),
                    attack: num(r, "fileg_attack").unwrap_or(0.0).max(0.0),
                    hold: num(r, "fileg_hold").unwrap_or(0.0).max(0.0),
                    decay: num(r, "fileg_decay").unwrap_or(0.0).max(0.0),
                    sustain: (num(r, "fileg_sustain").unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
                    release: num(r, "fileg_release").unwrap_or(0.0).max(0.0),
                },
                env_depth: num(r, "fileg_depth").unwrap_or(0.0),
            });
        }
    } else if let Some(cutoff) = num(r, "cutoff") {
        z.filter = Some(Filter {
            cutoff: cutoff.max(1.0),
            resonance: num(r, "resonance").unwrap_or(0.0),
            key_track: num(r, "fil_keytrack").unwrap_or(0.0),
            vel_cents: -num(r, "fil_veltrack").unwrap_or(0.0),
            env_depth: num(r, "fileg_depth").unwrap_or(0.0),
            ..Filter::default()
        });
    }
    z.group = int(r, "group").unwrap_or(0).max(0) as u32;
    z.off_by = int(r, "off_by").unwrap_or(0).max(0) as u32;
    z.off_fast = r.get("off_mode").is_some_and(|m| m == "fast");
    z.trigger = match r.get("trigger").map(String::as_str) {
        Some("release") => Trigger::Release,
        Some("first") => Trigger::First,
        Some("legato") => Trigger::Legato,
        _ => Trigger::Attack,
    };
    z.release_decay = num(r, "rt_decay").unwrap_or(0.0).max(0.0);
    if let (Some(lo), Some(hi)) = (num(r, "lorand"), num(r, "hirand")) {
        z.random = Some((lo, hi));
    } else if let Some(hi) = num(r, "hirand") {
        z.random = Some((0.0, hi));
    } else if let Some(lo) = num(r, "lorand") {
        z.random = Some((lo, 1.0));
    }
    if let (Some(len), Some(pos)) = (int(r, "seq_length"), int(r, "seq_position")) {
        // One counter per key range: regions of the same keys share a turn (the group is the lowest key)
        z.sequence = Some(Sequence {
            group: u16::from(z.keys.0),
            length: len.clamp(1, 1000) as u16,
            position: pos.clamp(1, 1000) as u16,
        });
    }
    z.output = int(r, "output").unwrap_or(0).clamp(0, 31) as u8;
    z
}

#[cfg(test)]
mod tests;
