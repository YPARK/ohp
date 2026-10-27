//! What a page draws that its text is made of: its glyphs, each with
//! where it sits, its size, its colour and whether it is a formula's or
//! bold, and the rules drawn across and down it.

use super::{BEARING, Mark, RULE, TYPE3};
use hayro::hayro_interpret::font::Glyph;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro::vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::collections::HashMap;

/// The letters of a typographic ligature, which fonts for terminals rarely have.
fn unligate(c: char) -> Option<&'static str> {
    Some(match c {
        'ﬀ' => "ff",
        'ﬁ' => "fi",
        'ﬂ' => "fl",
        'ﬃ' => "ffi",
        'ﬄ' => "ffl",
        _ => return None,
    })
}

/// Whether a font, by its PostScript name, is one formulas are set in.
pub(super) fn math_font(name: &str) -> bool {
    const TEX: [&str; 8] = [
        "CMMI", "CMSY", "CMEX", "CMBSY", "MSAM", "MSBM", "EUFM", "RSFS",
    ];
    name.contains("Math") || TEX.iter().any(|tex| name.starts_with(tex))
}

/// Whether a font, by its PostScript name, is a bold one.
pub(super) fn bold_font(name: &str) -> bool {
    const WEIGHTS: [&str; 4] = ["bold", "black", "heavy", "demi"];
    let lower = name.to_ascii_lowercase();
    name.starts_with("CMBX")
        || name.starts_with("CMB10")
        || WEIGHTS.iter().any(|w| lower.contains(w))
}

/// Whether a character is a formula's: Greek, a mathematical operator or
/// symbol, or a mathematical letter.
pub(super) fn math_char(c: char) -> bool {
    matches!(c,
        '\u{0391}'..='\u{03C9}'
        | '\u{2200}'..='\u{22FF}'
        | '\u{27C0}'..='\u{27EF}'
        | '\u{2980}'..='\u{2AFF}'
        | '\u{1D400}'..='\u{1D7FF}')
}

/// Collects the glyphs a page draws, and the rules; everything else is
/// ignored.
#[derive(Default)]
pub(super) struct Marks {
    pub(super) glyphs: Vec<Mark>,
    pub(super) across: Vec<Rect>,
    pub(super) down: Vec<Rect>,
    /// Whether each font, by its cache key, is a math font and a bold one.
    fonts: HashMap<u128, (bool, bool)>,
}

impl<'a> Device<'a> for Marks {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, path: &BezPath, transform: Affine, _: &Paint<'a>, mode: &PathDrawMode) {
        let mut bbox = transform.transform_rect_bbox(path.bounding_box());
        if let PathDrawMode::Stroke(stroke) = mode {
            let half = f64::from(stroke.line_width) * transform.determinant().abs().sqrt() / 2.;
            bbox = bbox.inflate(half, half);
        }
        let (w, h) = (bbox.width(), bbox.height());
        if h <= RULE && w >= 2. * RULE {
            self.across.push(bbox);
        } else if w <= RULE && h >= 2. * RULE {
            self.down.push(bbox);
        }
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        if matches!(draw_mode, GlyphDrawMode::Invisible) {
            return;
        }
        let text = match glyph.as_unicode() {
            Some(BfString::Char(c)) => unligate(c).map_or_else(|| c.to_string(), str::to_string),
            Some(BfString::String(s)) => s,
            None => return,
        };
        if text.trim().is_empty() {
            return;
        }
        let m = transform * glyph_transform;
        // Rotated text, as on a plot's axis, cannot be set in rows.
        let [a, b, ..] = m.as_coeffs();
        if a <= 0. || b.abs() > 0.1 * a {
            return;
        }
        let rgb = match paint {
            Paint::Color(c) => {
                let [r, g, b, alpha] = c.to_rgba().to_rgba8();
                if alpha < 128 {
                    return;
                }
                [r, g, b]
            }
            Paint::Pattern(_) => [0, 0, 0],
        };
        // Outlines are in units of 1000 per em.
        let origin = m * Point::ZERO;
        let size = (m * Point::new(0., 1000.) - origin).hypot() as f32;
        let bbox = match glyph {
            Glyph::Outline(o) => m.transform_rect_bbox(o.outline().bounding_box()),
            Glyph::Type3(_) => {
                let (ascent, width, descent) = TYPE3;
                let s = f64::from(size);
                Rect::new(
                    origin.x,
                    origin.y - ascent * s,
                    origin.x + width * s,
                    origin.y + descent * s,
                )
            }
        };
        let advance = match glyph {
            Glyph::Outline(o) => o.advance_width(),
            Glyph::Type3(_) => None,
        };
        let pen = |at: f64| (m * Point::new(at, 0.)).x;
        // A width past the outline by more than an em is in other units.
        let (start, end) = match advance.map(|a| pen(a.into())) {
            Some(end) if end >= origin.x && end <= bbox.x1 + f64::from(size) => (origin.x, end),
            _ => {
                let bearing = BEARING * f64::from(size);
                (bbox.x0 - bearing, bbox.x1 + bearing)
            }
        };
        // Filled and stroked text, as in fake bold, is drawn twice.
        let again = |l: &Mark| l.text == text && l.x0 == bbox.x0 as f32 && l.y == origin.y as f32;
        if self.glyphs.last().is_some_and(again) {
            return;
        }
        let (math_face, heavy) = match glyph {
            Glyph::Outline(o) => *self.fonts.entry(o.font_cache_key()).or_insert_with(|| {
                o.font_data()
                    .and_then(|f| f.postscript_name)
                    .map_or((false, false), |name| {
                        // Less a subset's tag, as `ABCDEF+`.
                        let name = name.split_once('+').map_or(&*name, |(_, base)| base);
                        (math_font(name), bold_font(name))
                    })
            }),
            Glyph::Type3(_) => (false, false),
        };
        let math = math_face || text.chars().any(math_char);
        self.glyphs.push(Mark {
            x0: bbox.x0 as f32,
            x1: bbox.x1 as f32,
            start: start as f32,
            end: end as f32,
            y0: bbox.y0 as f32,
            y1: bbox.y1 as f32,
            y: origin.y as f32,
            size,
            text,
            rgb,
            back: [255; 3],
            math,
            heavy,
        });
    }
}
