//! The editor: a vector-display panel in the look of the CHIMERA web app (blue for the body,
//! amber for sound, green/red/pink/white for the tongue, jaw, lips and soft palate).

use crate::performer::{nearest_pad_vowel, pad_position, PAD_VOWELS};
use crate::shared::{Shared, SCOPE_LEN};
use crate::{lyrics, ChimeraParams, ModeParam};
use chimera::anatomy::geom::Vec2 as V2;
use chimera::anatomy::human::{BodyMix, HumanArtic, HumanFrame, HumanVocalTract, N_ARTIC};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind};
use nih_plug_egui::create_egui_editor;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub const WIDTH: u32 = 960;
pub const HEIGHT: u32 = 640;

const BG: Color32 = Color32::from_rgb(2, 5, 9);
const PANEL: Color32 = Color32::from_rgb(5, 12, 19);
const FRAME: Color32 = Color32::from_rgb(19, 57, 84);
const BLUE: Color32 = Color32::from_rgb(79, 182, 242);
const BLUE_MID: Color32 = Color32::from_rgb(52, 135, 196);
const BLUE_DIM: Color32 = Color32::from_rgb(31, 95, 140);
const GREEN: Color32 = Color32::from_rgb(91, 227, 129);
const RED: Color32 = Color32::from_rgb(255, 93, 64);
const PINK: Color32 = Color32::from_rgb(255, 115, 207);
const WHITE: Color32 = Color32::from_rgb(238, 246, 248);
const AMBER: Color32 = Color32::from_rgb(255, 179, 64);
const TEXT: Color32 = Color32::from_rgb(226, 245, 252);

static HERSHEY: &[u8] = include_bytes!("../vendor/chimera-vocal-synth/public/fonts/AVHersheySimplexMedium.otf");

fn vfont(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("hershey".into()))
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// A glowing line: a wide faint pass under a thin bright one.
fn beam(p: &egui::Painter, pts: Vec<Pos2>, color: Color32, width: f32, a: f32, closed: bool) {
    if pts.len() < 2 {
        return;
    }
    let mk = |pts: Vec<Pos2>, s: Stroke| if closed { Shape::closed_line(pts, s) } else { Shape::line(pts, s) };
    p.add(mk(pts.clone(), Stroke::new(width * 3.2, alpha(color, 0.13 * a))));
    p.add(mk(pts, Stroke::new(width, alpha(color, a))));
}

fn panel(p: &egui::Painter, r: Rect, title: &str) {
    p.rect_filled(r, 6.0, PANEL);
    p.rect_stroke(r, 6.0, Stroke::new(1.0_f32, FRAME), StrokeKind::Inside);
    if !title.is_empty() {
        p.text(r.left_top() + vec2(12.0, 9.0), Align2::LEFT_TOP, title, vfont(11.0), BLUE_MID);
    }
}

// ---------------------------------------------------------------------- the head

struct Head {
    body: BodyMix,
    tract: HumanVocalTract,
    frame: HumanFrame,
}

fn catmull_rom(p: &[V2], n: usize) -> Vec<V2> {
    let mut out = Vec::with_capacity(p.len() * n + 1);
    if p.len() < 2 {
        return p.to_vec();
    }
    let at = |i: isize| p[i.clamp(0, p.len() as isize - 1) as usize];
    for i in 0..p.len() - 1 {
        let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
        for j in 0..n {
            let t = j as f64 / n as f64;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f64, b: f64, c: f64, d: f64| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3);
            out.push(V2 { x: f(p0.x, p1.x, p2.x, p3.x), y: f(p0.y, p1.y, p2.y, p3.y) });
        }
    }
    out.push(p[p.len() - 1]);
    out
}

fn ellipse(c: V2, rx: f64, ry: f64) -> Vec<V2> {
    (0..=32)
        .map(|i| {
            let an = i as f64 / 32.0 * std::f64::consts::TAU;
            V2 { x: c.x + an.cos() * rx, y: c.y + an.sin() * ry }
        })
        .collect()
}

impl Head {
    fn new(body: BodyMix) -> Head {
        Head { body, tract: HumanVocalTract::new(body.body()), frame: HumanFrame::with_capacity() }
    }

