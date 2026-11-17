//! A theme's colours and fonts, and colours as DrawingML writes them.

use super::super::xml::{attr, child, find, is};
use roxmltree::Node;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Theme {
    /// By scheme name, as `dk1` or `accent1`.
    colors: HashMap<String, Rgb>,
    /// The master's colour map, a slide's over it: which scheme colour the
    /// text and background colours, as `tx1`, are.
    map: HashMap<String, String>,
    /// The headings' font and the body's.
    pub major: Option<String>,
    pub minor: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn typst(self) -> String {
        format!("rgb(\"#{:02x}{:02x}{:02x}\")", self.0, self.1, self.2)
    }
}

impl Theme {
    pub fn read(root: Node) -> Self {
        let mut theme = Theme::default();
        if let Some(scheme) = find(root, "a:clrScheme") {
            for c in scheme.children().filter(Node::is_element) {
                let rgb = c.children().find_map(|v| {
                    if is(v, "a:srgbClr") {
                        hex(attr(v, "val")?)
                    } else if is(v, "a:sysClr") {
                        hex(attr(v, "lastClr")?)
                    } else {
                        None
                    }
                });
                if let Some(rgb) = rgb {
                    theme.colors.insert(c.tag_name().name().to_string(), rgb);
                }
            }
        }
        let font = |name: &str| {
            find(root, name)
                .and_then(|f| child(f, "a:latin"))
                .and_then(|l| attr(l, "typeface"))
                .filter(|t| !t.is_empty())
                .map(str::to_string)
        };
        theme.major = font("a:majorFont");
        theme.minor = font("a:minorFont");
        theme
    }

    /// The theme with a master's colour map, `p:clrMap`, and a slide's or
    /// layout's override of it, `a:overrideClrMapping`, the last read last.
    pub fn mapped(mut self, maps: &[Option<Node>]) -> Self {
        for m in maps.iter().flatten() {
            for a in m.attributes() {
                self.map.insert(a.name().to_string(), a.value().to_string());
            }
        }
        self
    }

    /// A typeface as a run names it, theme fonts resolved.
    pub fn typeface(&self, name: &str) -> Option<String> {
        match name {
            "+mj-lt" => self.major.clone(),
            "+mn-lt" => self.minor.clone(),
            n if n.starts_with('+') || n.is_empty() => None,
            n => Some(n.to_string()),
        }
    }

    /// The colour of a fill, as `a:solidFill`, or of an element holding a
    /// colour, as `a:fillRef`: `None` if it has none.
    pub fn color(&self, n: Node) -> Option<Rgb> {
        let c = n.children().find(|c| {
            ["a:srgbClr", "a:schemeClr", "a:sysClr", "a:prstClr"]
                .iter()
                .any(|name| is(*c, name))
        })?;
        let base = if is(c, "a:srgbClr") {
            hex(attr(c, "val")?)?
        } else if is(c, "a:sysClr") {
            hex(attr(c, "lastClr").unwrap_or("000000"))?
        } else if is(c, "a:prstClr") {
            match attr(c, "val")? {
                "white" => Rgb(255, 255, 255),
                "black" => Rgb(0, 0, 0),
                "red" => Rgb(255, 0, 0),
                "blue" => Rgb(0, 0, 255),
                "green" => Rgb(0, 128, 0),
                "gray" | "grey" => Rgb(128, 128, 128),
                _ => return None,
            }
        } else {
            let val = attr(c, "val")?;
            let name = match self.map.get(val) {
                Some(mapped) => mapped.as_str(),
                None => match val {
                    "tx1" => "dk1",
                    "bg1" => "lt1",
                    "tx2" => "dk2",
                    "bg2" => "lt2",
                    other => other,
                },
            };
            *self.colors.get(name)?
        };
        Some(modified(base, c))
    }
}

/// `rgb` as the colour's modifiers change it: luminance, tint and shade.
fn modified(rgb: Rgb, c: Node) -> Rgb {
    let pct = |name: &str| {
        child(c, name)
            .and_then(|m| attr(m, "val"))
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v / 100_000.0)
    };
    let (h, s, mut l) = hsl(rgb);
    if let Some(m) = pct("a:lumMod") {
        l *= m;
    }
    if let Some(o) = pct("a:lumOff") {
        l += o;
    }
    let mut out = from_hsl(h, s, l.clamp(0.0, 1.0));
    if let Some(t) = pct("a:tint") {
        let mix = |v: u8| (f64::from(v) + (255.0 - f64::from(v)) * (1.0 - t)).round() as u8;
        out = Rgb(mix(out.0), mix(out.1), mix(out.2));
    }
    if let Some(s) = pct("a:shade") {
        let mix = |v: u8| (f64::from(v) * s).round() as u8;
        out = Rgb(mix(out.0), mix(out.1), mix(out.2));
    }
    out
}

fn hex(s: &str) -> Option<Rgb> {
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn hsl(Rgb(r, g, b): Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    if s <= 0.0 {
        let v = (l * 255.0).round() as u8;
        return Rgb(v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round() as u8
    };
    Rgb(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}
