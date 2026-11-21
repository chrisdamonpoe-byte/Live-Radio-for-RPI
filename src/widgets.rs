//! Colours, procedural artwork and hand-drawn icons.
//!
//! Station art is the station's own logo, fetched from the public image
//! service at run time and cached on disk (nothing is shipped in the package);
//! a tile generated from the station id stands in until it loads. Icons are
//! painted with primitives, keeping the binary free of image or font assets.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, ColorImage, FontId, Painter, Pos2, Rect, Response, Sense,
    Shape, Stroke, TextureHandle, Ui, Vec2,
};

use crate::api::Station;

pub const BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
pub const CARD: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
pub const CARD_HI: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
pub const TRACK: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
pub const TEXT: Color32 = Color32::from_rgb(0xf7, 0xf7, 0xf7);
pub const MUTED: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);
pub const FAINT: Color32 = Color32::from_rgb(0x78, 0x78, 0x78);
pub const ACCENT: Color32 = Color32::from_rgb(0xc6, 0x00, 0x2b);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xe4, 0x1a, 0x44);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x3c, 0x12, 0x1a);
pub const LIVE: Color32 = Color32::from_rgb(0xff, 0x3b, 0x4e);

pub fn white_alpha(a: u8) -> Color32 {
    Color32::from_white_alpha(a)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub a: Color32,
    pub b: Color32,
}

impl Palette {
    const fn new(a: u32, b: u32) -> Self {
        Self {
            a: rgb(a),
            b: rgb(b),
        }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const PALETTES: &[Palette] = &[
    Palette::new(0x8e0e2c, 0x3b0716),
    Palette::new(0xb4530a, 0x5c1a06),
    Palette::new(0x0f766e, 0x083c3a),
    Palette::new(0x1d4ed8, 0x111a4a),
    Palette::new(0x6d28d9, 0x25104d),
    Palette::new(0xbe185d, 0x4a0b2c),
    Palette::new(0x0e7490, 0x083344),
    Palette::new(0x4d7c0f, 0x1a2e05),
    Palette::new(0x9a3412, 0x431407),
    Palette::new(0x334155, 0x0f172a),
    Palette::new(0x7c2d12, 0x2a0d05),
    Palette::new(0x1e3a8a, 0x0b1220),
    Palette::new(0xa16207, 0x422006),
    Palette::new(0x86198f, 0x300533),
];

pub fn palette_for(id: u64, name: &str) -> Palette {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= id.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    PALETTES[(h % PALETTES.len() as u64) as usize]
}

/// Station artwork. A station's real logo is downloaded in the background
/// (and kept on disk); until it arrives, or when there is none, a procedural
/// 256px tile is shown instead: a two-colour gradient, a seeded glow and
/// faint rings, with rounded, transparent corners.
pub struct ArtCache {
    textures: HashMap<u64, TextureHandle>,
    logos: HashMap<u64, Logo>,
    jobs: Option<Sender<(u64, String)>>,
    done_tx: Sender<(u64, Option<ColorImage>)>,
    done_rx: Receiver<(u64, Option<ColorImage>)>,
}

enum Logo {
    Pending,
    Ready(TextureHandle),
    Failed,
}

const LOGO_WORKERS: usize = 4;

impl Default for ArtCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtCache {
    pub fn new() -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        Self {
            textures: HashMap::new(),
            logos: HashMap::new(),
            jobs: None,
            done_tx,
            done_rx,
        }
    }

    /// Texture for a station, and whether it is the station's real logo.
    pub fn art_for(&mut self, ctx: &egui::Context, station: &Station) -> (egui::TextureId, bool) {
        self.collect(ctx);
        if !station.logo.is_empty() {
            match self.logos.get(&station.id) {
                Some(Logo::Ready(texture)) => return (texture.id(), true),
                Some(Logo::Pending) | Some(Logo::Failed) => {}
                None => self.request(ctx, station.id, &station.logo),
            }
        }
        let palette = palette_for(station.id, &station.name);
        (self.id_of(ctx, station.id, palette), false)
    }

    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok((key, image)) = self.done_rx.try_recv() {
            let state = match image {
                Some(image) => Logo::Ready(ctx.load_texture(
                    format!("logo-{key:016x}"),
                    image,
                    egui::TextureOptions::LINEAR,
                )),
                None => Logo::Failed,
            };
            self.logos.insert(key, state);
        }
    }