    /// Draw the midsagittal cutaway for this posture (a port of the web app's side view, without
    /// the deep structures).
    fn draw(&mut self, p: &egui::Painter, rect: Rect, artic: &[f32; N_ARTIC], level: f32) {
        let mut a64 = [0.0f64; N_ARTIC];
        for (d, s) in a64.iter_mut().zip(artic.iter()) {
            *d = *s as f64;
        }
        let a = HumanArtic::from_array(&a64);
        self.tract.compute(&a, &mut self.frame);
        let f = &self.frame;
        let t = &self.tract;
        let k = t.scale();
        let ph = self.body.body().pharynx;

        // the whole head, in body cm (the adult male's bounds, so smaller bodies look smaller)
        let (x0, x1, y0, y1) = (-9.4, 10.25, -4.55, 23.5);
        let scale = ((rect.width() as f64 - 16.0) / (x1 - x0)).min((rect.height() as f64 - 16.0) / (y1 - y0));
        // a smaller body sits in the middle of the panel, at its true size beside the adult's
        let ox = rect.center().x as f64 - (x0 + x1) / 2.0 * k * scale;
        let oy = rect.center().y as f64 + (y0 + y1) / 2.0 * k * scale;
        let px = |v: &V2| pos2((ox + v.x * scale) as f32, (oy - v.y * scale) as f32);
        let pts = |v: &[V2]| v.iter().map(px).collect::<Vec<_>>();

        let tp = |x: f64, y: f64| t.p(x, y);
        let b = |x: f64, y: f64| V2 { x: x * k, y: (if y < 7.0 { 7.0 - (7.0 - y) * ph } else { y }) * k };
        let condyle = tp(-2.5, 10.5);
        let far = tp(6.95, 7.75);
        let phi = -(a.jaw.clamp(-0.2, 3.0) * k) / ((condyle.x - far.x).powi(2) + (condyle.y - far.y).powi(2)).sqrt();
        let j = |x: f64, y: f64| {
            let q = tp(x, y);
            let (dx, dy) = (q.x - condyle.x, q.y - condyle.y);
            V2 { x: condyle.x + dx * phi.cos() - dy * phi.sin(), y: condyle.y + dx * phi.sin() + dy * phi.cos() }
        };

        let g = f.glottis;
        // deep structures, dim
        beam(p, pts(&ellipse(b(6.5, 15.4), 1.1 * k, 0.95 * k)), BLUE_MID, 1.0, 0.8, false);
        let nasal = catmull_rom(&[b(0.3, 11.2), b(1.1, 10.25), b(2.7, 10.75), b(4.6, 10.85), b(6.4, 10.65), b(7.6, 10.85), b(8.45, 11.3), b(8.0, 12.2), b(6.7, 13.4), b(4.4, 13.9), b(1.8, 13.6), b(0.2, 12.6)], 4);
        beam(p, pts(&nasal), BLUE_DIM, 1.0, 0.8, true);
        let pal: Vec<V2> = [(1.0, 9.6), (1.8, 9.92), (2.7, 10.18), (3.6, 10.3), (4.4, 10.25), (5.1, 10.05), (5.7, 9.72), (6.15, 9.36), (6.55, 9.15), (7.0, 9.6)].iter().map(|(x, y)| tp(*x, *y)).collect();
        beam(p, pts(&pal), BLUE_MID, 1.0, 0.9, false);
        let pal_nasal: Vec<V2> = pal.iter().map(|q| V2 { x: q.x, y: q.y + 0.55 * k }).collect();
        beam(p, pts(&pal_nasal), BLUE_MID, 1.0, 0.9, false);
        for i in 0..9 {
            let vy = -4.4 + i as f64 * 1.75;
            if vy > 10.5 {
                break;
            }
            let vx = -1.2 - 0.025 * vy - 2.25;
            let (c0, c1) = (tp(vx, vy + 1.4), tp(vx + 1.6, vy));
            beam(p, pts(&[c0, V2 { x: c1.x, y: c0.y }, c1, V2 { x: c0.x, y: c1.y }]), BLUE_DIM, 1.0, 0.7, true);
        }
        beam(p, pts(&[V2 { x: g.x - 0.65 * k, y: g.y }, V2 { x: g.x - 0.75 * k, y: -4.8 }]), BLUE_MID, 1.0, 0.8, false);
        beam(p, pts(&[V2 { x: g.x + 0.75 * k, y: g.y }, V2 { x: g.x + 0.95 * k, y: -4.8 }]), BLUE_MID, 1.0, 0.8, false);

        // skin
        if f.upper_lip.len() > 7 && f.lower_lip.len() > 7 {
            let ul_out = &f.upper_lip[7..];
            let ll_out = &f.lower_lip[7..];
            let mut lower = catmull_rom(&[b(2.3, -4.6), b(2.38, -1.5), b(2.52, 0.4), b(3.0, 1.7), j(5.3, 3.0), j(7.25, 3.35), j(7.95, 4.6), j(7.55, 5.7)], 4);
            lower.extend(ll_out.iter().rev().copied());
            let mut upper: Vec<V2> = ul_out.to_vec();
            let face = catmull_rom(&[ul_out[ul_out.len() - 1], b(7.05, 10.6), b(7.75, 10.78), b(8.75, 11.05), b(9.25, 11.65), b(8.95, 12.6), b(8.35, 13.9), b(7.7, 15.0), b(7.95, 16.1), b(7.9, 18.1), b(6.9, 20.5)], 4);
            let cranium = catmull_rom(&[b(6.9, 20.5), b(3.0, 22.9), b(-2.2, 22.7), b(-6.9, 19.6), b(-8.7, 15.4), b(-8.3, 11.9), b(-7.4, 8.9), b(-6.8, 4.8), b(-6.6, 0.0), b(-6.6, -4.6)], 4);
            upper.extend_from_slice(&face[1..]);
            upper.extend_from_slice(&cranium[1..]);
            beam(p, pts(&lower), BLUE, 1.6, 1.0, false);
            beam(p, pts(&upper), BLUE, 1.6, 1.0, false);
        }

        // airway walls, teeth
        let outer: Vec<V2> = f.outer.iter().filter(|q| q.y > g.y || q.x > 0.0).copied().collect();
        beam(p, pts(&outer), BLUE, 1.3, 1.0, false);
        beam(p, pts(&f.inner), BLUE, 1.2, 0.85, false);
        if f.epiglottis.len() >= 3 {
            beam(p, pts(&f.epiglottis[..3]), BLUE, 2.0, 0.85, false);
        }
        beam(p, pts(&f.upper_teeth), BLUE_MID, 1.0, 1.0, true);
        beam(p, pts(&f.lower_teeth), BLUE_MID, 1.0, 1.0, false);

        // the articulators in their colours
        beam(p, pts(&f.tongue), GREEN, 1.6, 1.0, false);
        let jaw: Vec<V2> = f.jaw_bone.iter().filter(|q| q.x > 0.8 * k).copied().collect();
        beam(p, pts(&jaw), RED, 1.4, 1.0, false);
        beam(p, pts(&f.upper_lip), PINK, 1.5, 1.0, false);
        beam(p, pts(&f.lower_lip), PINK, 1.5, 1.0, false);
        beam(p, pts(&f.velum), WHITE, 1.2, 0.9, false);

        // sound leaving the lips: amber arcs, as loud as the voice
        if level > 0.002 && f.upper_lip.len() > 6 && f.lower_lip.len() > 6 {
            let lips = V2 { x: (f.upper_lip[6].x + f.lower_lip[6].x) / 2.0 + 0.5, y: (f.upper_lip[6].y + f.lower_lip[6].y) / 2.0 };
            let c = px(&lips);
            let loud = (level * 6.0).clamp(0.0, 1.0);
            for i in 0..3 {
                let r = (14.0 + 11.0 * i as f32) * (scale as f32 / 14.0);
                let arc: Vec<Pos2> = (0..=12).map(|s| {
                    let an = -0.7 + 1.4 * s as f32 / 12.0;
                    pos2(c.x + r * an.cos(), c.y + r * an.sin())
                }).collect();
                beam(p, arc, AMBER, 1.4, loud * (1.0 - 0.28 * i as f32), false);
            }
        }
    }
}

