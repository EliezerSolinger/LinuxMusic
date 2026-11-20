//! Owner: agent D. Exporting audio (I-13, F-PRJ-022 to F-PRJ-025): what (the whole song, always exact; a range — the
//! loop, between two markers, or bars typed in — with the choice "Exact" or "Fast" and the warm-up, and the cost of
//! each, U37 and V18: Fast with 10 s is the default; one track with the others muted), as a loop with N repeats or
//! with the tail, stems per track or per desk channel (P-40); several formats at once (WAV, FLAC, OGG, MP3) with the
//! rate, the depth, FLAC's compression, OGG's quality, MP3's rate and mode; dither, normalizing by peak or loudness,
//! fades; the folder and the name with the files it writes; the third-party plugins that do not promise the same
//! result twice (H18); the progress with cancel (the shared loading component, I-2) and the report at the end.
//!
//! The export runs in the core as an operation ("export.start" with the request as data, through the port); this
//! screen can close while it runs, and the notices dock shows it.

use fake_core::project::{
    Depth, ExportRequest, Format, Mp3Mode, Normalize, RangeMode, Scope, Stems, export_cost, export_files,
};
use fake_core::{NodeId, Target};
use gpui::{AnyElement, Context, Entity, FontWeight, Render, SharedString, Subscription, Window, div, prelude::*, px};
use gpui_component::input::{Input, InputEvent, InputState};

use super::ScreenCx;
use super::palette::{port_action, run_action};
use super::settings::{badge, khz, modal_size, note, radio_card, segmented, stepper, switch, target_loading};
use crate::ui::{THEME, button, c, caption, tbutton, tr, trf};

/// What the range is taken from (the request keeps only the bars)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    Song,
    Loop,
    Markers,
    Bars,
    Track,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StemKind {
    None,
    Tracks,
    Channels,
}

pub struct Export {
    scx: ScreenCx,
    req: ExportRequest,
    what: What,
    /// a marker index (the region from it to the next)
    marker: usize,
    bars: (u32, u32),
    fast: bool,
    warmup: f64,
    track: usize,
    stem_kind: StemKind,
    stems: Vec<NodeId>,
    folder: Entity<InputState>,
    name: Entity<InputState>,
    _subs: Vec<Subscription>,
}

const RATES: [u32; 5] = [44_100, 48_000, 88_200, 96_000, 192_000];
const KBPS: [u32; 4] = [128, 192, 256, 320];

impl Export {
    pub fn new(scx: ScreenCx, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let port = scx.core.read(cx).port();
        let sv = port.services();
        let folder_v = format!("{}/exports", sv.project.folder);
        let name_v = sv.project.name.clone();
        let rate = sv.project.rate;
        let lb = song_of(&scx, cx).loop_bars;
        let folder = cx.new(|cx| InputState::new(window, cx).default_value(folder_v));
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name_v));
        let subs = vec![
            cx.subscribe(&folder, |_: &mut Export, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
            cx.subscribe(&name, |_: &mut Export, _, ev: &InputEvent, cx| if matches!(ev, InputEvent::Change) { cx.notify() }),
        ];
        Export {
            scx,
            req: ExportRequest {
                scope: Scope::Song,
                formats: vec![Format::Wav],
                rate,
                depth: Depth::I24,
                dither: true,
                mp3_kbps: 320,
                mp3_mode: Mp3Mode::Joint,
                ogg_quality: 6,
                flac_level: 5,
                normalize: Normalize::None,
                fade_in: 0.0,
                fade_out: 0.0,
                tail: true,
                repeats: 1,
                only_track: None,
                stems: Stems::None,
                folder: String::new(),
                name: String::new(),
            },
            what: What::Song,
            marker: 0,
            bars: lb,
            fast: true,
            warmup: 10.0,
            track: 0,
            stem_kind: StemKind::None,
            stems: Vec::new(),
            folder,
            name,
            _subs: subs,
        }
    }

    /// The request as it stands (the scope from what was chosen)
    fn request(&self, cx: &Context<Self>) -> ExportRequest {
        let port = self.scx.core.read(cx).port();
        let root = port.root();
        let mode = if self.fast { RangeMode::Fast { warmup: self.warmup } } else { RangeMode::Exact };
        let s = song_of(&self.scx, cx);
        let markers = marker_bars(&self.scx, cx);
        let range = |a: u32, b: u32| Scope::Range { start: a.max(1), end: b.max(a + 1), mode };
        let mut r = self.req.clone();
        r.only_track = None;
        r.scope = match self.what {
            What::Song => Scope::Song,
            What::Loop => range(s.loop_bars.0, s.loop_bars.1),
            What::Markers => {
                let (a, b) = markers.get(self.marker).map_or((1, s.bars + 1), |m| (m.1, m.2));
                range(a, b)
            }
            What::Bars => range(self.bars.0, self.bars.1),
            What::Track => {
                r.only_track = root.tracks.get(self.track).map(|t| t.id);
                Scope::Song
            }
        };
        r.stems = match self.stem_kind {
            StemKind::None => Stems::None,
            StemKind::Tracks => Stems::Tracks(self.stems.clone()),
            StemKind::Channels => Stems::Channels(self.stems.clone()),
        };
        r.folder = self.folder.read(cx).value().to_string();
        r.name = self.name.read(cx).value().to_string();
        r
    }

    fn toggle_format(&mut self, f: Format, cx: &mut Context<Self>) {
        if let Some(i) = self.req.formats.iter().position(|x| *x == f) {
            self.req.formats.remove(i);
        } else {
            self.req.formats.push(f);
            self.req.formats.sort_by_key(|x| Format::ALL.iter().position(|y| y == x));
        }
        cx.notify();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let req = self.request(cx);
        if req.formats.is_empty() || req.name.trim().is_empty() {
            return;
        }
        port_action(&self.scx, "export.start", req.to_params(), cx);
        cx.notify();
    }
}

