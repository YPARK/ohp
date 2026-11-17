//! A slide's shapes as typst, each placed where it is on the slide:
//! pictures, text, filled shapes and lines, tables and groups.

use super::super::media::Media;
use super::super::package::{Package, Rels};
use super::super::xml::{attr, child, children, chosen, find, is};
use super::inherit::{self, Parents, Ph, body_attr, sp_pr};
use super::text::Text;
use super::theme::{Rgb, Theme};
use super::{EMU_PER_PT, Pictures, pt};
use roxmltree::Node;

/// How a part's coordinates, in EMU, map onto the slide's, in points: a
/// group's children are in a space of their own.
#[derive(Clone, Copy, Debug)]
pub(super) struct Map {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

impl Default for Map {
    fn default() -> Self {
        Map {
            sx: 1.0 / EMU_PER_PT,
            sy: 1.0 / EMU_PER_PT,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

/// A box on the slide, in points, turned and flipped.
#[derive(Clone, Copy, Debug)]
struct Frame {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
}

fn emu(n: Node, name: &str) -> f64 {
    attr(n, name)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

impl Map {
    fn frame(&self, xfrm: Node) -> Option<Frame> {
        let off = child(xfrm, "a:off")?;
        let ext = child(xfrm, "a:ext")?;
        Some(Frame {
            x: self.tx + self.sx * emu(off, "x"),
            y: self.ty + self.sy * emu(off, "y"),
            w: self.sx * emu(ext, "cx"),
            h: self.sy * emu(ext, "cy"),
            rot: emu(xfrm, "rot") / 60_000.0,
            flip_h: matches!(attr(xfrm, "flipH"), Some("1" | "true")),
            flip_v: matches!(attr(xfrm, "flipV"), Some("1" | "true")),
        })
    }

    /// The map for a group's children, from its `a:xfrm`.
    fn group(&self, xfrm: Node) -> Self {
        let (Some(off), Some(ext)) = (child(xfrm, "a:off"), child(xfrm, "a:ext")) else {
            return *self;
        };
        let ch_off = child(xfrm, "a:chOff");
        let ch_ext = child(xfrm, "a:chExt");
        let ratio = |e: f64, c: Option<f64>| match c {
            Some(c) if c.abs() > 1e-9 => e / c,
            _ => 1.0,
        };
        let kx = ratio(emu(ext, "cx"), ch_ext.map(|c| emu(c, "cx")));
        let ky = ratio(emu(ext, "cy"), ch_ext.map(|c| emu(c, "cy")));
        let (cx, cy) = ch_off.map_or((0.0, 0.0), |c| (emu(c, "x"), emu(c, "y")));
        Map {
            sx: self.sx * kx,
            sy: self.sy * ky,
            tx: self.tx + self.sx * (emu(off, "x") - cx * kx),
            ty: self.ty + self.sy * (emu(off, "y") - cy * ky),
        }
    }
}

/// What a part's shapes are drawn with.
pub(super) struct Painter<'c, 'a, 'i> {
    pub pkg: &'c mut Package,
    pub media: &'c mut Media,
    pub pictures: &'c mut Pictures,
    pub theme: &'c Theme,
    pub parents: &'c Parents<'a, 'i>,
    pub rels: &'c Rels,
    /// Whether placeholders take from the layout and master: a slide's do.
    pub slide: bool,
    pub out: String,
    /// Charts and diagrams not drawn, for the status line.
    pub charts: usize,
    pub diagrams: usize,
}

impl<'a, 'i> Painter<'_, 'a, 'i> {
    /// The shapes under `tree`, a shape tree or a group, in order.
    pub fn shapes(&mut self, tree: Node<'a, 'i>, map: Map) {
        for c in tree.children().filter(Node::is_element) {
            self.shape(c, map);
        }
    }

    pub fn shape(&mut self, n: Node<'a, 'i>, map: Map) {
        if is(n, "p:sp") {
            self.sp(n, map);
        } else if is(n, "p:pic") {
            self.pic(n, map);
        } else if is(n, "p:cxnSp") {
            self.connector(n, map);
        } else if is(n, "p:graphicFrame") {
            self.graphic(n, map);
        } else if is(n, "p:grpSp") {
            let inner = child(n, "p:grpSpPr")
                .and_then(|p| child(p, "a:xfrm"))
                .map_or(map, |x| map.group(x));
            self.shapes(n, inner);
        } else if is(n, "mc:AlternateContent")
            && let Some(c) = chosen(n)
        {
            self.shapes(c, map);
        }
    }

    /// The placeholders `n` takes from, and what it is.
    fn chain(&self, n: Node<'a, 'i>) -> (Option<Ph>, Vec<Node<'a, 'i>>) {
        let ph = inherit::ph(n);
        let chain = match (&ph, self.slide) {
            (Some(p), true) => self.parents.of(p),
            _ => Vec::new(),
        };
        (ph, chain)
    }

    fn place(&mut self, f: Frame, body: &str) {
        let mut inner = body.to_string();
        if f.flip_h || f.flip_v {
            let x = if f.flip_h { "-100%" } else { "100%" };
            let y = if f.flip_v { "-100%" } else { "100%" };
            inner = format!("scale(x: {x}, y: {y}, {inner})");
        }
        if f.rot.abs() > 0.01 {
            inner = format!("rotate({:.2}deg, {inner})", f.rot);
        }
        self.out.push_str(&format!(
            "#place(top + left, dx: {}, dy: {}, {inner})\n",
            pt(f.x),
            pt(f.y)
        ));
    }

    /// A shape or picture: what placeholder it is, itself and the
    /// placeholders it takes from, and where it is.
    fn placed(&self, n: Node<'a, 'i>, map: Map) -> Option<(Option<Ph>, Vec<Node<'a, 'i>>, Frame)> {
        let (ph, chain) = self.chain(n);
        let with: Vec<Node> = std::iter::once(n).chain(chain).collect();
        let f = sp_pr(&with, "a:xfrm").and_then(|x| map.frame(x))?;
        Some((ph, with, f))
    }

    fn sp(&mut self, n: Node<'a, 'i>, map: Map) {
        let Some((ph, with, f)) = self.placed(n, map) else {
            return;
        };
        let pr = child(n, "p:spPr");
        let style = child(n, "p:style");
        // Its outline and fill, drawn under its text.
        let fill = fill(self.theme, pr, style);
        let stroke = stroke(self.theme, pr, style);
        if fill.is_some() || stroke.is_some() {
            let geom = pr
                .and_then(|p| child(p, "a:prstGeom"))
                .and_then(|g| attr(g, "prst"))
                .unwrap_or("rect");
            let paint = paint(fill, stroke);
            let shape = match geom {
                "ellipse" => format!("ellipse(width: {}, height: {}{paint})", pt(f.w), pt(f.h)),
                "line" => format!(
                    "line(end: ({}, {}){})",
                    pt(f.w),
                    pt(f.h),
                    stroke_only(stroke)
                ),
                "roundRect" => format!(
                    "rect(width: {}, height: {}, radius: {}{paint})",
                    pt(f.w),
                    pt(f.h),
                    pt(f.w.min(f.h) * 0.1667)
                ),
                _ => format!("rect(width: {}, height: {}{paint})", pt(f.w), pt(f.h)),
            };
            self.place(f, &shape);
        }
        let Some(body) = child(n, "p:txBody") else {
            return;
        };
        let pct = |n: Option<Node>, name: &str| {
            n.and_then(|n| attr(n, name))
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 100_000.0)
        };
        let autofit = child(body, "a:bodyPr").and_then(|b| child(b, "a:normAutofit"));
        let text = Text {
            chain: with[1..].to_vec(),
            ph,
            scale: pct(autofit, "fontScale").unwrap_or(1.0).clamp(0.05, 1.0),
            line_cut: pct(autofit, "lnSpcReduction")
                .unwrap_or(0.0)
                .clamp(0.0, 0.9),
            color: style
                .and_then(|s| child(s, "a:fontRef"))
                .and_then(|r| self.theme.color(r)),
            ..self.text(n)
        };
        let content = text.body(body);
        if content.is_empty() {
            return;
        }
        let inset = |name: &str, default: f64| {
            body_attr(&with, name)
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(default, |v| v / EMU_PER_PT)
        };
        let anchor = match body_attr(&with, "anchor") {
            Some("ctr") => "horizon",
            Some("b") => "bottom",
            _ => "top",
        };
        let frame = format!(
            "block(width: {}, height: {}, inset: (left: {}, right: {}, top: {}, bottom: {}), align({anchor})[{content}])",
            pt(f.w),
            pt(f.h),
            pt(inset("lIns", 7.2)),
            pt(inset("rIns", 7.2)),
            pt(inset("tIns", 3.6)),
            pt(inset("bIns", 3.6)),
        );
        // Text is not flipped with its shape, only turned.
        self.place(
            Frame {
                flip_h: false,
                flip_v: false,
                ..f
            },
            &frame,
        );
    }

    /// Text in `shape`, set as nothing else says.
    fn text(&self, shape: Node<'a, 'i>) -> Text<'_, 'a, 'i> {
        Text {
            theme: self.theme,
            parents: self.parents,
            rels: self.rels,
            shape,
            chain: Vec::new(),
            ph: None,
            scale: 1.0,
            line_cut: 0.0,
            color: None,
        }
    }