// ---------------------------------------------------------------------- widgets

fn drag_param<P: Param>(ui: &egui::Ui, resp: &egui::Response, param: &P, setter: &ParamSetter, id: egui::Id) {
    if resp.drag_started() {
        setter.begin_set_parameter(param);
        ui.memory_mut(|m| m.data.insert_temp(id, param.unmodulated_normalized_value()));
    }
    if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let delta = -resp.drag_delta().y * if fine { 0.0008 } else { 0.005 };
        let v: f32 = ui.memory(|m| m.data.get_temp(id)).unwrap_or_else(|| param.unmodulated_normalized_value());
        let v = (v + delta).clamp(0.0, 1.0);
        ui.memory_mut(|m| m.data.insert_temp(id, v));
        setter.set_parameter_normalized(param, v);
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
    }
}

/// A rotary knob: drag up and down, Shift for fine, double-click for the default.
fn knob<P: Param>(ui: &mut egui::Ui, center: Pos2, label: &str, param: &P, setter: &ParamSetter, color: Color32) {
    let r = 19.0;
    let rect = Rect::from_center_size(center, vec2(64.0, 76.0));
    let id = ui.id().with(("knob", label));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    drag_param(ui, &resp, param, setter, id.with("v"));
    let v = param.unmodulated_normalized_value();
    let p = ui.painter();
    let c = center + vec2(0.0, -4.0);
    let ang = |u: f32| (135.0 + 270.0 * u).to_radians();
    let arc = |u0: f32, u1: f32| -> Vec<Pos2> {
        let n = ((u1 - u0) * 40.0).ceil().max(1.0) as usize;
        (0..=n).map(|i| {
            let a = ang(u0 + (u1 - u0) * i as f32 / n as f32);
            pos2(c.x + r * a.cos(), c.y + r * a.sin())
        }).collect()
    };
    let hot = resp.hovered() || resp.dragged();
    p.add(Shape::line(arc(0.0, 1.0), Stroke::new(2.0_f32, FRAME)));
    if v > 0.001 {
        beam(p, arc(0.0, v), color, 2.2, if hot { 1.0 } else { 0.85 }, false);
    }
    let a = ang(v);
    p.line_segment([pos2(c.x + 7.0 * a.cos(), c.y + 7.0 * a.sin()), pos2(c.x + (r - 4.0) * a.cos(), c.y + (r - 4.0) * a.sin())], Stroke::new(2.0_f32, if hot { WHITE } else { color }));
    p.text(pos2(center.x, rect.top() + 1.0), Align2::CENTER_TOP, label, vfont(10.5), if hot { TEXT } else { BLUE });
    let value = param.normalized_value_to_string(v, true);
    p.text(pos2(center.x, rect.bottom() - 1.0), Align2::CENTER_BOTTOM, value, FontId::proportional(10.5), alpha(TEXT, if hot { 1.0 } else { 0.7 }));
}