    fn request(&mut self, ctx: &egui::Context, key: u64, url: &str) {
        if self.jobs.is_none() {
            let (tx, rx) = mpsc::channel::<(u64, String)>();
            let rx = Arc::new(Mutex::new(rx));
            for _ in 0..LOGO_WORKERS {
                let rx = Arc::clone(&rx);
                let done = self.done_tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || loop {
                    let job = match rx.lock() {
                        Ok(guard) => guard.recv(),
                        Err(_) => break,
                    };
                    let Ok((key, url)) = job else { break };
                    let image = load_logo(key, &url);
                    if done.send((key, image)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                });
            }
            self.jobs = Some(tx);
        }
        if let Some(jobs) = &self.jobs {
            if jobs.send((key, url.to_string())).is_ok() {
                self.logos.insert(key, Logo::Pending);
            }
        }
    }

    pub fn id_of(&mut self, ctx: &egui::Context, key: u64, palette: Palette) -> egui::TextureId {
        if !self.textures.contains_key(&key) {
            let image = render_art(key, palette);
            let texture = ctx.load_texture(
                format!("art-{key:016x}"),
                image,
                egui::TextureOptions::LINEAR,
            );
            self.textures.insert(key, texture);
        }
        self.textures
            .get(&key)
            .map(|t| t.id())
            .unwrap_or(egui::TextureId::default())
    }
}

/// Small stable hash so a changed logo URL gets a new cache file.
fn fnv(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Fetch (or read from the disk cache) and decode one logo into a tile.
fn load_logo(key: u64, url: &str) -> Option<ColorImage> {
    let dir = crate::store::cache_dir().join("logos");
    let path = dir.join(format!("{key}-{:016x}.img", fnv(url)));

    if let Ok(bytes) = std::fs::read(&path) {
        if let Some(tile) = logo_tile(&bytes) {
            return Some(tile);
        }
        let _ = std::fs::remove_file(&path);
    }

    let bytes = crate::api::fetch_image(url).ok()?;
    let tile = logo_tile(&bytes)?;
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(&path, &bytes);
    }
    Some(tile)
}

/// Fit a logo onto a white rounded 256px tile (same shape as the procedural art).
fn logo_tile(bytes: &[u8]) -> Option<ColorImage> {
    const N: u32 = 256;
    const R: f32 = 14.0;
    let src = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let scale = (N as f32 / w as f32).min(N as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).clamp(1, N);
    let nh = ((h as f32 * scale).round() as u32).clamp(1, N);
    let fitted = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::CatmullRom);
    let ox = (N - nw) / 2;
    let oy = (N - nh) / 2;

    let n = N as usize;
    let mut out = ColorImage::new([n, n], Color32::TRANSPARENT);
    let half = N as f32 * 0.5;
    for y in 0..N {
        for x in 0..N {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let qx = (px - half).abs() - (half - R);
            let qy = (py - half).abs() - (half - R);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - R;
            let mask = (0.5 - d).clamp(0.0, 1.0);
            if mask <= 0.0 {
                continue;
            }
            let (mut r, mut g, mut b) = (255.0f32, 255.0f32, 255.0f32);
            if x >= ox && y >= oy && x < ox + nw && y < oy + nh {
                let p = fitted.get_pixel(x - ox, y - oy).0;
                let a = p[3] as f32 / 255.0;
                r = p[0] as f32 * a + r * (1.0 - a);
                g = p[1] as f32 * a + g * (1.0 - a);
                b = p[2] as f32 * a + b * (1.0 - a);
            }
            out.pixels[(y * N + x) as usize] =
                Color32::from_rgba_unmultiplied(r as u8, g as u8, b as u8, (mask * 255.0) as u8);
        }
    }
    Some(out)
}

fn render_art(key: u64, palette: Palette) -> ColorImage {
    const N: usize = 256;
    const R: f32 = 14.0;
    let mut image = ColorImage::new([N, N], Color32::TRANSPARENT);

    let s = (key % 977) as f32;
    let gx = 0.30 + 0.40 * ((s * 1.7).sin().abs());
    let gy = 0.22 + 0.38 * ((s * 2.3).cos().abs());
    let gr = 0.30 + 0.14 * ((s * 3.7).sin().abs());

    for y in 0..N {
        for x in 0..N {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let fx = px / N as f32;
            let fy = py / N as f32;

            let half = N as f32 * 0.5;
            let qx = (px - half).abs() - (half - R);
            let qy = (py - half).abs() - (half - R);
            let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - R;
            let alpha = (0.5 - d).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }

            let t = (fy * 0.80 + fx * 0.20).clamp(0.0, 1.0);
            let mut color = mix(palette.a, palette.b, t);

            let dx = fx - gx;
            let dy = fy - gy;
            let dist = (dx * dx + dy * dy).sqrt();
            let glow = (1.0 - (dist / gr).clamp(0.0, 1.0)).powf(1.8) * 0.22;
            color = mix(color, Color32::WHITE, glow);

            let ring = (dist * 26.0).fract();
            if ring < 0.10 && dist < 0.62 {
                color = mix(color, Color32::WHITE, 0.045);
            }

            let band = ((fx + fy) * 0.5 - 0.55).abs();
            if band < 0.10 {
                color = mix(color, Color32::WHITE, 0.04 * (1.0 - band / 0.10));
            }

            let edge = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt() / 0.71;
            color = mix(color, Color32::BLACK, (edge * 0.24).clamp(0.0, 0.24));

            image.pixels[y * N + x] = Color32::from_rgba_unmultiplied(
                color.r(),
                color.g(),
                color.b(),
                (alpha * 255.0) as u8,
            );
        }
    }
    image
}

