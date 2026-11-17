//! What a slide's shapes take from its layout and master: a placeholder's
//! place, its text's sizes, alignment and bullets.
//!
//! As DeckCraft's craft-pptx has it: a slide's placeholder matches one on
//! its layout by index, then by type, then a compatible type; the master's
//! is the title, the footer's own, or the body. Text properties are taken
//! from the most particular place that gives them: the paragraph, the
//! shape's list style, the layout's and master's placeholders', then the
//! master's title, body or other style, then the presentation's default.

use super::super::xml::{attr, child, children, find, is};
use roxmltree::Node;

pub(super) use super::super::pptx::{Ph, ph};

fn title(kind: &str) -> bool {
    matches!(kind, "title" | "ctrTitle")
}

fn compatible(slide: &str, other: &str) -> bool {
    match slide {
        "title" | "ctrTitle" => title(other),
        "subTitle" | "body" | "obj" | "chart" | "tbl" | "clipArt" | "dgm" | "media" | "pic" => {
            !matches!(
                other,
                "title" | "ctrTitle" | "dt" | "ftr" | "sldNum" | "hdr"
            )
        }
        k => k == other,
    }
}

/// The placeholders on a layout or master.
fn placeholders<'a, 'i>(root: Node<'a, 'i>) -> Vec<(Node<'a, 'i>, Ph)> {
    let Some(tree) = find(root, "p:spTree") else {
        return Vec::new();
    };
    tree.descendants()
        .filter(|n| is(*n, "p:sp") || is(*n, "p:pic"))
        .filter_map(|n| Some((n, ph(n)?)))
        .collect()
}

/// A slide's layout and master, as they bear on its shapes.
pub(super) struct Parents<'a, 'i> {
    layout: Vec<(Node<'a, 'i>, Ph)>,
    master: Vec<(Node<'a, 'i>, Ph)>,
    /// The master's text styles.
    styles: Option<Node<'a, 'i>>,
    /// The presentation's default text style.
    default: Option<Node<'a, 'i>>,
}

impl<'a, 'i> Parents<'a, 'i> {
    pub fn new(
        layout: Option<Node<'a, 'i>>,
        master: Option<Node<'a, 'i>>,
        default: Option<Node<'a, 'i>>,
    ) -> Self {
        Parents {
            layout: layout.map(placeholders).unwrap_or_default(),
            master: master.map(placeholders).unwrap_or_default(),
            styles: master.and_then(|m| find(m, "p:txStyles")),
            default,
        }
    }

    /// The layout's and the master's placeholders a slide's placeholder
    /// `p` takes from, the layout's first.
    pub fn of(&self, p: &Ph) -> Vec<Node<'a, 'i>> {
        let mut out = Vec::new();
        let on_layout = (p.idx != 0)
            .then(|| {
                self.layout
                    .iter()
                    .find(|(_, l)| l.idx == p.idx && compatible(&p.kind, &l.kind))
            })
            .flatten()
            .or_else(|| self.layout.iter().find(|(_, l)| l.kind == p.kind))
            .or_else(|| {
                self.layout
                    .iter()
                    .find(|(_, l)| compatible(&p.kind, &l.kind))
            });
        let kind = on_layout.map_or(p.kind.as_str(), |(_, l)| l.kind.as_str());
        if let Some((n, _)) = on_layout {
            out.push(*n);
        }
        let want = match kind {
            k if title(k) => "title",
            k @ ("dt" | "ftr" | "sldNum" | "hdr") => k,
            _ => "body",
        };
        if let Some((n, _)) = self.master.iter().find(|(_, m)| m.kind == want) {
            out.push(*n);
        }
        out
    }

