//! CPU text baking — rasterizes short strings (including Japanese kana/kanji) to
//! RGBA `Image`s with `fontdue`, then exposes them as unlit `StandardMaterial`s for
//! mapping onto world-space quads. This replaces `bevy_rich_text3d`, which has no
//! Bevy 0.19 release.
//!
//! All baking is cached and meant to run up-front (see the pre-warm system in
//! `main.rs`), so no new glyphs are rasterized/uploaded mid-run. That also fixes
//! the mobile gate-freeze documented in NOTES.md.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use fontdue::{Font, FontSettings};
use std::collections::HashMap;

const FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/TakaoPGothic.ttf");

/// Pixel height used to rasterize glyphs. Higher = crisper text on the quads.
const RASTER_PX: f32 = 110.0;

/// Which on-gate role a string plays — selects fill color and outline.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextKind {
    /// The question on the crossbeam: dark ink with a light outline so it reads
    /// against the pale beam.
    Question,
    /// The answer on a sign: dark ink, no outline (sits on a colored sign).
    Answer,
}

/// A baked string: a ready-to-use material plus the texture's width/height ratio
/// so callers can size the quad without distorting the glyphs.
#[derive(Clone)]
pub struct Baked {
    pub material: Handle<StandardMaterial>,
    pub aspect: f32,
}

#[derive(Resource)]
pub struct TextBaker {
    font: Font,
    cache: HashMap<(String, TextKind), Baked>,
}

impl TextBaker {
    pub fn new() -> Self {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default())
            .expect("embedded TakaoPGothic font should parse");
        Self { font, cache: HashMap::new() }
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Get-or-bake: rasterizes `text` for `kind` if not already cached, inserting a
    /// new `Image` + `StandardMaterial`, and returns the cached handle.
    pub fn bake(
        &mut self,
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
        text: &str,
        kind: TextKind,
    ) -> Baked {
        let key = (text.to_owned(), kind);
        if let Some(b) = self.cache.get(&key) {
            return b.clone();
        }

        let (fill, stroke) = match kind {
            TextKind::Question => (
                Srgba::new(0.12, 0.06, 0.01, 1.0),
                Some(((RASTER_PX * 0.05).round() as usize, Srgba::new(1.0, 0.95, 0.8, 1.0))),
            ),
            TextKind::Answer => (Srgba::new(0.05, 0.05, 0.1, 1.0), None),
        };

        let (data, w, h) = rasterize(&self.font, text, RASTER_PX, fill, stroke);
        let image = Image::new(
            Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        let material = materials.add(StandardMaterial {
            base_color_texture: Some(images.add(image)),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            cull_mode: None,
            ..default()
        });

        let baked = Baked { material, aspect: w as f32 / h as f32 };
        self.cache.insert(key, baked.clone());
        baked
    }
}

impl Default for TextBaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Rasterizes a single line of text to a tightly-cropped (plus padding) RGBA buffer.
/// Returns `(rgba, width, height)`. Glyphs are laid out left-to-right on one
/// baseline — sufficient for the short, unshaped strings the game shows.
fn rasterize(
    font: &Font,
    text: &str,
    px: f32,
    fill: Srgba,
    stroke: Option<(usize, Srgba)>,
) -> (Vec<u8>, usize, usize) {
    let stroke_r = stroke.map(|(r, _)| r).unwrap_or(0);
    let pad = stroke_r + 2;

    // Measure: rasterize each glyph, track pen advance and vertical extent.
    let mut glyphs = Vec::new();
    let mut pen = 0.0_f32;
    let mut max_top = 1_i32; // pixels above baseline
    let mut min_bot = 0_i32; // pixels below baseline (negative)
    for ch in text.chars() {
        let (m, bitmap) = font.rasterize(ch, px);
        max_top = max_top.max(m.ymin + m.height as i32);
        min_bot = min_bot.min(m.ymin);
        glyphs.push((pen + m.xmin as f32, m.ymin, m.width, m.height, bitmap));
        pen += m.advance_width;
    }

    let asc = max_top.max(1);
    let desc = (-min_bot).max(0);
    let text_w = pen.ceil().max(1.0) as usize;
    let text_h = (asc + desc).max(1) as usize;
    let w = text_w + pad * 2;
    let h = text_h + pad * 2;

    // Composite glyph coverage (max-combine) into a grayscale buffer.
    let mut cov = vec![0u8; w * h];
    for (left, ymin, gw, gh, bitmap) in &glyphs {
        let x0 = left.round() as i32 + pad as i32;
        let y0 = (asc - (ymin + *gh as i32)) + pad as i32;
        for gy in 0..*gh {
            for gx in 0..*gw {
                let c = bitmap[gy * gw + gx];
                if c == 0 {
                    continue;
                }
                let dx = x0 + gx as i32;
                let dy = y0 + gy as i32;
                if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 {
                    continue;
                }
                let idx = dy as usize * w + dx as usize;
                if c > cov[idx] {
                    cov[idx] = c;
                }
            }
        }
    }

    let stroke_cov = stroke.map(|(r, _)| dilate(&cov, w, h, r));

    // Compose RGBA: optional stroke underneath, fill "over" it.
    let f = [s8(fill.red), s8(fill.green), s8(fill.blue)];
    let s = stroke.map(|(_, c)| [s8(c.red), s8(c.green), s8(c.blue)]).unwrap_or([0, 0, 0]);
    let mut data = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let fa = cov[i] as f32 / 255.0;
        let sa = stroke_cov.as_ref().map(|b| b[i] as f32 / 255.0).unwrap_or(0.0);
        let oa = fa + sa * (1.0 - fa);
        let o = i * 4;
        if oa <= 0.0 {
            continue;
        }
        for ch in 0..3 {
            let over = (f[ch] as f32 * fa + s[ch] as f32 * sa * (1.0 - fa)) / oa;
            data[o + ch] = over.clamp(0.0, 255.0) as u8;
        }
        data[o + 3] = (oa * 255.0).clamp(0.0, 255.0) as u8;
    }

    (data, w, h)
}

/// Separable square max-filter — grows opaque regions by `r` px to form an outline.
fn dilate(cov: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    if r == 0 {
        return cov.to_vec();
    }
    let mut tmp = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let x0 = x.saturating_sub(r);
            let x1 = (x + r).min(w - 1);
            let mut m = 0u8;
            for xx in x0..=x1 {
                m = m.max(cov[y * w + xx]);
            }
            tmp[y * w + x] = m;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(r);
        let y1 = (y + r).min(h - 1);
        for x in 0..w {
            let mut m = 0u8;
            for yy in y0..=y1 {
                m = m.max(tmp[yy * w + x]);
            }
            out[y * w + x] = m;
        }
    }
    out
}

/// sRGB component (0..1) → byte, for storage in an `Rgba8UnormSrgb` texture.
fn s8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}