pub fn art_image(painter: &Painter, rect: Rect, texture: egui::TextureId) {
    painter.image(
        texture,
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Heart,
    Search,
    Volume,
    Mute,
    Home,
    Radio,
}

pub fn icon(painter: &Painter, rect: Rect, kind: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let w = (s * 0.09).max(1.2);
    let stroke = Stroke::new(w, color);
    let p = |x: f32, y: f32| pos2(c.x + x * s, c.y + y * s);
    match kind {
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![p(-0.13, -0.24), p(0.26, 0.0), p(-0.13, 0.24)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            for x in [-0.14, 0.04] {
                painter.rect_filled(
                    Rect::from_min_size(p(x, -0.22), vec2(0.10 * s, 0.44 * s)),
                    1.5,
                    color,
                );
            }
        }
        Icon::Heart => {
            painter.circle_filled(p(-0.11, -0.07), 0.155 * s, color);
            painter.circle_filled(p(0.11, -0.07), 0.155 * s, color);
            painter.add(Shape::convex_polygon(
                vec![p(-0.255, -0.02), p(0.255, -0.02), p(0.0, 0.25)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Search => {
            painter.circle_stroke(p(-0.06, -0.06), 0.22 * s, stroke);
            painter.line_segment([p(0.11, 0.11), p(0.28, 0.28)], stroke);
        }
        Icon::Volume | Icon::Mute => {
            painter.add(Shape::convex_polygon(
                vec![
                    p(-0.30, -0.09),
                    p(-0.16, -0.09),
                    p(-0.02, -0.24),
                    p(-0.02, 0.24),
                    p(-0.16, 0.09),
                    p(-0.30, 0.09),
                ],
                color,
                Stroke::NONE,
            ));
            if kind == Icon::Mute {
                painter.line_segment([p(0.10, -0.10), p(0.28, 0.10)], stroke);
                painter.line_segment([p(0.28, -0.10), p(0.10, 0.10)], stroke);
            } else {
                painter.add(Shape::line(
                    arc_points(p(0.06, 0.0), 0.16 * s, -0.9, 0.9, 10),
                    stroke,
                ));
                painter.add(Shape::line(
                    arc_points(p(0.02, 0.0), 0.28 * s, -0.9, 0.9, 14),
                    stroke,
                ));
            }
        }
        Icon::Home => {
            painter.add(Shape::convex_polygon(
                vec![p(-0.30, 0.02), p(0.0, -0.30), p(0.30, 0.02)],
                color,
                Stroke::NONE,
            ));
            painter.rect_filled(
                Rect::from_min_size(p(-0.21, 0.02), vec2(0.42 * s, 0.26 * s)),
                2.0,
                color,
            );
        }
        Icon::Radio => {
            painter.rect_stroke(
                Rect::from_min_size(p(-0.30, -0.16), vec2(0.60 * s, 0.38 * s)),
                3.0,
                stroke,
            );
            painter.circle_stroke(p(0.0, 0.03), 0.10 * s, stroke);
            painter.line_segment([p(-0.22, -0.26), p(-0.08, -0.16)], stroke);
        }
    }
}

fn arc_points(center: Pos2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pos2> {
    let mut out = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let a = a0 + (a1 - a0) * t;
        out.push(center + vec2(a.cos(), a.sin()) * r);
    }
    out
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

pub fn chip(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), TEXT);
    let size = vec2(galley.size().x + 28.0, 31.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
    let fill = if selected {
        ACCENT
    } else if hover > 0.0 {
        lerp_color(CARD, CARD_HI, hover)
    } else {
        CARD
    };
    ui.painter().rect_filled(rect, 16.0, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        font,
        if selected {
            Color32::WHITE
        } else if resp.hovered() {
            TEXT
        } else {
            MUTED
        },
    );
    resp
}

pub fn section_header(ui: &mut Ui, title: &str, action: Option<&str>) -> Option<Response> {
    let mut action_resp = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(18.0)
                .color(TEXT)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(text) = action {
                let resp = ui.add(
                    egui::Button::new(egui::RichText::new(text).size(12.0).color(MUTED))
                        .frame(false),
                );
                action_resp = Some(resp);
            }
        });
    });
    ui.add_space(10.0);
    action_resp
}

pub fn elide(text: &str, width: f32, size: f32) -> String {
    let per_char = size * 0.54;
    let max = (width / per_char).floor().max(3.0) as usize;
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn line(
    painter: &Painter,
    pos: Pos2,
    text: &str,
    size: f32,
    color: Color32,
    max_width: f32,
) {
    let text = elide(text, max_width, size);
    painter.text(
        pos,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}

/// Navigation rail item: icon above a small label.
pub fn nav_item(ui: &mut Ui, kind: Icon, label: &str, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
    if selected {
        ui.painter().rect_filled(rect.shrink2(vec2(6.0, 2.0)), 10.0, ACCENT_SOFT);
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(rect.left() + 6.0, rect.center().y - 12.0),
                vec2(3.0, 24.0),
            ),
            1.5,
            ACCENT,
        );
    } else if hover > 0.0 {
        ui.painter().rect_filled(
            rect.shrink2(vec2(6.0, 2.0)),
            10.0,
            white_alpha((hover * 9.0) as u8),
        );
    }
    let color = if selected {
        ACCENT_HI
    } else if resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 19.0), Vec2::splat(22.0)),
        kind,
        color,
    );
    ui.painter().text(
        pos2(rect.center().x, rect.top() + 40.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.5),
        color,
    );
    resp
}