    /// The level properties, as `a:lvl1pPr`, that text at `level`, from 0,
    /// in a shape takes, the most particular first: the shape's list
    /// style, its parents', and the master's style for its kind.
    pub fn levels(
        &self,
        shape: Node<'a, 'i>,
        parents: &[Node<'a, 'i>],
        p: Option<&Ph>,
        level: usize,
    ) -> Vec<Node<'a, 'i>> {
        let name = format!("a:lvl{}pPr", level + 1);
        let mut out = Vec::new();
        for s in std::iter::once(&shape).chain(parents) {
            if let Some(l) = child(*s, "p:txBody")
                .and_then(|b| child(b, "a:lstStyle"))
                .and_then(|l| child(l, &name))
            {
                out.push(l);
            }
        }
        // Footers, dates and slide numbers are not body text: they take
        // the other style, with no bullets.
        let style = match p.map(|p| p.kind.as_str()) {
            Some(k) if title(k) => "p:titleStyle",
            Some("dt" | "ftr" | "sldNum" | "hdr") | None => "p:otherStyle",
            Some(_) => "p:bodyStyle",
        };
        if let Some(l) = self
            .styles
            .and_then(|s| child(s, style))
            .and_then(|s| child(s, &name))
        {
            out.push(l);
        }
        if let Some(l) = self.default.and_then(|d| child(d, &name)) {
            out.push(l);
        }
        out
    }
}

/// The first of `nodes` whose `a:bodyPr` gives the attribute `name`.
pub(super) fn body_attr<'a>(nodes: &[Node<'a, '_>], name: &str) -> Option<&'a str> {
    nodes.iter().find_map(|n| {
        let body = child(*n, "p:txBody").and_then(|b| child(b, "a:bodyPr"))?;
        attr(body, name)
    })
}

/// The first of `nodes` with a child `name` under its `p:spPr`, as
/// `a:xfrm`.
pub(super) fn sp_pr<'a, 'i>(nodes: &[Node<'a, 'i>], name: &str) -> Option<Node<'a, 'i>> {
    nodes.iter().find_map(|n| child(child(*n, "p:spPr")?, name))
}

/// Of level properties, the first to give attribute `name` of their
/// default run properties, as `sz`.
pub(super) fn run_default<'a>(levels: &[Node<'a, '_>], name: &str) -> Option<&'a str> {
    levels
        .iter()
        .find_map(|l| attr(child(*l, "a:defRPr")?, name))
}

/// Of level properties, the first child named `name` of their default run
/// properties, as `a:solidFill`.
pub(super) fn run_default_child<'a, 'i>(
    levels: &[Node<'a, 'i>],
    name: &str,
) -> Option<Node<'a, 'i>> {
    levels
        .iter()
        .find_map(|l| child(child(*l, "a:defRPr")?, name))
}

/// Whether the level properties, or the paragraph's own `ppr`, say how its
/// bullet is: `Some(None)` for none, `Some(Some(c))` for character `c`,
/// `Some(Some("#"))` for a number.
pub(super) fn bullet(ppr: Option<Node>, levels: &[Node]) -> Bullet {
    let all: Vec<Node> = ppr.into_iter().chain(levels.iter().copied()).collect();
    let font = all
        .iter()
        .find_map(|n| attr(child(*n, "a:buFont")?, "typeface"))
        .unwrap_or("");
    for n in &all {
        if child(*n, "a:buNone").is_some() {
            return Bullet::None;
        }
        if let Some(c) = child(*n, "a:buChar") {
            return Bullet::Char(symbol(attr(c, "char").unwrap_or("•"), font));
        }
        if let Some(a) = child(*n, "a:buAutoNum") {
            return Bullet::Number {
                scheme: attr(a, "type").unwrap_or("arabicPeriod").to_string(),
                start: attr(a, "startAt")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1)
                    .max(1),
            };
        }
    }
    Bullet::None
}

/// How a paragraph is bulleted.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Bullet {
    None,
    Char(String),
    /// Numbered in a scheme, as `alphaLcParenR`, from `start`.
    Number {
        scheme: String,
        start: usize,
    },
}

/// A bullet character in a symbol font, as Unicode: Wingdings' and
/// Symbol's characters stand at Latin codes.
fn symbol(c: &str, font: &str) -> String {
    let font = font.to_ascii_lowercase();
    let mapped = if font.starts_with("wingdings") {
        match c {
            "§" => "▪",
            "Ø" => "➢",
            "ü" => "✓",
            "q" => "❑",
            "v" => "❖",
            "n" => "■",
            "l" => "●",
            "o" => "□",
            "à" => "➔",
            "è" => "➔",
            "ð" => "⇨",
            _ => "•",
        }
    } else if font == "symbol" {
        match c {
            "·" => "•",
            "-" => "–",
            "®" => "→",
            _ => "•",
        }
    } else {
        return c.to_string();
    };
    mapped.to_string()
}