/// A pill that is on or off.
fn toggle(ui: &mut egui::Ui, rect: Rect, label: &str, on: bool, color: Color32) -> bool {
    let resp = ui.interact(rect, ui.id().with(("toggle", label)), Sense::click());
    let p = ui.painter();
    let hot = resp.hovered();
    p.rect_filled(rect, rect.height() / 2.0, if on { alpha(color, 0.2) } else { BG });
    p.rect_stroke(rect, rect.height() / 2.0, Stroke::new(1.2_f32, if on { color } else if hot { BLUE_MID } else { FRAME }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, label, vfont(11.0), if on { color } else if hot { TEXT } else { BLUE_MID });
    resp.clicked()
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

fn note_name(hz: f32) -> String {
    let m = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
    format!("{}{}", NOTES[m.rem_euclid(12) as usize], m.div_euclid(12) - 1)
}

/// What the pad calls each vowel: plain spellings, not phonetic symbols.
fn vowel_label(ipa: &str) -> &'static str {
    match ipa {
        "i" => "ee",
        "ɪ" => "ih",
        "e" => "ay",
        "ɛ" => "eh",
        "æ" => "a",
        "a" => "aa",
        "ɑ" => "ah",
        "ɔ" => "aw",
        "o" => "oh",
        "ʊ" => "oo(k)",
        "u" => "oo",
        "ʌ" => "uh",
        "ə" => "er",
        _ => "",
    }
}

// ---------------------------------------------------------------------- the editor

struct State {
    head: Head,
    lyrics: String,
    chips: Vec<String>,
    chips_version: i64,
    last_cursor: i64,
    level: f32,
}

pub fn create(params: Arc<ChimeraParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let state = State { head: Head::new(BodyMix::MALE), lyrics: params.lyrics.read().clone(), chips: shared.chips.lock().clone(), chips_version: -1, last_cursor: -1, level: 0.0 };
    create_egui_editor(
        params.editor_state.clone(),
        state,
        |ctx, _| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("hershey".into(), Arc::new(egui::FontData::from_static(HERSHEY)));
            let mut stack = vec!["hershey".to_string()];
            stack.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
            fonts.families.insert(FontFamily::Name("hershey".into()), stack);
            ctx.set_fonts(fonts);
            let mut v = egui::Visuals::dark();
            v.panel_fill = BG;
            v.extreme_bg_color = BG;
            v.selection.bg_fill = alpha(BLUE, 0.35);
            v.selection.stroke = Stroke::new(1.0_f32, BLUE);
            v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FRAME);
            v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BLUE_MID);
            v.override_text_color = Some(TEXT);
            ctx.set_visuals(v);
        },
        move |ctx, setter, st| {
            ctx.request_repaint();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(BG)).show(ctx, |ui| draw(ui, &params, &shared, setter, st));
        },
    )
}