fn song_of(scx: &ScreenCx, cx: &gpui::App) -> fake_core::project::Song {
    // the song as the export sees it, read from the port (the length of the items, the loop range)
    let port = scx.core.read(cx).port();
    let root = port.root();
    let bar = fake_core::arrange::BAR;
    let end = root.tracks.iter().flat_map(|t| t.items.iter()).map(|i| i.start + i.length).max().unwrap_or(0);
    let lr = &root.song.loop_range;
    fake_core::project::Song {
        bars: ((end + bar - 1) / bar).max(1) as u32,
        bpm: 60.0 * 4.0 * fake_core::SAMPLE_RATE as f64 / bar as f64,
        loop_bars: ((lr.start / bar) as u32 + 1, (lr.end / bar) as u32 + 1),
        render_speed: port.services().render_speed,
    }
}

/// Markers as regions: (name, first bar, the next marker's bar or the song's end)
fn marker_bars(scx: &ScreenCx, cx: &gpui::App) -> Vec<(String, u32, u32)> {
    let root = scx.core.read(cx).port().root();
    let bar = fake_core::arrange::BAR;
    let end = song_of(scx, cx).bars + 1;
    let m = &root.song.markers;
    m.iter()
        .enumerate()
        .map(|(i, k)| {
            let a = (k.at / bar) as u32 + 1;
            let b = m.get(i + 1).map_or(end, |n| (n.at / bar) as u32 + 1);
            (k.name.clone(), a, b.max(a + 1))
        })
        .collect()
}

fn mmss(s: f64) -> String {
    let s = s.max(0.0);
    if s < 60.0 { format!("{s:.1} s") } else { format!("{}:{:02}", (s / 60.0) as u32, (s % 60.0) as u32) }
}

fn section(title: &'static str) -> gpui::Div {
    div().mt(px(6.)).text_size(px(12.)).font_weight(FontWeight::BOLD).text_color(c(THEME.text2)).child(tr(title))
}

fn line(label: &'static str, control: impl IntoElement) -> gpui::Div {
    div().flex().items_center().gap(px(10.)).child(div().w(px(130.)).flex_none().text_size(px(12.)).child(tr(label))).child(control)
}

impl Render for Export {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = modal_size(window, 1060.0, 760.0);
        let req = self.request(cx);
        let s = song_of(&self.scx, cx);
        let markers = marker_bars(&self.scx, cx);
        let port = self.scx.core.read(cx).port();
        let sv = port.services();
        let project_rate = sv.project.rate;
        let exporting = sv.exporting;
        let report = sv.last_export.clone();
        let third: Vec<(String, String)> = sv.plugins.iter().filter(|p| p.third_party).map(|p| (p.name.clone(), p.track.clone())).collect();
        let tracks: Vec<(NodeId, String)> = port.root().tracks.iter().map(|t| (t.id, t.name.clone())).collect();
        let channels: Vec<(NodeId, String)> = port.root().channels.iter().map(|c| (c.id, c.name.clone())).collect();