    fn pic(&mut self, n: Node<'a, 'i>, map: Map) {
        let Some((_, _, f)) = self.placed(n, map) else {
            return;
        };
        let Some(fill) = child(n, "p:blipFill") else {
            return;
        };
        let Some(id) = child(fill, "a:blip").and_then(|b| attr(b, "r:embed")) else {
            return;
        };
        let Some(name) = self.pictures.add(self.pkg, self.media, self.rels, id) else {
            let alt = find(n, "p:cNvPr")
                .and_then(|c| attr(c, "name"))
                .unwrap_or("picture");
            let shown = format!(
                "rect(width: {}, height: {}, fill: luma(240), stroke: 0.5pt + luma(180), align(center + horizon, text(size: 9pt, fill: luma(120), {})))",
                pt(f.w),
                pt(f.h),
                crate::markdown::string(alt),
            );
            self.place(f, &shown);
            return;
        };
        // A crop, in thousandths of a percent from each side.
        let crop = child(fill, "a:srcRect");
        let side = |name: &str| crop.map_or(0.0, |c| emu(c, name) / 100_000.0);
        let (l, t, r, b) = (side("l"), side("t"), side("r"), side("b"));
        let image = if l == 0.0 && t == 0.0 && r == 0.0 && b == 0.0 {
            format!(
                "image({}, width: {}, height: {}, fit: \"stretch\")",
                crate::markdown::string(&name),
                pt(f.w),
                pt(f.h)
            )
        } else {
            let fw = f.w / (1.0 - l - r).max(0.01);
            let fh = f.h / (1.0 - t - b).max(0.01);
            format!(
                "box(width: {}, height: {}, clip: true, place(top + left, dx: {}, dy: {}, image({}, width: {}, height: {}, fit: \"stretch\")))",
                pt(f.w),
                pt(f.h),
                pt(-l * fw),
                pt(-t * fh),
                crate::markdown::string(&name),
                pt(fw),
                pt(fh),
            )
        };
        self.place(f, &image);
    }