fn draw(ui: &mut egui::Ui, params: &ChimeraParams, shared: &Shared, setter: &ParamSetter, st: &mut State) {
    let full = ui.max_rect();
    let m = 12.0;

    // ---- the voice, as the audio thread last saw it
    let (artic, body, f0, voicing, level, scope, active) = {
        let v = shared.viz.lock();
        (v.artic, v.body, v.f0, v.voicing, v.level, v.scope, v.active.clone())
    };
    st.level += (level - st.level) * 0.3;
    if st.head.body != body {
        st.head = Head::new(body);
    }
    let version = shared.chips_version.load(Ordering::Relaxed);
    if version != st.chips_version {
        // a preset or project was loaded: show its lyrics
        st.chips_version = version;
        st.lyrics = params.lyrics.read().clone();
        st.chips = shared.chips.lock().clone();
    }
    let mode = params.mode.value();

    // ---- top bar
    let top = Rect::from_min_size(full.min + vec2(m, m), vec2(full.width() - 2.0 * m, 40.0));
    {
        let p = ui.painter();
        p.text(top.left_center(), Align2::LEFT_CENTER, "CHIMERA", vfont(24.0), BLUE);
        p.text(top.left_center() + vec2(98.0, 4.0), Align2::LEFT_CENTER, "VOICE", vfont(13.0), AMBER);
    }
    let seg = Rect::from_min_size(pos2(top.left() + 330.0, top.top() + 6.0), vec2(130.0, 28.0));
    if toggle(ui, seg, "SING LYRICS", mode == ModeParam::Sing, GREEN) {
        set(setter, &params.mode, ModeParam::Sing);
    }
    if toggle(ui, seg.translate(vec2(138.0, 0.0)), "VOWEL PAD", mode == ModeParam::Vowel, PINK) {
        set(setter, &params.mode, ModeParam::Vowel);
    }
    if toggle(ui, Rect::from_min_size(pos2(top.right() - 84.0, top.top() + 6.0), vec2(84.0, 28.0)), "SILENCE", false, RED) {
        shared.panic.store(true, Ordering::Release);
    }
    // half the processor time for a slightly duller voice
    if toggle(ui, Rect::from_min_size(pos2(top.right() - 150.0, top.top() + 6.0), vec2(58.0, 28.0)), "ECO", params.eco.value(), AMBER) {
        set(setter, &params.eco, !params.eco.value());
    }
    {
        let p = ui.painter();
        let singing = voicing > 0.2 && st.level > 0.003;
        let pitch = if singing { format!("{}  {:.0} Hz", note_name(f0), f0) } else { "--".to_string() };
        p.text(pos2(top.right() - 166.0, top.center().y), Align2::RIGHT_CENTER, pitch, vfont(15.0), if singing { AMBER } else { BLUE_DIM });
    }

    // ---- left: the head, the body, the octave
    let body_h = 108.0;
    let left = Rect::from_min_max(pos2(full.left() + m, top.bottom() + 8.0), pos2(full.left() + m + 318.0, full.bottom() - m));
    let head_rect = Rect::from_min_max(left.min, pos2(left.right(), left.bottom() - body_h - 8.0));
    panel(ui.painter(), head_rect, "");
    st.head.draw(&ui.painter().with_clip_rect(head_rect.shrink(2.0)), head_rect, &artic, st.level);
    if !active.is_empty() {
        ui.painter().text(head_rect.right_bottom() + vec2(-12.0, -10.0), Align2::RIGHT_BOTTOM, &active, FontId::proportional(13.0), alpha(AMBER, 0.9));
    }

    let body_rect = Rect::from_min_max(pos2(left.left(), left.bottom() - body_h), left.max);
    panel(ui.painter(), body_rect, "BODY");
    {
        // the body slider: man, woman, child and everything between
        let track = Rect::from_min_max(pos2(body_rect.left() + 26.0, body_rect.top() + 34.0), pos2(body_rect.right() - 26.0, body_rect.top() + 58.0));
        let id = ui.id().with("body");
        let resp = ui.interact(track.expand2(vec2(14.0, 6.0)), id, Sense::click_and_drag());
        if resp.drag_started() {
            setter.begin_set_parameter(&params.body);
        }
        if resp.dragged() || resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let mut u = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
                // the three bodies the voice was calibrated on catch the handle
                for snap in [0.0, 0.5, 1.0] {
                    if (u - snap).abs() < 0.035 {
                        u = snap;
                    }
                }
                if resp.clicked() {
                    setter.begin_set_parameter(&params.body);
                }
                setter.set_parameter_normalized(&params.body, u);
                if resp.clicked() {
                    setter.end_set_parameter(&params.body);
                }
            }
        }
        if resp.drag_stopped() {
            setter.end_set_parameter(&params.body);
        }
        let u = params.body.unmodulated_normalized_value();
        let p = ui.painter();
        let y = track.center().y;
        p.line_segment([pos2(track.left(), y), pos2(track.right(), y)], Stroke::new(2.0_f32, FRAME));
        for (snap, name) in [(0.0, "MAN"), (0.5, "WOMAN"), (1.0, "CHILD")] {
            let x = track.left() + track.width() * snap;
            p.line_segment([pos2(x, y - 5.0), pos2(x, y + 5.0)], Stroke::new(1.5_f32, BLUE_DIM));
            p.text(pos2(x, y + 11.0), Align2::CENTER_TOP, name, vfont(10.5), if (u - snap).abs() < 0.02 { TEXT } else { BLUE_MID });
        }
        let x = track.left() + track.width() * u;
        p.circle_filled(pos2(x, y), 7.0, BG);
        p.circle_stroke(pos2(x, y), 7.0, Stroke::new(2.0_f32, if resp.hovered() || resp.dragged() { WHITE } else { BLUE }));
        p.circle_stroke(pos2(x, y), 11.0, Stroke::new(3.0_f32, alpha(BLUE, 0.15)));

        // octave
        let oy = body_rect.bottom() - 18.0;
        p.text(pos2(body_rect.left() + 12.0, oy), Align2::LEFT_CENTER, "OCTAVE", vfont(11.0), BLUE_MID);
        let oct = params.octave.value();
        p.text(pos2(body_rect.left() + 128.0, oy), Align2::CENTER_CENTER, format!("{:+}", oct), vfont(14.0), TEXT);
        let minus = Rect::from_center_size(pos2(body_rect.left() + 96.0, oy), vec2(24.0, 22.0));
        let plus = Rect::from_center_size(pos2(body_rect.left() + 160.0, oy), vec2(24.0, 22.0));
        if toggle(ui, minus, "-", false, BLUE) && oct > -3 {
            set(setter, &params.octave, oct - 1);
        }
        if toggle(ui, plus, "+", false, BLUE) && oct < 3 {
            set(setter, &params.octave, oct + 1);
        }
        let wr = Rect::from_min_size(pos2(body_rect.right() - 104.0, oy - 12.0), vec2(92.0, 24.0));
        if toggle(ui, wr, "WHISPER", params.whisper.value(), WHITE) {
            set(setter, &params.whisper, !params.whisper.value());
        }
    }

    // ---- right: the words or the pad, the scope, the knobs
    let right = Rect::from_min_max(pos2(left.right() + 10.0, left.top()), pos2(full.right() - m, full.bottom() - m));
    let knobs_h = 214.0;
    let scope_h = 56.0;
    let main = Rect::from_min_max(right.min, pos2(right.right(), right.bottom() - knobs_h - scope_h - 16.0));
    let scope_rect = Rect::from_min_size(pos2(right.left(), main.bottom() + 8.0), vec2(right.width(), scope_h));
    let knob_rect = Rect::from_min_max(pos2(right.left(), scope_rect.bottom() + 8.0), right.max);

    match mode {
        ModeParam::Sing => draw_lyrics(ui, main, params, shared, setter, st),
        ModeParam::Vowel => draw_pad(ui, main, params, setter),
    }

    // scope: the sound itself, in amber
    panel(ui.painter(), scope_rect, "");
    {
        let p = ui.painter().with_clip_rect(scope_rect.shrink(1.0));
        let inner = scope_rect.shrink2(vec2(10.0, 6.0));
        let pts: Vec<Pos2> = (0..SCOPE_LEN).map(|i| pos2(inner.left() + inner.width() * i as f32 / (SCOPE_LEN - 1) as f32, inner.center().y - scope[i].clamp(-1.0, 1.0) * inner.height() * 0.62)).collect();
        beam(&p, pts, AMBER, 1.3, 0.95, false);
    }

    // knobs
    panel(ui.painter(), knob_rect, "");
    {
        let rows = [knob_rect.top() + 62.0, knob_rect.top() + 160.0];
        let col = |i: usize| if i == 7 { knob_rect.right() - 54.0 } else { knob_rect.left() + 46.0 + 76.0 * i as f32 };
        let p = ui.painter();
        p.text(pos2(knob_rect.left() + 12.0, knob_rect.top() + 9.0), Align2::LEFT_TOP, "VOICE", vfont(11.0), BLUE_MID);
        p.text(pos2(knob_rect.left() + 12.0, knob_rect.top() + 107.0), Align2::LEFT_TOP, "PITCH", vfont(11.0), BLUE_MID);
        p.line_segment([pos2(knob_rect.left() + 10.0, knob_rect.top() + 102.0), pos2(knob_rect.right() - 10.0, knob_rect.top() + 102.0)], Stroke::new(1.0_f32, FRAME));
        knob(ui, pos2(col(0), rows[0]), "BREATH", &params.breath, setter, AMBER);
        knob(ui, pos2(col(1), rows[0]), "PRESSED", &params.pressed, setter, GREEN);
        knob(ui, pos2(col(2), rows[0]), "BRIGHT", &params.bright, setter, GREEN);
        knob(ui, pos2(col(3), rows[0]), "BREATHY", &params.breathy, setter, WHITE);
        knob(ui, pos2(col(4), rows[0]), "ROUGH", &params.rough, setter, RED);
        knob(ui, pos2(col(5), rows[0]), "FRY", &params.fry, setter, RED);
        knob(ui, pos2(col(7), rows[0]), "VOLUME", &params.gain, setter, AMBER);
        knob(ui, pos2(col(0), rows[1]), "VIBRATO", &params.vibrato, setter, PINK);
        knob(ui, pos2(col(1), rows[1]), "RATE", &params.vibrato_rate, setter, PINK);
        knob(ui, pos2(col(2), rows[1]), "GLIDE", &params.glide, setter, BLUE);
        knob(ui, pos2(col(3), rows[1]), "DRIFT", &params.drift, setter, BLUE);
        knob(ui, pos2(col(4), rows[1]), "BEND", &params.bend_range, setter, BLUE);
        // level
        let meter = Rect::from_min_size(pos2(col(7) - 30.0, rows[1] - 6.0), vec2(60.0, 8.0));
        let p = ui.painter();
        p.text(pos2(col(7), rows[1] - 22.0), Align2::CENTER_CENTER, "LEVEL", vfont(10.5), BLUE);
        p.rect_stroke(meter, 3.0, Stroke::new(1.0_f32, FRAME), StrokeKind::Inside);
        let db = 20.0 * (st.level.max(1e-5)).log10();
        let u = ((db + 50.0) / 50.0).clamp(0.0, 1.0);
        p.rect_filled(Rect::from_min_size(meter.min, vec2(meter.width() * u, meter.height())).shrink(1.5), 2.0, AMBER);
    }
}