/// A row in the genre browser.
pub fn genre_row(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
    if selected {
        ui.painter().rect_filled(rect, 6.0, CARD);
        ui.painter().rect_filled(
            Rect::from_min_size(
                pos2(rect.left(), rect.center().y - 9.0),
                vec2(3.0, 18.0),
            ),
            1.5,
            ACCENT,
        );
    } else if hover > 0.0 {
        ui.painter().rect_filled(
            rect,
            6.0,
            white_alpha((hover * 7.0) as u8),
        );
    }
    ui.painter().text(
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        elide(label, width - 24.0, 12.5),
        FontId::proportional(12.5),
        if selected {
            TEXT
        } else if resp.hovered() {
            TEXT
        } else {
            MUTED
        },
    );
    resp
}

pub struct CardHit {
    pub play: bool,
    pub favorite: bool,
}

/// Station tile: generated art, call sign overlay, title and subtitle.
#[allow(clippy::too_many_arguments)]
pub fn station_card(
    ui: &mut Ui,
    cache: &mut ArtCache,
    station: &Station,
    width: f32,
    playing: bool,
    favorite: bool,
) -> CardHit {
    let art_size = width;
    let height = art_size + 50.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let art_rect = Rect::from_min_size(rect.min, vec2(art_size, art_size));
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());

    let (texture, has_logo) = cache.art_for(ui.ctx(), station);
    art_image(ui.painter(), art_rect, texture);

    if hover > 0.0 {
        ui.painter()
            .rect_filled(art_rect, 10.0, white_alpha((hover * 16.0) as u8));
    }

    // Call sign, or the shortest distinctive part of the name.
    let call = if !station.call.is_empty() {
        station.call.clone()
    } else {
        station.name.to_uppercase()
    };
    let call = elide(&call, art_size - 24.0, art_size * 0.16);
    if !has_logo {
        ui.painter().text(
            art_rect.center(),
            Align2::CENTER_CENTER,
            call,
            FontId::proportional((art_size * 0.145).clamp(15.0, 26.0)),
            Color32::from_white_alpha(238),
        );
    }

    // Live pill, top-left.
    let badge = Rect::from_min_size(art_rect.min + vec2(8.0, 8.0), vec2(34.0, 16.0));
    ui.painter().rect_filled(badge, 4.0, Color32::from_black_alpha(140));
    ui.painter().circle_filled(
        pos2(badge.left() + 8.0, badge.center().y),
        2.4,
        LIVE,
    );
    ui.painter().text(
        pos2(badge.left() + 14.0, badge.center().y),
        Align2::LEFT_CENTER,
        "LIVE",
        FontId::proportional(8.5),
        Color32::from_white_alpha(230),
    );

    // Favourite target, top-right.
    let fav_rect = Rect::from_min_size(
        pos2(art_rect.right() - 34.0, art_rect.top() + 6.0),
        vec2(28.0, 28.0),
    );
    let fav_resp = ui.interact(fav_rect, resp.id.with("fav"), Sense::click());
    let fav_hover = ui.ctx().animate_bool(fav_resp.id, fav_resp.hovered());
    if favorite || fav_hover > 0.02 {
        ui.painter().circle_filled(
            fav_rect.center(),
            13.0,
            Color32::from_black_alpha((90.0 + 70.0 * fav_hover) as u8),
        );
        icon(
            ui.painter(),
            fav_rect.shrink(7.0),
            Icon::Heart,
            if favorite { ACCENT_HI } else { TEXT },
        );
    }

    let show = hover > 0.02 || playing;
    if show {
        let alpha = if playing { 1.0 } else { hover };
        let c = pos2(art_rect.right() - 26.0, art_rect.bottom() - 26.0);
        ui.painter().circle_filled(
            c,
            19.0,
            Color32::from_rgba_unmultiplied(
                ACCENT.r(),
                ACCENT.g(),
                ACCENT.b(),
                (alpha * 235.0) as u8,
            ),
        );
        let icon_color = Color32::from_rgba_unmultiplied(255, 255, 255, (alpha * 255.0) as u8);
        icon(
            ui.painter(),
            Rect::from_center_size(c, Vec2::splat(17.0)),
            if playing { Icon::Pause } else { Icon::Play },
            icon_color,
        );
    }

    line(
        ui.painter(),
        pos2(rect.left(), art_rect.bottom() + 15.0),
        &station.name,
        13.5,
        if playing { ACCENT_HI } else { TEXT },
        width,
    );
    line(
        ui.painter(),
        pos2(rect.left(), art_rect.bottom() + 33.0),
        &station.subtitle(),
        11.5,
        MUTED,
        width,
    );

    CardHit {
        play: resp.clicked() && !fav_resp.clicked(),
        favorite: fav_resp.clicked(),
    }
}

