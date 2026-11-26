//! The presets: data. The shipped ones are the files of `keymaps/<context>/<preset>.json` (embedded at build); the
//! user's own are files of the same shape in `<config>/keymaps/<context>/<name>.json`. A preset says only what it
//! changes: an action it does not list keeps the built-in default. A file is
//! `{ "context", "name", "keys": { action: [keys] }, "mouse": { action: [gestures] }, "unsure": { id: why } }`;
//! `unsure` marks what the author could not confirm against the program the preset follows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use lm_port::Value;
use lm_state::{from_json, to_json};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Preset {
    pub ctx: String,
    pub name: String,
    pub keys: BTreeMap<String, Vec<String>>,
    pub mouse: BTreeMap<String, Vec<String>>,
    pub unsure: BTreeMap<String, String>,
    /// The user's own (a file in the config folder), not a shipped one
    pub user: bool,
}

macro_rules! shipped {
    ($($ctx:literal / $name:literal),* $(,)?) => {
        &[$(($ctx, $name, include_str!(concat!("../../keymaps/", $ctx, "/", $name, ".json")))),*]
    };
}

/// (context, preset, text)
pub const SHIPPED: &[(&str, &str, &str)] = shipped![
    "Global" / "LMMS",
    "Global" / "REAPER",
    "Transport" / "LMMS",
    "Transport" / "REAPER",
    "Composition" / "LMMS",
    "Composition" / "REAPER",
    "Mixing" / "LMMS",
    "Mixing" / "REAPER",
    "PianoRoll" / "LMMS",
    "PianoRoll" / "REAPER",
    "Automation" / "LMMS",
    "Automation" / "REAPER",
    "Steps" / "LMMS",
    "Steps" / "REAPER",
    "Browser" / "LMMS",
    "Browser" / "REAPER",
    "Instrument" / "LMMS",
    "Instrument" / "REAPER",
    "Effect" / "LMMS",
    "Effect" / "REAPER",
];

fn list_of(v: &Value) -> Vec<String> {
    match v.as_text() {
        Some("") => Vec::new(),
        Some(t) => vec![t.to_string()],
        None => v.items().filter_map(Value::as_text).map(str::to_string).collect(),
    }
}

fn map_of(v: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    v.map(|m| m.entries().map(|(k, v)| (k.to_string(), list_of(v))).collect()).unwrap_or_default()
}

/// A preset from its file's text; `ctx` is the context of the folder it came from (the file's own `context` wins)
pub fn parse(ctx: &str, text: &str, user: bool) -> Result<Preset, String> {
    let v = from_json(text).map_err(|e| format!("{e:?}"))?;
    let name = v.get("name").and_then(Value::as_text).ok_or("no name")?.to_string();
    let ctx = v.get("context").and_then(Value::as_text).unwrap_or(ctx).to_string();
    let unsure = v
        .get("unsure")
        .map(|m| m.entries().filter_map(|(k, v)| v.as_text().map(|t| (k.to_string(), t.to_string()))).collect())
        .unwrap_or_default();
    Ok(Preset { ctx, name, keys: map_of(v.get("keys")), mouse: map_of(v.get("mouse")), unsure, user })
}

fn list_value(list: &[String]) -> Value {
    Value::list(list.iter().map(|t| Value::text(t.as_str())))
}

/// The text of a preset's file
pub fn to_text(p: &Preset) -> Result<String, String> {
    let section = |m: &BTreeMap<String, Vec<String>>| Value::object(m.iter().map(|(k, v)| (k.as_str(), list_value(v))));
    let mut v = Value::object([
        ("context", Value::text(p.ctx.as_str())),
        ("name", Value::text(p.name.as_str())),
        ("keys", section(&p.keys)),
        ("mouse", section(&p.mouse)),
    ]);
    if !p.unsure.is_empty() {
        v = v.with("unsure", Value::object(p.unsure.iter().map(|(k, t)| (k.as_str(), Value::text(t.as_str())))));
    }
    to_json(&v).map_err(|e| format!("{e:?}"))
}

static BUILTIN: LazyLock<Vec<Preset>> = LazyLock::new(|| {
    SHIPPED
        .iter()
        .map(|(ctx, name, text)| {
            // the files are part of the program: a broken one is a bug that the tests catch
            let mut p = parse(ctx, text, false)
                .unwrap_or_else(|e| Preset { name: format!("{name}: {e}"), ..Preset::default() });
            p.ctx = (*ctx).to_string();
            p
        })
        .collect()
});

/// The shipped presets
pub fn shipped() -> &'static [Preset] {
    &BUILTIN
}

/// The names a context offers: the shipped ones, then the user's
pub fn names(ctx: &str, user: &[Preset]) -> Vec<String> {
    shipped().iter().chain(user.iter()).filter(|p| p.ctx == ctx).map(|p| p.name.clone()).collect()
}

pub fn find<'a>(ctx: &str, name: &str, user: &'a [Preset]) -> Option<&'a Preset> {
    // `user` and the shipped list have different lifetimes: the shipped one is 'static, which outlives 'a
    shipped().iter().chain(user.iter()).find(|p| p.ctx == ctx && p.name == name)
}

// --- the user's files -------------------------------------------------------------------------------------------------

fn file_stem(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    s.trim().to_string()
}

pub fn user_folder(config: &Path, ctx: &str) -> PathBuf {
    config.join("keymaps").join(ctx)
}

/// Every preset of the user's folders. A file that does not read is left out
pub fn load_user(config: &Path) -> Vec<Preset> {
    let mut out = Vec::new();
    for c in super::CONTEXTS {
        let Ok(dir) = std::fs::read_dir(user_folder(config, c.id)) else { continue };
        let mut files: Vec<PathBuf> = dir
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        for f in files {
            if let Ok(text) = std::fs::read_to_string(&f)
                && let Ok(p) = parse(c.id, &text, true)
            {
                out.push(Preset { ctx: c.id.to_string(), ..p });
            }
        }
    }
    out
}

/// A name no preset of the context has yet ("Mine", "Mine 2", ...)
pub fn free_name(ctx: &str, wanted: &str, user: &[Preset]) -> String {
    let taken = names(ctx, user);
    let base = wanted.trim();
    let base = if base.is_empty() { "Mine" } else { base };
    if !taken.iter().any(|n| n == base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken.contains(n)).unwrap_or_default()
}

pub fn save_user(config: &Path, p: &Preset) -> Result<PathBuf, String> {
    let dir = user_folder(config, &p.ctx);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{}.json", file_stem(&p.name)));
    std::fs::write(&file, to_text(p)?).map_err(|e| e.to_string())?;
    Ok(file)
}

pub fn delete_user(config: &Path, p: &Preset) -> Result<(), String> {
    std::fs::remove_file(user_folder(config, &p.ctx).join(format!("{}.json", file_stem(&p.name))))
        .map_err(|e| e.to_string())
}

/// A preset file from anywhere (the import): its context from the file, else `ctx`
pub fn read_file(path: &Path, ctx: &str) -> Result<Preset, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(ctx, &text, true)
}

pub fn write_file(path: &Path, p: &Preset) -> Result<(), String> {
    std::fs::write(path, to_text(p)?).map_err(|e| e.to_string())
}