        // --- what -------------------------------------------------------------------------------------------
        let what_items = [
            (What::Song, "The whole song"),
            (What::Loop, "The loop range"),
            (What::Markers, "Between markers"),
            (What::Bars, "Bars"),
            (What::Track, "One track"),
        ];
        let what = self.what;
        let is_range = matches!(what, What::Loop | What::Markers | What::Bars);
        let mut what_col = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(section("What to export"))
            .child(segmented("what", what_items.iter().map(|(k, l)| (tr(l).into(), *k == what, true)).collect(), cx, move |e, i, _, cx| {
                e.what = what_items[i].0;
                cx.notify();
            }));
        match what {
            What::Song => {
                what_col = what_col.child(caption(trf("Bars 1 to {0}, {1}; always exact", &[&s.bars, &mmss(s.seconds())])));
            }
            What::Loop => {
                what_col = what_col.child(caption(trf("Bars {0} to {1} (the loop on the ruler)", &[&s.loop_bars.0, &(s.loop_bars.1 - 1)])));
            }
            What::Markers => {
                what_col = what_col.child(div().flex().flex_wrap().gap(px(4.)).children(markers.iter().enumerate().map(|(i, (n, a, b))| {
                    tbutton(("marker", i), format!("{n} ({a}–{})", b - 1), i == self.marker, THEME.green).on_click(cx.listener(move |e, _, _, cx| {
                        e.marker = i;
                        cx.notify();
                    }))
                })));
            }
            What::Bars => {
                let (a, b) = self.bars;
                what_col = what_col.child(
                    div()
                        .flex()
                        .gap(px(14.))
                        .child(line("From bar", stepper("bar-from", a.to_string(), cx, move |e, d, cx| {
                            e.bars.0 = (a as i32 + d).clamp(1, b as i32 - 1) as u32;
                            cx.notify();
                        })))
                        .child(stepper("bar-to", trf("to {0}", &[&(b - 1)]), cx, move |e, d, cx| {
                            e.bars.1 = (b as i32 + d).clamp(a as i32 + 1, s.bars as i32 + 1) as u32;
                            cx.notify();
                        })),
                );
            }
            What::Track => {
                what_col = what_col.child(caption(tr("The whole song with the other tracks muted"))).child(
                    div().id("track-list").max_h(px(120.)).overflow_y_scroll().flex().flex_wrap().gap(px(4.)).children(tracks.iter().enumerate().take(64).map(|(i, (_, n))| {
                        button(("only-track", i), n.clone(), i == self.track, THEME.green).on_click(cx.listener(move |e, _, _, cx| {
                            e.track = i;
                            cx.notify();
                        }))
                    })),
                );
            }
        }
        // Exact or Fast, with the cost of each (U37, V18)
        if is_range {
            let mut exact = req.clone();
            let mut fast = req.clone();
            if let Scope::Range { start, end, .. } = req.scope {
                exact.scope = Scope::Range { start, end, mode: RangeMode::Exact };
                fast.scope = Scope::Range { start, end, mode: RangeMode::Fast { warmup: self.warmup } };
            }
            let (ea, ec) = export_cost(&s, &exact);
            let (fa, fc) = export_cost(&s, &fast);
            let warm = self.warmup;
            what_col = what_col
                .child(
                    radio_card("mode-fast", self.fast, tr("Fast"), trf("Starts {0} s before and drops them: renders {1}, about {2} of work. Almost identical, and the report says so", &[&warm, &mmss(fa), &mmss(fc)]))
                        .debug_selector(|| "mode-fast".into())
                        .on_click(cx.listener(|e, _, _, cx| {
                            e.fast = true;
                            cx.notify();
                        })),
                )
                .when(self.fast, |d| {
                    d.child(line("Warm-up", stepper("warmup", trf("{0} s", &[&warm]), cx, move |e, dd, cx| {
                        e.warmup = (warm + dd as f64).clamp(0.0, 60.0);
                        cx.notify();
                    })))
                })
                .child(
                    radio_card("mode-exact", !self.fast, tr("Exact"), trf("Renders from the start of the song and drops what comes before: renders {0}, about {1} of work. Identical, bit for bit, to the same range in the whole song", &[&mmss(ea), &mmss(ec)]))
                        .debug_selector(|| "mode-exact".into())
                        .on_click(cx.listener(|e, _, _, cx| {
                            e.fast = false;
                            cx.notify();
                        })),
                );
        }
        let (tail, repeats) = (req.tail, req.repeats);
        what_col = what_col
            .child(section("Loop and tail"))
            .child(line("Keep the tail", switch("tail", tail, cx, |e: &mut Export, v, cx| {
                e.req.tail = v;
                cx.notify();
            })))
            .child(caption(tr("On: reverbs and releases ring past the end; off: cut as a loop, without the tail")))
            .child(line("Repeat", stepper("repeats", trf("{0} times", &[&repeats]), cx, move |e, d, cx| {
                e.req.repeats = (repeats as i32 + d).clamp(1, 64) as u32;
                cx.notify();
            })));