fn draw_lyrics(ui: &mut egui::Ui, rect: Rect, params: &ChimeraParams, shared: &Shared, setter: &ParamSetter, st: &mut State) {
    panel(ui.painter(), rect, "LYRICS  -  each note sings the next syllable");
    let edit_rect = Rect::from_min_max(rect.min + vec2(10.0, 30.0), pos2(rect.right() - 10.0, rect.top() + 30.0 + (rect.height() - 76.0) * 0.48));
    let chips_rect = Rect::from_min_max(pos2(rect.left() + 10.0, edit_rect.bottom() + 8.0), pos2(rect.right() - 10.0, rect.bottom() - 38.0));

    ui.painter().rect_filled(edit_rect, 4.0, BG);
    ui.painter().rect_stroke(edit_rect, 4.0, Stroke::new(1.0_f32, FRAME), StrokeKind::Inside);
    let changed = ui
        .allocate_new_ui(egui::UiBuilder::new().max_rect(edit_rect.shrink(4.0)), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("lyrics-edit")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut st.lyrics)
                            .frame(false)
                            .desired_width(f32::INFINITY)
                            .desired_rows(4)
                            .font(FontId::proportional(15.0))
                            .hint_text("Type the words to sing. A lone - holds the vowel onto the next note."),
                    )
                    .changed()
                })
                .inner
        })
        .inner;
    if changed {
        *params.lyrics.write() = st.lyrics.clone();
        let syllables = lyrics::parse(&st.lyrics);
        shared.post_lyrics(syllables);
        st.chips = shared.chips.lock().clone();
    }

    // the syllables: amber is sounding, green sings next; click one to start from it
    let cursor = shared.cursor.load(Ordering::Relaxed);
    let sounding = shared.sounding.load(Ordering::Relaxed);
    let moved = cursor != st.last_cursor;
    st.last_cursor = cursor;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(chips_rect), |ui| {
        egui::ScrollArea::vertical().id_salt("chips").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 5.0);
                for (i, chip) in st.chips.iter().enumerate() {
                    let i = i as i64;
                    let galley = ui.painter().layout_no_wrap(chip.clone(), FontId::proportional(14.0), TEXT);
                    let (r, resp) = ui.allocate_exact_size(galley.size() + vec2(14.0, 8.0), Sense::click());
                    let (fill, line, text) = if i == sounding {
                        (alpha(AMBER, 0.25), AMBER, AMBER)
                    } else if i == cursor {
                        (alpha(GREEN, 0.16), GREEN, GREEN)
                    } else if resp.hovered() {
                        (BG, BLUE_MID, TEXT)
                    } else {
                        (BG, FRAME, alpha(TEXT, 0.75))
                    };
                    ui.painter().rect_filled(r, 4.0, fill);
                    ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0_f32, line), StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, chip, FontId::proportional(14.0), text);
                    if resp.clicked() {
                        shared.cursor_request.store(i, Ordering::Release);
                    }
                    if moved && i == cursor {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            });
        });
    });

    let by = rect.bottom() - 19.0;
    if toggle(ui, Rect::from_min_size(pos2(rect.left() + 10.0, by - 12.0), vec2(110.0, 24.0)), "FROM THE TOP", false, GREEN) {
        shared.cursor_request.store(0, Ordering::Release);
    }
    let on = params.restart_on_play.value();
    if toggle(ui, Rect::from_min_size(pos2(rect.left() + 128.0, by - 12.0), vec2(190.0, 24.0)), "RESTART WHEN PLAY STARTS", on, GREEN) {
        set(setter, &params.restart_on_play, !on);
    }
    ui.painter().text(pos2(rect.right() - 12.0, by), Align2::RIGHT_CENTER, format!("{} syllables", st.chips.len()), vfont(11.0), BLUE_DIM);
}

