//! The tests that drive no view (assets, texts, the snapshots): they run in release too. The views' tests
//! (`tests.rs`) need GPUI's `debug_selector`, which exists only with debug assertions.

use crate::panels;
use crate::spec::{Bind, Kind};
use crate::widgets::Range;

#[test]
fn every_asset_decodes() {
    for a in crate::assets::lmms::ALL.iter().chain(panels::ASSETS.iter().flat_map(|a| a.iter())) {
        if !a.is_svg() {
            let img = crate::assets::image(a);
            assert!(img.width() > 1 && img.height() > 1, "{} decodes", a.path);
        }
    }
}

#[test]
fn every_text_has_its_translation() {
    let table: std::collections::HashSet<&str> = crate::i18n::FILE
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(" = ").map(|(k, _)| k.trim()))
        .collect();
    let mut missing: Vec<&str> = Vec::new();
    let mut want = |k: &'static str| {
        if !k.is_empty() && !table.contains(k) && !missing.contains(&k) {
            missing.push(k);
        }
    };
    for (_, spec) in panels::ALL {
        for l in &spec().labels {
            want(l.text.key);
        }
        for c in &spec().controls {
            want(c.name.key);
            want(c.tip.key);
            if let Kind::Combo(cb) = &c.kind {
                for i in &cb.items {
                    want(i.text.key);
                }
            }
            if let Kind::Fader(f) = &c.kind {
                want(f.hint.key);
            }
            if let Kind::Readout(r) = &c.kind {
                want(r.empty.key);
            }
            if let Kind::Knob(k) = &c.kind {
                want(k.hint.key);
                if let Some(l) = &k.label {
                    want(l.key);
                }
                if k.unit.trim().chars().any(char::is_alphabetic) {
                    want(k.unit.trim());
                }
            }
        }
    }
    for k in crate::panel::TEXTS {
        want(k);
    }
    assert!(missing.is_empty(), "missing in locales/pt-BR/panels.txt: {missing:#?}");
}

