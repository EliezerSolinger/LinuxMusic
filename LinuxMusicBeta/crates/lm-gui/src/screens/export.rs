//! Owner: agent D. Exporting audio, as the legacy's export wizard (F-PRJ-022): four pages with Back, Next and Export —
//! what to export, the formats, the processing, where — in the same order and with the same choices, plus what the
//! new model needs (recorded in decisoes-autonomas.md):
//! - a range (the loop's, or bars typed) is "Fast" (default, 10 s of warm-up) or "Exact", with what each renders and
//!   costs (U37, V18); the whole song is always exact;
//! - one track with the others muted (F-PRJ-023), and stems per track or per desk channel on the same page (P-40),
//!   instead of the legacy's separate "Export tracks" dialog;
//! - on the last page, the third-party plugins that do not promise the same result twice (H18);
//! - after Export, the progress with cancel (the shared loading component) and the report, in the same window.
//!
//! The export is the session's operation `project.export`, with the request as data (`lm_port::trees`).

use gpui::{AnyElement, Context, Entity, FontWeight, Render, SharedString, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};
use lm_port::{NodeId, Target, Value};

use super::ScreenCx;
use super::settings::{
    form_row, khz, modal_size, note, obj, port_action, real, segmented, stepper, switch, target_loading,
};
use crate::model;
use crate::pages::marks::MarksState;
use crate::ui::{THEME, button, c, caption, tbutton, tr, trf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    Song,
    Loop,
    Markers,
    Bars,
    Track,
    /// a range given by the ruler ("Export the region as audio"), in ticks
    Ticks(i64, i64),
}