/// Compact recent-station tile used on the home page.
pub fn mini_card(
    ui: &mut Ui,
    cache: &mut ArtCache,
    station: &Station,
    width: f32,
    playing: bool,
) -> bool {
    let art = width;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, art + 42.0), Sense::click());
    let art_rect = Rect::from_min_size(rect.min, vec2(art, art));
    let hover = ui.ctx().animate_bool(resp.id, resp.hovered());
    let (texture, _) = cache.art_for(ui.ctx(), station);
    art_image(ui.painter(), art_rect, texture);
    if hover > 0.0 {
        ui.painter()
            .rect_filled(art_rect, 8.0, white_alpha((hover * 18.0) as u8));
    }
    if hover > 0.02 || playing {
        let alpha = if playing { 1.0 } else { hover };
        let center = pos2(art_rect.right() - 21.0, art_rect.bottom() - 21.0);
        ui.painter().circle_filled(
            center,
            16.0,
            Color32::from_rgba_unmultiplied(
                ACCENT.r(),
                ACCENT.g(),
                ACCENT.b(),
                (alpha * 235.0) as u8,
            ),
        );
        icon(
            ui.painter(),
            Rect::from_center_size(center, Vec2::splat(14.0)),
            if playing { Icon::Pause } else { Icon::Play },
            Color32::from_white_alpha((alpha * 255.0) as u8),
        );
    }
    line(
        ui.painter(),
        pos2(rect.left(), art_rect.bottom() + 13.0),
        &station.name,
        12.0,
        if playing { ACCENT_HI } else { TEXT },
        width,
    );
    line(
        ui.painter(),
        pos2(rect.left(), art_rect.bottom() + 29.0),
        &station.call,
        10.5,
        MUTED,
        width,
    );
    resp.clicked()
}

/// Small animated "on air" meter.
pub fn equalizer(painter: &Painter, rect: Rect, t: f64, color: Color32) {
    let n = 3;
    let gap = rect.width() / (n as f32 * 2.0);
    for i in 0..n {
        let phase = t * 6.0 + i as f64 * 1.9;
        let h = 0.35 + 0.55 * (phase.sin() * 0.5 + 0.5) as f32;
        let x = rect.left() + gap + i as f32 * gap * 2.0;
        let bar = Rect::from_min_max(
            pos2(x, rect.bottom() - rect.height() * h),
            pos2(x + gap, rect.bottom()),
        );
        painter.rect_filled(bar, 1.0, color);
    }
}