/// Writes each panel at its defaults as a PNG (`LM_PANEL_SNAPSHOTS=<folder>`), to compare by eye with a screenshot of
/// the legacy; the backgrounds in SVG are left out (GPUI draws them)
#[test]
fn snapshots() {
    let Ok(dir) = std::env::var("LM_PANEL_SNAPSHOTS") else { return };
    let schemas: Vec<_> = [&lm_plugins_tripleoscillator::DESCRIPTOR]
        .into_iter()
        .chain(lm_plugins_instruments::ALL.iter().copied())
        .chain([&lm_plugins_sid::DESCRIPTOR, &lm_plugins_freeboy::DESCRIPTOR, &lm_plugins_opulenz::DESCRIPTOR])
        .chain(lm_plugins_effects::ALL.iter().copied())
        .map(|d| d.schema())
        .collect();
    for (id, spec) in panels::ALL {
        let spec = spec();
        let Some(schema) = schemas.iter().find(|s| s.type_name == format!("plugin:{id}")) else { continue };
        for scale in [1.0f32, 2.0] {
            let mut c = crate::raster::Canvas::new(spec.size.0, spec.size.1, scale);
            c.fill_rect(0.0, 0.0, spec.size.0, spec.size.1, crate::raster::Rgba::hex(spec.fill));
            if let Some(bg) = spec.background.filter(|b| !b.is_svg()) {
                c.pixmap(crate::assets::image(bg), 0.0, 0.0, false);
            }
            for ctl in spec.controls.iter().filter(|c| c.in_view(0) && c.when.is_none_or(|(_, on)| on)) {
                let (w, h) = ctl.size();
                let mut one = crate::raster::Canvas::new(w, h, scale);
                match &ctl.kind {
                    Kind::Knob(k) if k.choices.is_some() => {
                        let n = k.choices.map_or(1, |c| c.len()) as f64 - 1.0;
                        let r = Range { min: 0.0, max: n, step: 1.0, center: 0.0, log: false };
                        crate::widgets::draw_knob(&mut one, &k.style, r.angle(0.0), r.angle(0.0), true);
                    }
                    Kind::Knob(k) if k.int_field => {
                        let f = schema.find_field(k.param).unwrap();
                        let (min, max) = match f.kind {
                            lm_port::FieldKind::Int { min, max } => (min as f64, max as f64),
                            _ => (0.0, 1.0),
                        };
                        let def = f.default.as_ref().and_then(|d| d.as_int()).unwrap_or(0) as f64;
                        let r = Range { min, max, step: 1.0, center: min, log: false };
                        crate::widgets::draw_knob(&mut one, &k.style, r.angle(def), r.angle(r.center), true);
                    }
                    Kind::Knob(k) => {
                        let p = schema.find_param(k.param).unwrap();
                        let r = Range {
                            min: p.min,
                            max: p.max,
                            step: k.step,
                            center: k.center.unwrap_or(p.min),
                            log: false,
                        };
                        crate::widgets::draw_knob(&mut one, &k.style, r.angle(p.def), r.angle(r.center), true);
                    }
                    Kind::Button(b) => {
                        let on = match b.bind {
                            Bind::Choice { field, key } => {
                                schema
                                    .find_field(field)
                                    .and_then(|f| f.default.clone())
                                    .and_then(|d| d.as_text().map(|t| t == key))
                                    == Some(true)
                            }
                            Bind::Switch { param } => schema.find_param(param).is_some_and(|p| p.def >= 0.5),
                            Bind::Step { param, value } => {
                                schema.find_param(param).is_some_and(|p| (p.def - value).abs() < 0.5)
                            }
                            Bind::Flag { .. } | Bind::Press(_) | Bind::Momentary { .. } | Bind::Open { .. } | Bind::Action { .. } => false,
                            Bind::View(n) => n == 0,
                        };
                        one.pixmap(crate::assets::image(if on { b.active } else { b.inactive }), 0.0, 0.0, false);
                    }
                    Kind::Led(l) => one.pixmap(crate::assets::image(l.color.asset()), 0.0, 0.0, false),
                    Kind::Lcd(l) if l.frac > 0 => crate::widgets::draw_lcd_float(
                        &mut one,
                        l.sheet,
                        l.digits,
                        l.frac,
                        &crate::widgets::lcd_float_texts(0.0, l.frac),
                        true,
                    ),
                    Kind::Lcd(l) => crate::widgets::draw_lcd(&mut one, l.sheet, l.digits, "0", true),
                    Kind::Graph(g) => {
                        let def = schema.find_field(g.field).and_then(|f| f.default.clone()).unwrap_or_default();
                        let mut v: Vec<f64> = def.items().map(|x| x.as_real().unwrap_or(0.0)).collect();
                        v.resize(g.length, 0.0);
                        crate::widgets::draw_graph(&mut one, g, &v, None);
                    }
                    Kind::Fader(f) => {
                        let p = schema.find_param(f.param).unwrap();
                        crate::widgets::draw_fader(&mut one, f, p.def, p.min, p.max, None);
                    }
                    Kind::Surface(sf) => {
                        let param = |k: &str| schema.find_param(k).map_or(0.0, |p| p.def);
                        let range = |k: &str| schema.find_param(k).map_or((0.0, 1.0), |p| (p.min, p.max));
                        let field = |k: &str| schema.find_field(k).and_then(|f| f.default.clone()).unwrap_or_default();
                        let sc = crate::spec::SurfaceCx {
                            w: sf.w,
                            h: sf.h,
                            param: &param,
                            field: &field,
                            frame: &[],
                            active: None,
                            range: &range,
                            peaks: None,
                            window: (0.0, 1.0),
                            rate: 48_000.0,
                            primary: sf.primary,
                            tag: sf.tag,
                            zoom: crate::spec::zoom_of(sf.tag).0,
                            history: &[],
                        };
                        (sf.draw)(&mut one, &sc);
                    }
                    Kind::Toggle(t) => (t.draw)(&mut one, false),
                    Kind::Slider(s) => {
                        let ph = crate::assets::image(s.handle).height() as f32;
                        let top = ((s.a + s.b) / 2.0 - ph / 2.0).trunc() - ctl.y;
                        one.pixmap(crate::assets::image(s.handle), 0.0, top, false);
                    }
                    Kind::Readout(_) | Kind::TextEdit(_) | Kind::List(_) => {}
                    Kind::Combo(cb) => {
                        let def = schema.find_field(cb.field).and_then(|f| f.default.clone());
                        let icon = cb
                            .items
                            .iter()
                            .find(|i| def.as_ref().and_then(|d| d.as_text()) == Some(i.key))
                            .and_then(|i| i.icon);
                        let _ = crate::widgets::draw_combo(&mut one, cb.w, icon.map(crate::assets::image), false);
                    }
                }
                c.composite(&one, ctl.x, ctl.y);
            }
            let path = format!("{dir}/{id}@{scale}x.png");
            c.save_png(&path);
        }
    }
}