/// A range the ruler asks the export window to open on (a region's ticks), taken as the window opens
pub struct ExportRange(pub Option<(i64, i64)>);
impl gpui::Global for ExportRange {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stems {
    None,
    Tracks,
    Channels,
}

const PAGES: [&str; 4] = ["What to export", "Formats", "Processing", "Where"];
const RATES: [i64; 4] = [44_100, 48_000, 88_200, 96_000];
const KBPS: [i64; 4] = [128, 192, 256, 320];
/// The fake renderer's speed, in times real time, for the estimate (the session will say its own)
const RENDER_SPEED: f64 = 40.0;

pub struct Export {
    scx: ScreenCx,
    page: usize,
    what: What,
    bars: (i64, i64),
    fast: bool,
    warmup: f64,
    as_loop: bool,
    repeats: i64,
    track: usize,
    /// "Between markers": the span chosen (marker i to the next)
    marker: usize,
    stems: Stems,
    chosen: Vec<NodeId>,
    wav: Option<i64>,
    flac: Option<i64>,
    ogg: Option<i64>,
    mp3: Option<i64>,
    rate: i64,
    normalize: &'static str,
    target: f64,
    fade_in: f64,
    fade_out: f64,
    dither: bool,
    folder: Entity<InputState>,
    name: Entity<InputState>,
    started: bool,
    _subs: Vec<Subscription>,
}

impl Export {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let status = scx.core.read(cx).port().status().clone();
        let project = status.get("project").cloned().unwrap_or(Value::NULL);
        let pfolder = project.get("folder").and_then(Value::as_text).unwrap_or("").to_string();
        let folder_v = if pfolder.is_empty() { String::new() } else { format!("{pfolder}/exports") };
        let name_v =
            project.get("name").and_then(Value::as_text).map_or_else(|| tr("untitled").to_string(), str::to_string);
        let folder = cx.new(|cx| InputState::new(window, cx).default_value(folder_v));
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name_v));
        let subs = vec![
            cx.subscribe(&folder, |_: &mut Export, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&name, |_: &mut Export, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        let root = scx.core.read(cx).port().root().clone();
        let (lb, le) = loop_bars(&root);
        Export {
            scx,
            page: 0,
            what: match take_range(cx) {
                Some((a, z)) => What::Ticks(a, z),
                None => What::Song,
            },
            bars: (lb, le),
            fast: true,
            warmup: 10.0,
            as_loop: false,
            repeats: 1,
            track: 0,
            marker: 0,
            stems: Stems::None,
            chosen: Vec::new(),
            wav: Some(24),
            flac: None,
            ogg: None,
            mp3: None,
            rate: model::sample_rate(&root) as i64,
            normalize: "none",
            target: -1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            dither: true,
            folder,
            name,
            started: false,
            _subs: subs,
        }
    }

    fn formats(&self) -> Vec<&'static str> {
        [
            ("wav", self.wav.is_some()),
            ("flac", self.flac.is_some()),
            ("ogg", self.ogg.is_some()),
            ("mp3", self.mp3.is_some()),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(f, _)| f)
        .collect()
    }

    /// The range in ticks (start inclusive, end exclusive), or None for the whole song
    fn range(&self, root: &Value) -> Option<(i64, i64)> {
        let map = model::tempo_map(root);
        let bar = |b: i64| map.bar_start(b).unwrap_or(0);
        match self.what {
            What::Song | What::Track => None,
            What::Loop => Some(model::song(root).loop_range).filter(|(a, z)| z > a),
            What::Bars => Some((bar(self.bars.0), bar(self.bars.1))),
            What::Markers => {
                let end = map.bar_start(song_end(root).0).unwrap_or(0);
                MarksState::between_markers(root, end).get(self.marker).map(|(_, a, z)| (*a, *z))
            }
            What::Ticks(a, z) => Some((a, z)),
        }
    }

    /// The request, as the session's `project.export` reads it (lm_port::trees)
    fn request(&self, cx: &Context<Self>) -> Value {
        let root = self.scx.core.read(cx).port().root().clone();
        let tracks = model::tracks(&root);
        let range = match self.range(&root) {
            None => Value::NULL,
            Some((a, b)) => obj(vec![
                ("from", Value::int(a)),
                ("to", Value::int(b)),
                ("fast", Value::bool(self.fast)),
                ("warmup", real(self.warmup)),
            ]),
        };
        let stems = match self.stems {
            Stems::None => Value::NULL,
            s => obj(vec![
                ("by", Value::text(if s == Stems::Tracks { "tracks" } else { "channels" })),
                ("nodes", Value::list(self.chosen.iter().map(|n| Value::text(n.to_string())))),
            ]),
        };
        let solo =
            if self.what == What::Track { tracks.get(self.track).map(|t| Value::text(t.id.to_string())) } else { None };
        let depth = self.wav.or(self.flac).unwrap_or(24);
        obj(vec![
            ("folder", Value::text(self.folder.read(cx).value().trim().to_string())),
            ("name", Value::text(self.name.read(cx).value().trim().to_string())),
            ("formats", Value::list(self.formats().into_iter().map(Value::text))),
            ("rate", Value::int(self.rate)),
            ("depth", Value::int(depth)),
            ("flacDepth", Value::int(self.flac.unwrap_or(24))),
            ("dither", Value::bool(self.dither)),
            ("oggQuality", Value::int(self.ogg.unwrap_or(6))),
            ("mp3Kbps", Value::int(self.mp3.unwrap_or(320))),
            ("normalize", obj(vec![("kind", Value::text(self.normalize)), ("target", real(self.target))])),
            ("fadeIn", real(self.fade_in)),
            ("fadeOut", real(self.fade_out)),
            ("range", range),
            ("asLoop", Value::bool(self.as_loop)),
            ("repeats", Value::int(self.repeats)),
            ("onlyTrack", solo.unwrap_or(Value::NULL)),
            ("stems", stems),
        ])
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let req = self.request(cx);
        port_action(&self.scx, "project.export", req, cx);
        self.started = true;
        cx.notify();
    }
}

/// The range the ruler asked for, taken once
fn take_range(cx: &mut gpui::App) -> Option<(i64, i64)> {
    cx.try_global::<ExportRange>()?;
    cx.update_global::<ExportRange, _>(|r, _| r.0.take())
}

/// The loop's bars (start inclusive, end exclusive), from the tree's transport
fn loop_bars(root: &Value) -> (i64, i64) {
    let map = model::tempo_map(root);
    let (a, b) = model::song(root).loop_range;
    let start = map.bar_beat_tick(a).bar;
    let end = map.bar_beat_tick(b).bar;
    (start.max(1), end.max(start + 1))
}

/// The song's length: the end of the last item, in bars and seconds
fn song_end(root: &Value) -> (i64, f64) {
    let map = model::tempo_map(root);
    let end = model::tracks(root).iter().flat_map(|t| t.items.iter()).map(|i| i.start + i.length).max().unwrap_or(0);
    let bar = map.bar_beat_tick(end).bar + 1;
    let secs = map.bar_start(bar).map(|t| map.sample_at(t) as f64 / model::sample_rate(root) as f64).unwrap_or(0.0);
    (bar, secs)
}

fn tick_seconds(root: &Value, tick: i64) -> f64 {
    let map = model::tempo_map(root);
    map.sample_at(tick) as f64 / model::sample_rate(root) as f64
}