        // stems (P-40): per track or per desk channel
        let kind = self.stem_kind;
        let stem_list: Vec<(NodeId, String)> = match kind {
            StemKind::Tracks => tracks.clone(),
            StemKind::Channels => channels.clone(),
            StemKind::None => Vec::new(),
        };
        what_col = what_col
            .child(section("Stems"))
            .child(segmented(
                "stems",
                vec![
                    (tr("None").into(), kind == StemKind::None, true),
                    (tr("One file per track").into(), kind == StemKind::Tracks, true),
                    (tr("One file per desk channel").into(), kind == StemKind::Channels, true),
                ],
                cx,
                |e, i, _, cx| {
                    e.stem_kind = [StemKind::None, StemKind::Tracks, StemKind::Channels][i];
                    e.stems.clear();
                    cx.notify();
                },
            ))
            .when(kind != StemKind::None, |d| {
                let all: Vec<NodeId> = stem_list.iter().map(|(id, _)| *id).collect();
                d.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(button("stems-all", tr("All"), false, 0).on_click(cx.listener(move |e, _, _, cx| {
                            e.stems = all.clone();
                            cx.notify();
                        })))
                        .child(button("stems-none", tr("None"), false, 0).on_click(cx.listener(|e, _, _, cx| {
                            e.stems.clear();
                            cx.notify();
                        })))
                        .child(caption(trf("{0} chosen", &[&self.stems.len()]))),
                )
                .child(
                    div().id("stem-list").max_h(px(130.)).overflow_y_scroll().flex().flex_wrap().gap(px(4.)).children(stem_list.iter().enumerate().map(|(i, (id, n))| {
                        let id = *id;
                        button(("stem", i), n.clone(), self.stems.contains(&id), THEME.green).debug_selector(move || format!("stem-{i}")).on_click(cx.listener(move |e, _, _, cx| {
                            if let Some(k) = e.stems.iter().position(|x| *x == id) {
                                e.stems.remove(k);
                            } else {
                                e.stems.push(id);
                            }
                            cx.notify();
                        }))
                    })),
                )
            });

        // --- how --------------------------------------------------------------------------------------------
        let has = |f: Format| req.formats.contains(&f);
        let any_lossless = req.formats.iter().any(|f| f.lossless());
        let depth_items: Vec<(SharedString, bool, bool)> = vec![
            (tr("16 bits").into(), req.depth == Depth::I16, any_lossless),
            (tr("24 bits").into(), req.depth == Depth::I24, any_lossless),
            (tr("32 bits float").into(), req.depth == Depth::F32, has(Format::Wav)),
        ];
        let (ogg, flac, kbps) = (req.ogg_quality, req.flac_level, req.mp3_kbps);
        let norm_target = match req.normalize {
            Normalize::Peak(x) | Normalize::Lufs(x) => Some(x),
            Normalize::None => None,
        };
        let (fi, fo) = (req.fade_in, req.fade_out);
        let how_col = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(section("Formats"))
            .child(caption(tr("Several at once: the song renders once and each format is written from it")))
            .child(div().flex().gap(px(4.)).children(Format::ALL.iter().map(|&f| {
                tbutton(("fmt", f as usize), f.ext().to_uppercase(), has(f), THEME.green)
                    .debug_selector(move || format!("fmt-{}", f.ext()))
                    .on_click(cx.listener(move |e, _, _, cx| e.toggle_format(f, cx)))
            })))
            .child(line("Bit depth", segmented("depth", depth_items, cx, |e, i, _, cx| {
                e.req.depth = [Depth::I16, Depth::I24, Depth::F32][i];
                cx.notify();
            })))
            .when(has(Format::Flac), |d| {
                d.child(line("FLAC compression", stepper("flac", trf("level {0}", &[&flac]), cx, move |e, dd, cx| {
                    e.req.flac_level = (flac as i32 + dd).clamp(0, 8) as u32;
                    cx.notify();
                })))
            })
            .when(has(Format::Ogg), |d| {
                d.child(line("OGG quality", stepper("ogg", trf("{0} of 10", &[&ogg]), cx, move |e, dd, cx| {
                    e.req.ogg_quality = (ogg as i32 + dd).clamp(0, 10) as u32;
                    cx.notify();
                })))
            })
            .when(has(Format::Mp3), |d| {
                d.child(line("MP3", div().flex().flex_col().gap(px(4.)).child(segmented("kbps", KBPS.iter().map(|k| (format!("{k} kbps").into(), *k == kbps, true)).collect(), cx, |e, i, _, cx| {
                    e.req.mp3_kbps = KBPS[i];
                    cx.notify();
                })).child(segmented(
                    "mp3-mode",
                    vec![
                        (tr("Stereo").into(), req.mp3_mode == Mp3Mode::Stereo, true),
                        (tr("Joint stereo").into(), req.mp3_mode == Mp3Mode::Joint, true),
                        (tr("Mono").into(), req.mp3_mode == Mp3Mode::Mono, true),
                    ],
                    cx,
                    |e, i, _, cx| {
                        e.req.mp3_mode = [Mp3Mode::Stereo, Mp3Mode::Joint, Mp3Mode::Mono][i];
                        cx.notify();
                    },
                ))))
            })
            .child(line(
                "Sample rate",
                segmented(
                    "exp-rate",
                    RATES.iter().map(|r| (if *r == project_rate { trf("{0} (project)", &[&khz(*r)]).into() } else { khz(*r).into() }, req.rate == *r, true)).collect(),
                    cx,
                    |e, i, _, cx| {
                        e.req.rate = RATES[i];
                        cx.notify();
                    },
                ),
            ))
            .when(req.rate != project_rate, |d| d.child(caption(trf("Converted once from the project's {0}, in high quality", &[&khz(project_rate)]))))
            .child(section("Processing"))
            .child(line("Dither (TPDF)", switch("dither", req.dither, cx, |e: &mut Export, v, cx| {
                e.req.dither = v;
                cx.notify();
            })))
            .child(caption(tr("To 16 and 24 bits; float and the lossy formats need none")))
            .child(line(
                "Normalize",
                segmented(
                    "normalize",
                    vec![
                        (tr("None").into(), matches!(req.normalize, Normalize::None), true),
                        (tr("Peak (dBFS)").into(), matches!(req.normalize, Normalize::Peak(_)), true),
                        (tr("Loudness (LUFS)").into(), matches!(req.normalize, Normalize::Lufs(_)), true),
                    ],
                    cx,
                    |e, i, _, cx| {
                        e.req.normalize = [Normalize::None, Normalize::Peak(-1.0), Normalize::Lufs(-14.0)][i];
                        cx.notify();
                    },
                ),
            ))
            .when_some(norm_target, |d, t| {
                d.child(line("Target", stepper("norm-target", format!("{t:.0}"), cx, move |e, dd, cx| {
                    let v = (t + dd as f64).clamp(-60.0, 0.0);
                    e.req.normalize = match e.req.normalize {
                        Normalize::Peak(_) => Normalize::Peak(v),
                        Normalize::Lufs(_) => Normalize::Lufs(v),
                        Normalize::None => Normalize::None,
                    };
                    cx.notify();
                })))
                .child(caption(tr("Loudness by ITU-R BS.1770; the target goes from -60 to 0")))
            })
            .child(line("Fades", div().flex().gap(px(10.)).child(stepper("fade-in", trf("in {0} s", &[&fi]), cx, move |e, d, cx| {
                e.req.fade_in = (fi + d as f64 * 0.5).clamp(0.0, 30.0);
                cx.notify();
            })).child(stepper("fade-out", trf("out {0} s", &[&fo]), cx, move |e, d, cx| {
                e.req.fade_out = (fo + d as f64 * 0.5).clamp(0.0, 30.0);
                cx.notify();
            }))))
            .child(section("Where"))
            .child(line("Folder", div().w(px(320.)).child(Input::new(&self.folder))))
            .child(line("Name", div().w(px(320.)).child(Input::new(&self.name))));

        // the files it writes
        let names: Vec<String> = match &req.stems {
            Stems::None => Vec::new(),
            Stems::Tracks(v) => tracks.iter().filter(|(id, _)| v.contains(id)).map(|(_, n)| n.clone()).collect(),
            Stems::Channels(v) => channels.iter().filter(|(id, _)| v.contains(id)).map(|(_, n)| n.clone()).collect(),
        };
        let files = export_files(&req, &names);
        let (_, cost) = export_cost(&s, &req);

        // --- the bottom: H18, progress, report, start --------------------------------------------------------
        let progress: Option<AnyElement> = exporting.map(|(id, _)| {
            target_loading(&self.scx, "export-loading", Target::Export(id), cx)
                .map(|l| div().h(px(30.)).child(l).into_any_element())
                .unwrap_or_else(|| caption(tr("Starting…")).into_any_element())
        });
        let can_start = exporting.is_none() && !req.formats.is_empty() && !req.name.trim().is_empty();
        let scx = self.scx.clone();
        let bottom = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pt(px(10.))
            .border_t_1()
            .border_color(c(THEME.line))
            .when(!third.is_empty(), |d| {
                d.child(note(
                    trf(
                        "These third-party plugins do not promise the same result twice, so two exports may differ: {0}",
                        &[&third.iter().map(|(p, t)| format!("{p} ({t})")).collect::<Vec<_>>().join(", ")],
                    ),
                    THEME.orange,
                ))
            })
            .when_some(report.filter(|_| exporting.is_none()), |d, r| {
                let folder = r.folder.clone();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(note(
                            if r.identical {
                                trf("Exported {0} files in {1}", &[&r.files.len(), &r.folder])
                            } else {
                                trf("Exported {0} files in {1}; fast range: almost identical to the same range in the whole song, not bit for bit", &[&r.files.len(), &r.folder])
                            },
                            THEME.green,
                        ))
                        .child(button("reveal", tr("Show the files"), false, 0).on_click(move |_, window, cx| run_action(&scx, "export.reveal", &folder, window, cx))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().flex_1().min_w_0().children(progress).when(exporting.is_none(), |d| {
                        d.child(caption(trf("{0} files, about {1} of work: {2}", &[&files.len(), &mmss(cost), &files.iter().take(4).cloned().collect::<Vec<_>>().join(", ")])))
                    }))
                    .child(button("close-export", tr("Close"), false, 0).on_click(cx.listener(|e, _, _, cx| e.scx.close(cx))))
                    .child(
                        tbutton("start-export", tr("Export"), can_start, THEME.green)
                            .debug_selector(|| "start-export".into())
                            .when(!can_start, |b| b.opacity(0.5))
                            .when(can_start, |b| b.on_click(cx.listener(|e, _, _, cx| e.start(cx)))),
                    ),
            );

        div()
            .id("export")
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .p(px(18.))
            .gap(px(8.))
            .rounded(px(10.))
            .bg(c(THEME.bg2))
            .border_1()
            .border_color(c(THEME.line2))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().text_size(px(19.)).font_weight(FontWeight::BOLD).child(tr("Export audio")))
                    .child(badge(if matches!(req.scope, Scope::Range { mode: RangeMode::Fast { .. }, .. }) { tr("almost identical") } else { tr("exact") }, THEME.green, false)),
            )
            .child(
                div()
                    .id("export-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .gap(px(24.))
                    .child(div().flex_1().min_w_0().child(what_col))
                    .child(div().w(px(1.)).bg(c(THEME.line)))
                    .child(div().flex_1().min_w_0().child(how_col)),
            )
            .child(bottom)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::Screen;
    use super::super::palette::testing::*;

    /// A range defaults to Fast with 10 s of warm-up (V18); the export runs in the core, with the request as data,
    /// and its report says the fast range is not bit for bit
    #[gpui::test]
    fn export_a_range_fast(cx: &mut gpui::TestAppContext) {
        let (ws, cx) = open(cx);
        open_screen(&ws, cx, Screen::Export);
        click(cx, "what-1");
        click(cx, "fmt-mp3");
        click(cx, "start-export");
        let core = ws.read_with(cx, |ws, _| ws.core.clone());
        core.read_with(cx, |c, _| assert!(c.port().services().exporting.is_some(), "the export is running"));
        // the fake export's worker takes real time (0.5 s for this range)
        std::thread::sleep(Duration::from_millis(900));
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        core.read_with(cx, |c, _| {
            let sv = c.port().services();
            assert!(sv.exporting.is_none());
            let r = sv.last_export.as_ref().expect("a report");
            assert!(!r.identical, "fast: almost identical, and said so");
            assert!(r.files.iter().any(|f| f.ends_with(".mp3")) && r.files.iter().any(|f| f.ends_with(".wav")));
        });
    }
}