/// The `n`th number, from 1, of the numbering `scheme`, as `alphaLcParenR`
/// writes 3 as `c)`.
pub(super) fn numbered(scheme: &str, n: usize) -> String {
    let alpha = |upper: bool| {
        let mut s = String::new();
        let mut k = n;
        while k > 0 {
            k -= 1;
            let c = (b'a' + (k % 26) as u8) as char;
            s.insert(0, if upper { c.to_ascii_uppercase() } else { c });
            k /= 26;
        }
        s
    };
    let roman = |upper: bool| {
        let mut s = String::new();
        let mut k = n.min(3999);
        for (v, r) in [
            (1000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ] {
            while k >= v {
                s.push_str(r);
                k -= v;
            }
        }
        if upper { s.to_uppercase() } else { s }
    };
    let body = if scheme.starts_with("alphaLc") {
        alpha(false)
    } else if scheme.starts_with("alphaUc") {
        alpha(true)
    } else if scheme.starts_with("romanLc") {
        roman(false)
    } else if scheme.starts_with("romanUc") {
        roman(true)
    } else {
        n.to_string()
    };
    if scheme.ends_with("ParenBoth") {
        format!("({body})")
    } else if scheme.ends_with("ParenR") {
        format!("{body})")
    } else if scheme.ends_with("Plain") {
        body
    } else {
        format!("{body}.")
    }
}

/// The first of the paragraph's own `ppr` and the level properties to give
/// attribute `name`, as `algn` or `marL`.
pub(super) fn para_attr<'a>(
    ppr: Option<Node<'a, '_>>,
    levels: &[Node<'a, '_>],
    name: &str,
) -> Option<&'a str> {
    ppr.into_iter()
        .chain(levels.iter().copied())
        .find_map(|n| attr(n, name))
}

/// Spacing, as `a:spcBef`, in points, or as a fraction of the line for a
/// percentage.
pub(super) fn spacing(ppr: Option<Node>, levels: &[Node], name: &str) -> Option<Spacing> {
    ppr.into_iter().chain(levels.iter().copied()).find_map(|n| {
        let s = child(n, name)?;
        if let Some(p) = child(s, "a:spcPts").and_then(|p| attr(p, "val")) {
            return Some(Spacing::Points(p.parse::<f64>().ok()? / 100.0));
        }
        let p = child(s, "a:spcPct").and_then(|p| attr(p, "val"))?;
        Some(Spacing::Lines(p.parse::<f64>().ok()? / 100_000.0))
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Spacing {
    Points(f64),
    Lines(f64),
}

/// Whether the slide or its layout hides the master's own shapes.
pub(super) fn hides_master(root: Node) -> bool {
    matches!(attr(root, "showMasterSp"), Some("0" | "false"))
}

/// The shapes of a layout or master that are not placeholders, drawn on
/// every slide that shows them.
pub(super) fn decorations<'a, 'i>(root: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    let Some(tree) = find(root, "p:spTree") else {
        return Vec::new();
    };
    tree.children()
        .filter(Node::is_element)
        .filter(|n| !is(*n, "p:nvGrpSpPr") && !is(*n, "p:grpSpPr"))
        .filter(|n| ph(*n).is_none())
        .collect()
}

/// The slide's background fill, from the first of the parts to give one.
pub(super) fn background(roots: &[Node], theme: &super::theme::Theme) -> Option<super::theme::Rgb> {
    // The first part to have a background is the one shown, whatever its
    // fill: a gradient's first stop stands for it, a picture's for nothing.
    let bg = roots.iter().find_map(|r| find(*r, "p:bg"))?;
    if let Some(r) = child(bg, "p:bgRef") {
        return theme.color(r);
    }
    let pr = child(bg, "p:bgPr")?;
    if let Some(f) = child(pr, "a:solidFill") {
        return theme.color(f);
    }
    let stop = child(pr, "a:gradFill")
        .and_then(|g| child(g, "a:gsLst"))
        .and_then(|l| children(l, "a:gs").next())?;
    theme.color(stop)
}