/// A plugin's name: its translation key, shown as the key when the table has no line for it
fn tr_dyn(key: &str) -> String {
    key.to_string()
}

fn mmss(s: f64) -> String {
    let s = s.max(0.0);
    if s < 60.0 { format!("{s:.1} s") } else { format!("{}:{:02}", (s / 60.0) as u32, (s % 60.0) as u32) }
}

impl Export {
    fn page_what(&mut self, root: &Value, cx: &mut Context<Self>) -> AnyElement {
        let (end_bar, song_secs) = song_end(root);
        let what = self.what;
        let items = [
            (What::Song, "The whole song"),
            (What::Loop, "The loop range"),
            (What::Markers, "Between markers"),
            (What::Bars, "Bars"),
            (What::Track, "One track"),
        ];
        let mut col = div().flex().flex_col().gap(px(8.)).child(segmented(
            "what",
            items.iter().map(|(k, l)| (tr(l).into(), *k == what, true)).collect(),
            cx,
            move |e, i, _, cx| {
                e.what = items[i].0;
                cx.notify();
            },
        ));
        match what {
            What::Song => {
                col = col.child(caption(trf("Bars 1 to {0}, {1}; always exact", &[&(end_bar - 1), &mmss(song_secs)])))
            }
            What::Loop => {
                let (a, b) = loop_bars(root);
                col = col.child(caption(trf("Bars {0} to {1} (the loop on the ruler)", &[&a, &(b - 1)])));
            }
            What::Markers => {
                let map = model::tempo_map(root);
                let end = map.bar_start(end_bar).unwrap_or(0);
                let spans = MarksState::between_markers(root, end);
                if spans.is_empty() {
                    col = col.child(caption(tr("The project has no markers")));
                }
                col = col.child(div().flex().flex_wrap().gap(px(3.)).children(spans.into_iter().enumerate().map(
                    |(i, (name, a, z))| {
                        let label = format!("{name} ({}–{})", map.bar_beat_tick(a).bar, map.bar_beat_tick(z).bar - 1);
                        button(("marker-span", i), label, i == self.marker, THEME.green)
                            .debug_selector(move || format!("marker-span-{i}"))
                            .on_click(cx.listener(move |e, _, _, cx| {
                                e.marker = i;
                                cx.notify();
                            }))
                    },
                )));
            }
            What::Ticks(a, z) => {
                let map = model::tempo_map(root);
                col = col.child(caption(trf(
                    "The region: bars {0} to {1}",
                    &[&map.bar_beat_tick(a).bar, &map.bar_beat_tick(z).bar],
                )));
            }
            What::Bars => {
                let (a, b) = self.bars;
                col = col.child(form_row(
                    tr("Bars"),
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(stepper("bar-from", trf("from {0}", &[&a]), cx, move |e, d, cx| {
                            e.bars.0 = (a + d).clamp(1, b - 1);
                            cx.notify();
                        }))
                        .child(stepper("bar-to", trf("to {0}", &[&(b - 1)]), cx, move |e, d, cx| {
                            e.bars.1 = (b + d).max(a + 1);
                            cx.notify();
                        })),
                ));
            }
            What::Track => {
                let tracks = model::tracks(root);
                col = col.child(caption(tr("The whole song with the other tracks muted"))).child(
                    div().id("track-list").max_h(px(110.)).overflow_y_scroll().flex().flex_wrap().gap(px(3.)).children(
                        tracks.iter().enumerate().filter(|(_, t)| !t.folder).map(|(i, t)| {
                            button(("only-track", i), t.name.to_string(), i == self.track, THEME.green).on_click(
                                cx.listener(move |e, _, _, cx| {
                                    e.track = i;
                                    cx.notify();
                                }),
                            )
                        }),
                    ),
                );
            }
        }
        // a range: Fast or Exact, each with what it renders and costs (U37)
        if let Some((a, b)) = self.range(root) {
            let len = tick_seconds(root, b) - tick_seconds(root, a);
            let before = tick_seconds(root, a);
            let fast_s = len + self.warmup.min(before);
            let exact_s = tick_seconds(root, b);
            let warm = self.warmup;
            col = col
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(segmented(
                            "mode",
                            vec![(tr("Fast").into(), self.fast, true), (tr("Exact").into(), !self.fast, true)],
                            cx,
                            |e, i, _, cx| {
                                e.fast = i == 0;
                                cx.notify();
                            },
                        ))
                        .when(self.fast, |d| {
                            d.child(stepper("warmup", trf("warm-up {0} s", &[&warm]), cx, move |e, dd, cx| {
                                e.warmup = (warm + dd as f64).clamp(0.0, 60.0);
                                cx.notify();
                            }))
                        }),
                )
                .child(caption(if self.fast {
                    trf(
                        "Starts {0} s before and drops them: renders {1}, about {2} of work. Almost identical, and the report says so",
                        &[&warm, &mmss(fast_s), &mmss(fast_s / RENDER_SPEED)],
                    )
                } else {
                    trf(
                        "Renders from the start of the song and drops what comes before: renders {0}, about {1} of work. Identical, bit for bit, to the same range in the whole song",
                        &[&mmss(exact_s), &mmss(exact_s / RENDER_SPEED)],
                    )
                }));
        }
        let repeats = self.repeats;
        col = col
            .child(switch("as-loop", self.as_loop, cx, |e: &mut Export, v, cx| {
                e.as_loop = v;
                cx.notify();
            }))
            .child(caption(tr("As a loop: without the tail at the end (reverbs, releases)")))
            .child(form_row(
                tr("Times"),
                stepper("repeats", repeats.to_string(), cx, move |e, d, cx| {
                    e.repeats = (repeats + d).clamp(1, 64);
                    cx.notify();
                }),
            ));
        // stems (P-40): per track or per desk channel
        let stems = self.stems;
        let list: Vec<(NodeId, String)> = match stems {
            Stems::Tracks => {
                model::tracks(root).into_iter().filter(|t| !t.folder).map(|t| (t.id, t.name.to_string())).collect()
            }
            Stems::Channels => model::channels(root).into_iter().map(|c| (c.id, c.name.to_string())).collect(),
            Stems::None => Vec::new(),
        };
        col.child(form_row(
            tr("Stems"),
            segmented(
                "stems",
                vec![
                    (tr("None").into(), stems == Stems::None, true),
                    (tr("One file per track").into(), stems == Stems::Tracks, true),
                    (tr("One file per desk channel").into(), stems == Stems::Channels, true),
                ],
                cx,
                |e, i, _, cx| {
                    e.stems = [Stems::None, Stems::Tracks, Stems::Channels][i];
                    e.chosen.clear();
                    cx.notify();
                },
            ),
        ))
        .when(stems != Stems::None, |d| {
            let all: Vec<NodeId> = list.iter().map(|(id, _)| *id).collect();
            d.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(button("stems-all", tr("All"), false, 0).on_click(cx.listener(move |e, _, _, cx| {
                        e.chosen = all.clone();
                        cx.notify();
                    })))
                    .child(caption(trf("{0} chosen", &[&self.chosen.len()]))),
            )
            .child(
                div().id("stem-list").max_h(px(110.)).overflow_y_scroll().flex().flex_wrap().gap(px(3.)).children(
                    list.iter().enumerate().map(|(i, (id, n))| {
                        let id = *id;
                        button(("stem", i), n.clone(), self.chosen.contains(&id), THEME.green)
                            .debug_selector(move || format!("stem-{i}"))
                            .on_click(cx.listener(move |e, _, _, cx| {
                                if let Some(k) = e.chosen.iter().position(|x| *x == id) {
                                    e.chosen.remove(k);
                                } else {
                                    e.chosen.push(id);
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
        })
        .into_any_element()
    }

    fn page_formats(&mut self, root: &Value, cx: &mut Context<Self>) -> AnyElement {
        let project_rate = model::sample_rate(root) as i64;
        let bits =
            |id: &'static str, cur: Option<i64>, float: bool, cx: &mut Context<Self>, set: fn(&mut Export, i64)| {
                let mut items: Vec<(SharedString, bool, bool)> = vec![
                    (tr("16 bits").into(), cur == Some(16), cur.is_some()),
                    (tr("24 bits").into(), cur == Some(24), cur.is_some()),
                ];
                if float {
                    items.push((tr("32 bits float").into(), cur == Some(32), cur.is_some()));
                }
                segmented(id, items, cx, move |e, i, _, cx| {
                    set(e, [16, 24, 32][i]);
                    cx.notify();
                })
            };
        let check =
            |id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>, toggle: fn(&mut Export, bool)| {
                div()
                    .w(px(150.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(switch(id, on, cx, move |e: &mut Export, v, cx| {
                        toggle(e, v);
                        cx.notify();
                    }))
                    .child(div().text_size(px(12.)).child(label))
            };
        let (wav, flac, ogg, mp3) = (self.wav, self.flac, self.ogg, self.mp3);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(tr("Several at once, each a file in the folder chosen at the end.")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(check("fmt-wav", "WAV", wav.is_some(), cx, |e, v| e.wav = v.then_some(24)))
                    .child(bits("wav-bits", wav, true, cx, |e, b| e.wav = Some(b))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(check("fmt-flac", "FLAC", flac.is_some(), cx, |e, v| e.flac = v.then_some(24)))
                    .child(bits("flac-bits", flac, false, cx, |e, b| e.flac = Some(b))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(check("fmt-ogg", "OGG Vorbis", ogg.is_some(), cx, |e, v| e.ogg = v.then_some(6)))
                    .when_some(ogg, |d, q| {
                        d.child(stepper("ogg-quality", trf("quality {0}", &[&q]), cx, move |e, dd, cx| {
                            e.ogg = Some((q + dd).clamp(0, 10));
                            cx.notify();
                        }))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(check("fmt-mp3", "MP3", mp3.is_some(), cx, |e, v| e.mp3 = v.then_some(320)))
                    .child(segmented(
                        "mp3-kbps",
                        KBPS.iter().map(|k| (format!("{k} kbps").into(), mp3 == Some(*k), mp3.is_some())).collect(),
                        cx,
                        |e, i, _, cx| {
                            e.mp3 = Some(KBPS[i]);
                            cx.notify();
                        },
                    )),
            )
            .child(form_row(
                tr("Sample rate"),
                segmented(
                    "exp-rate",
                    RATES
                        .iter()
                        .map(|r| {
                            let label: SharedString = if *r == project_rate {
                                trf("{0} (project)", &[&khz(*r)]).into()
                            } else {
                                khz(*r).into()
                            };
                            (label, self.rate == *r, true)
                        })
                        .collect(),
                    cx,
                    |e, i, _, cx| {
                        e.rate = RATES[i];
                        cx.notify();
                    },
                ),
            ))
            .when(self.formats().is_empty(), |d| d.child(note(tr("Choose at least one format."), THEME.orange)))
            .into_any_element()
    }

    fn page_processing(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (target, fi, fo, kind) = (self.target, self.fade_in, self.fade_out, self.normalize);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(caption(tr("Applied to every format.")))
            .child(form_row(
                tr("Normalize"),
                segmented(
                    "normalize",
                    vec![
                        (tr("None").into(), kind == "none", true),
                        (tr("Peak (dBFS)").into(), kind == "peak", true),
                        (tr("Loudness (LUFS, ITU-R BS.1770)").into(), kind == "lufs", true),
                    ],
                    cx,
                    |e, i, _, cx| {
                        e.normalize = ["none", "peak", "lufs"][i];
                        // the usual targets: -1 dBFS for a peak, -14 LUFS for streaming
                        e.target = if i == 2 { -14.0 } else { -1.0 };
                        cx.notify();
                    },
                ),
            ))
            .when(kind != "none", |d| {
                d.child(form_row(
                    tr("Target"),
                    stepper("norm-target", format!("{target:.1}"), cx, move |e, dd, cx| {
                        e.target = (target + dd as f64).clamp(-60.0, 0.0);
                        cx.notify();
                    }),
                ))
            })
            .child(form_row(
                tr("Fade in"),
                stepper("fade-in", format!("{fi:.2} s"), cx, move |e, d, cx| {
                    e.fade_in = (fi + d as f64 * 0.5).clamp(0.0, 30.0);
                    cx.notify();
                }),
            ))
            .child(form_row(
                tr("Fade out"),
                stepper("fade-out", format!("{fo:.2} s"), cx, move |e, d, cx| {
                    e.fade_out = (fo + d as f64 * 0.5).clamp(0.0, 30.0);
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(switch("dither", self.dither, cx, |e: &mut Export, v, cx| {
                        e.dither = v;
                        cx.notify();
                    }))
                    .child(div().text_size(px(12.)).child(tr("Dither (TPDF) to 16 and 24 bits"))),
            )
            .into_any_element()
    }

    fn page_where(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // H18: the third-party plugins the project uses do not promise the same result twice
        let port = self.scx.core.read(cx).port();
        let outside: Vec<(String, String)> =
            port.plugins().into_iter().filter(|p| p.third_party).map(|p| (p.id.to_string(), tr_dyn(&p.name))).collect();
        let mut third: Vec<String> = Vec::new();
        port.root().walk(&mut |_, v| {
            if let Some(id) = v.get("plugin").and_then(Value::as_text)
                && let Some((_, name)) = outside.iter().find(|(pid, _)| pid == id)
                && !third.contains(name)
            {
                third.push(name.clone());
            }
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(form_row(tr("Folder"), Input::new(&self.folder)))
            .child(form_row(tr("Name"), Input::new(&self.name)))
            .when(!third.is_empty(), |d| {
                d.child(note(
                    trf("These third-party plugins do not promise the same result twice, so two exports may differ: {0}", &[&third.join(", ")]),
                    THEME.orange,
                ))
            })
            .into_any_element()
    }

    /// After Export: the progress (the shared loading component), then the report
    fn progress(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let loading = target_loading(&self.scx, "export-loading", Target::Export(1), cx);
        let report = self.scx.core.read(cx).port().status().get("lastExport").cloned().unwrap_or(Value::NULL);
        if let Some(l) = loading {
            return div().h(px(30.)).child(l).into_any_element();
        }
        if report.is_null() {
            return caption(tr("Starting…")).into_any_element();
        }
        let ok = report.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let file = report.get("file").and_then(Value::as_text).unwrap_or("").to_string();
        if !ok {
            let err = report.get("error").and_then(Value::as_text).unwrap_or("The operation failed");
            const DISK: &str = "The export failed: there is not enough space on the disk";
            let text = if err == DISK { tr(DISK).to_string() } else { trf("The export failed: {0}", &[&err]) };
            return note(text, THEME.red).into_any_element();
        }
        let identical = report.get("identical").and_then(Value::as_bool).unwrap_or(true);
        let peak = report.get("peakDb").and_then(Value::as_real).unwrap_or(0.0);
        let lufs = report.get("lufs").and_then(Value::as_real).unwrap_or(0.0);
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(note(trf("Exported: {0}", &[&file]), THEME.green))
            .child(caption(trf("Peak {0} dBFS, loudness {1} LUFS", &[&format!("{peak:.1}"), &format!("{lufs:.1}")])))
            .when(!identical, |d| {
                d.child(caption(tr(
                    "Fast range: almost identical to the same range in the whole song, not bit for bit",
                )))
            })
            .into_any_element()
    }
}

impl Render for Export {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 640.0, 520.0);
        let root = self.scx.core.read(cx).port().root().clone();
        let body = if self.started {
            self.progress(cx)
        } else {
            match self.page {
                0 => self.page_what(&root, cx),
                1 => self.page_formats(&root, cx),
                2 => self.page_processing(cx),
                _ => self.page_where(cx),
            }
        };
        let last = self.page == PAGES.len() - 1;
        let can_export = !self.formats().is_empty() && !self.name.read(cx).value().trim().is_empty();
        let page = self.page;
        div()
            .id("export")
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .px(px(18.))
                    .py(px(12.))
                    .bg(c(THEME.bg1))
                    .border_b_1()
                    .border_color(c(THEME.line))
                    .child(div().text_size(px(12.)).text_color(c(THEME.text3)).child(tr("Export the song")))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::BOLD).child(if self.started {
                        tr("Exporting")
                    } else {
                        tr(PAGES[page])
                    })),
            )
            .child(div().id("export-body").flex_1().overflow_y_scroll().px(px(18.)).py(px(12.)).child(body))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .px(px(18.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(c(THEME.line))
                    .when(!self.started && page > 0, |d| {
                        d.child(button("back", tr("Back"), false, 0).h(px(24.)).px(px(10.)).on_click(cx.listener(
                            |e, _, _, cx| {
                                e.page -= 1;
                                cx.notify();
                            },
                        )))
                    })
                    .when(!self.started && !last, |d| {
                        d.child(
                            button("next", tr("Next"), false, 0)
                                .h(px(24.))
                                .px(px(10.))
                                .debug_selector(|| "next".into())
                                .on_click(cx.listener(|e, _, _, cx| {
                                    e.page += 1;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!self.started && last, |d| {
                        d.child(
                            tbutton("start-export", tr("Export"), can_export, THEME.green)
                                .debug_selector(|| "start-export".into())
                                .when(!can_export, |b| b.opacity(0.5))
                                .when(can_export, |b| b.on_click(cx.listener(|e, _, _, cx| e.start(cx)))),
                        )
                    })
                    .child(
                        button("close-export", if self.started { tr("Close") } else { tr("Cancel") }, false, 0)
                            .h(px(24.))
                            .px(px(10.))
                            .on_click(cx.listener(|e, _, _, cx| e.scx.close(cx))),
                    ),
            )
    }
}