    fn connector(&mut self, n: Node<'a, 'i>, map: Map) {
        let pr = child(n, "p:spPr");
        let Some(f) = pr
            .and_then(|p| child(p, "a:xfrm"))
            .and_then(|x| map.frame(x))
        else {
            return;
        };
        let Some(stroke) = stroke(self.theme, pr, child(n, "p:style")) else {
            return;
        };
        let (x0, x1) = if f.flip_h { (f.w, 0.0) } else { (0.0, f.w) };
        let (y0, y1) = if f.flip_v { (f.h, 0.0) } else { (0.0, f.h) };
        let line = format!(
            "line(start: ({}, {}), end: ({}, {}){})",
            pt(x0),
            pt(y0),
            pt(x1),
            pt(y1),
            stroke_only(Some(stroke))
        );
        self.place(
            Frame {
                flip_h: false,
                flip_v: false,
                ..f
            },
            &line,
        );
    }

    fn graphic(&mut self, n: Node<'a, 'i>, map: Map) {
        let Some(f) = child(n, "p:xfrm").and_then(|x| map.frame(x)) else {
            return;
        };
        let Some(tbl) = find(n, "a:tbl") else {
            let what = if n.descendants().any(|d| d.tag_name().name() == "chart") {
                self.charts += 1;
                "chart"
            } else {
                self.diagrams += 1;
                "diagram"
            };
            let shown = format!(
                "rect(width: {}, height: {}, stroke: 0.5pt + luma(180), align(center + horizon, text(size: 9pt, fill: luma(120), \"{what}\")))",
                pt(f.w),
                pt(f.h)
            );
            self.place(f, &shown);
            return;
        };
        let table = self.table(n, tbl);
        self.place(f, &format!("block(width: {}, {table})", pt(f.w)));
    }