fn draw_pad(ui: &mut egui::Ui, rect: Rect, params: &ChimeraParams, setter: &ParamSetter) {
    panel(ui.painter(), rect, "VOWEL  -  drag to shape the mouth while you play");
    let pad = Rect::from_min_max(rect.min + vec2(44.0, 44.0), rect.max - vec2(44.0, 30.0));
    let resp = ui.interact(pad.expand(16.0), ui.id().with("vowel-pad"), Sense::click_and_drag());
    if resp.drag_started() {
        setter.begin_set_parameter(&params.vowel_x);
        setter.begin_set_parameter(&params.vowel_y);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let x = ((pos.x - pad.left()) / pad.width()).clamp(0.0, 1.0);
            let y = ((pos.y - pad.top()) / pad.height()).clamp(0.0, 1.0);
            if resp.clicked() {
                setter.begin_set_parameter(&params.vowel_x);
                setter.begin_set_parameter(&params.vowel_y);
            }
            setter.set_parameter(&params.vowel_x, x);
            setter.set_parameter(&params.vowel_y, y);
            if resp.clicked() {
                setter.end_set_parameter(&params.vowel_x);
                setter.end_set_parameter(&params.vowel_y);
            }
        }
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(&params.vowel_x);
        setter.end_set_parameter(&params.vowel_y);
    }
    let (x, y) = (params.vowel_x.value(), params.vowel_y.value());
    let at = |vx: f32, vy: f32| pos2(pad.left() + pad.width() * vx, pad.top() + pad.height() * vy);
    let p = ui.painter();
    // the vowel chart's outline, like a border on a map
    beam(p, vec![at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.3, 1.0)], BLUE_DIM, 1.0, 0.9, true);
    p.text(pos2(pad.left(), rect.bottom() - 9.0), Align2::LEFT_BOTTOM, "FRONT", vfont(10.0), BLUE_DIM);
    p.text(pos2(pad.right(), rect.bottom() - 9.0), Align2::RIGHT_BOTTOM, "BACK", vfont(10.0), BLUE_DIM);
    p.text(pos2(rect.left() + 10.0, pad.top()), Align2::LEFT_CENTER, "SHUT", vfont(10.0), BLUE_DIM);
    p.text(pos2(rect.left() + 10.0, pad.bottom()), Align2::LEFT_CENTER, "OPEN", vfont(10.0), BLUE_DIM);
    let near = nearest_pad_vowel(x as f64, y as f64);
    for ipa in PAD_VOWELS {
        let (vx, vy) = pad_position(ipa);
        let c = at(vx as f32, vy as f32);
        let on = ipa == near;
        p.circle_stroke(c, 3.0, Stroke::new(1.2_f32, if on { PINK } else { BLUE_DIM }));
        p.text(c + vec2(0.0, -8.0), Align2::CENTER_BOTTOM, vowel_label(ipa), vfont(if on { 15.0 } else { 13.0 }), if on { PINK } else { alpha(TEXT, 0.8) });
    }
    let h = at(x, y);
    let hot = resp.hovered() || resp.dragged();
    p.circle_stroke(h, 15.0, Stroke::new(5.0_f32, alpha(PINK, 0.15)));
    p.circle_stroke(h, 10.0, Stroke::new(2.0_f32, if hot { WHITE } else { PINK }));
    p.circle_filled(h, 2.5, PINK);
}