    fn table(&mut self, frame: Node<'a, 'i>, tbl: Node<'a, 'i>) -> String {
        let cols: Vec<String> = child(tbl, "a:tblGrid")
            .into_iter()
            .flat_map(|g| children(g, "a:gridCol"))
            .map(|c| pt(emu(c, "w") / EMU_PER_PT))
            .collect();
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        let mut widest = 0;
        let text = self.text(frame);
        for tr in children(tbl, "a:tr") {
            rows.push(pt(emu(tr, "h") / EMU_PER_PT));
            widest = widest.max(children(tr, "a:tc").count());
            for tc in children(tr, "a:tc") {
                let merged = |name: &str| matches!(attr(tc, name), Some("1" | "true"));
                if merged("hMerge") || merged("vMerge") {
                    continue;
                }
                let body = child(tc, "a:txBody").map_or_else(String::new, |b| text.body(b));
                let mut args = Vec::new();
                // Spans are read as numbers, so nothing else is written
                // into the source.
                let span = |name: &str| {
                    attr(tc, name)
                        .and_then(|n| n.trim().parse::<u32>().ok())
                        .filter(|n| *n > 1)
                        .map(|n| n.min(64))
                };
                if let Some(n) = span("gridSpan") {
                    args.push(format!("colspan: {n}"));
                }
                if let Some(n) = span("rowSpan") {
                    args.push(format!("rowspan: {n}"));
                }
                if let Some(color) = child(tc, "a:tcPr")
                    .and_then(|p| child(p, "a:solidFill"))
                    .and_then(|f| self.theme.color(f))
                {
                    args.push(format!("fill: {}", color.typst()));
                }
                cells.push(format!("table.cell({})[{body}]", args.join(", ")));
            }
        }
        // A table with no grid is as wide as its widest row, in columns of
        // a share each.
        let columns = if cols.is_empty() {
            widest.max(1).to_string()
        } else {
            format!("({},)", cols.join(", "))
        };
        let rows = if rows.is_empty() {
            String::new()
        } else {
            format!("rows: ({},), ", rows.join(", "))
        };
        format!(
            "table(columns: {columns}, {rows}stroke: 0.5pt + luma(160), inset: 0pt, {})",
            cells.join(", ")
        )
    }
}

/// A shape's fill: its own, or its style's, or none.
fn fill(theme: &Theme, pr: Option<Node>, style: Option<Node>) -> Option<Rgb> {
    if let Some(pr) = pr {
        if child(pr, "a:noFill").is_some() {
            return None;
        }
        if let Some(f) = child(pr, "a:solidFill") {
            return theme.color(f);
        }
        if child(pr, "a:gradFill").is_some() || child(pr, "a:blipFill").is_some() {
            return None;
        }
    }
    let r = child(style?, "a:fillRef")?;
    (attr(r, "idx").unwrap_or("0") != "0")
        .then(|| theme.color(r))
        .flatten()
}

/// A shape's outline, its colour and width in points.
fn stroke(theme: &Theme, pr: Option<Node>, style: Option<Node>) -> Option<(Rgb, f64)> {
    let ln = pr.and_then(|p| child(p, "a:ln"));
    let width = ln
        .and_then(|l| attr(l, "w"))
        .and_then(|w| w.parse::<f64>().ok())
        .map_or(0.75, |w| w / EMU_PER_PT);
    if let Some(ln) = ln {
        if child(ln, "a:noFill").is_some() {
            return None;
        }
        if let Some(f) = child(ln, "a:solidFill") {
            return theme.color(f).map(|c| (c, width));
        }
    }
    let r = child(style?, "a:lnRef")?;
    (attr(r, "idx").unwrap_or("0") != "0")
        .then(|| theme.color(r))
        .flatten()
        .map(|c| (c, width))
}

fn paint(fill: Option<Rgb>, stroke: Option<(Rgb, f64)>) -> String {
    let mut s = String::new();
    if let Some(f) = fill {
        s.push_str(&format!(", fill: {}", f.typst()));
    }
    s.push_str(&stroke_only(stroke));
    s
}

fn stroke_only(stroke: Option<(Rgb, f64)>) -> String {
    match stroke {
        Some((c, w)) => format!(", stroke: {} + {}", pt(w), c.typst()),
        None => ", stroke: none".into(),
    }
}
