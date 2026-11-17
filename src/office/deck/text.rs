//! A shape's text as typst: paragraphs at their sizes, in their fonts and
//! colours, aligned, bulleted and spaced as the slide, its layout and its
//! master say.

use super::super::omml;
use super::super::package::Rels;
use super::super::xml::{attr, child, children, is};
use super::inherit::{
    Bullet, Parents, Ph, Spacing, bullet, numbered, para_attr, run_default, run_default_child,
    spacing,
};
use super::theme::{Rgb, Theme};
use super::{EMU_PER_PT, pt};
use crate::markdown::string;
use roxmltree::Node;

/// Size of text nothing gives one, in points.
const DEFAULT_SIZE: f64 = 18.0;

/// Fonts typst falls back to, after the slide's own.
const FALLBACK_FONTS: [&str; 3] = ["Helvetica Neue", "Arial", "Libertinus Serif"];

/// What a shape's text is set from.
pub(super) struct Text<'c, 'a, 'i> {
    pub theme: &'c Theme,
    pub parents: &'c Parents<'a, 'i>,
    pub rels: &'c Rels,
    /// The shape, and the placeholders it takes from.
    pub shape: Node<'a, 'i>,
    pub chain: Vec<Node<'a, 'i>>,
    pub ph: Option<Ph>,
    /// Shrunk to fit, as a fraction.
    pub scale: f64,
    /// Lines closer by, as a fraction.
    pub line_cut: f64,
    /// The colour the shape's style gives its text.
    pub color: Option<Rgb>,
}

impl Text<'_, '_, '_> {
    /// The paragraphs of `body`, `a:txBody`, as typst content, one block
    /// after another; empty if none has text.
    pub fn body(&self, body: Node) -> String {
        let mut out = String::new();
        let mut numbers = [0usize; 9];
        let mut any = false;
        for (i, p) in children(body, "a:p").enumerate() {
            let (para, has_text) = self.paragraph(p, i == 0, &mut numbers);
            any |= has_text;
            out.push_str(&para);
        }
        if any { out } else { String::new() }
    }

    fn paragraph(&self, p: Node, first: bool, numbers: &mut [usize; 9]) -> (String, bool) {
        let ppr = child(p, "a:pPr");
        let level = ppr
            .and_then(|p| attr(p, "lvl"))
            .and_then(|l| l.parse::<usize>().ok())
            .unwrap_or(0)
            .min(8);
        let levels = self
            .parents
            .levels(self.shape, &self.chain, self.ph.as_ref(), level);
        let size = run_default(&levels, "sz")
            .and_then(|s| s.parse::<f64>().ok())
            .map_or(DEFAULT_SIZE, |s| s / 100.0)
            * self.scale;
        let runs = self.runs(p, &levels, size);
        let has_text = !runs.trim().is_empty();
        let end_size = child(p, "a:endParaRPr")
            .and_then(|r| attr(r, "sz"))
            .and_then(|s| s.parse::<f64>().ok())
            .map_or(size, |s| s / 100.0 * self.scale);
        let before = match spacing(ppr, &levels, "a:spcBef") {
            _ if first => 0.0,
            Some(Spacing::Points(p)) => p,
            Some(Spacing::Lines(l)) => l * size * 1.2,
            None => 0.0,
        };
        // PowerPoint's single spacing is about 1.2 of the size; typst's
        // leading is the gap between lines.
        let lines = match spacing(ppr, &levels, "a:lnSpc") {
            Some(Spacing::Lines(l)) => (l - self.line_cut).max(0.5),
            Some(Spacing::Points(p)) => p / size.max(1.0) / 1.2,
            None => 1.0 - self.line_cut,
        };
        let leading = (1.2 * lines - 0.95).max(0.05);
        if !has_text {
            numbers[level] = 0;
            return (
                format!(
                    "#block(above: {}, below: 0pt, height: {})[]\n",
                    pt(before),
                    pt(end_size * 1.2 * lines)
                ),
                false,
            );
        }
        let algn = para_attr(ppr, &levels, "algn");
        let align = match algn {
            Some("ctr") => "center",
            Some("r") => "right",
            _ => "left",
        };
        let justify = matches!(algn, Some("just" | "dist"));
        let emu = |name: &str| {
            para_attr(ppr, &levels, name)
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(0.0, |v| v / EMU_PER_PT)
        };
        let (mar_l, indent) = (emu("marL"), emu("indent"));
        // A paragraph ends the numbering of the levels under it; one not
        // numbered ends its own.
        for n in numbers.iter_mut().skip(level + 1) {
            *n = 0;
        }
        let mark = match bullet(ppr, &levels) {
            Bullet::Number { scheme, start } => {
                numbers[level] += 1;
                Some(numbered(&scheme, start + numbers[level] - 1))
            }
            Bullet::Char(c) => {
                numbers[level] = 0;
                Some(c)
            }
            Bullet::None => {
                numbers[level] = 0;
                None
            }
        };
        let content = match &mark {
            Some(mark) => {
                let color = self.color_of(None, &levels).typst();
                let hang = (-indent).max(size * 0.6);
                format!(
                    "#pad(left: {})[#grid(columns: ({}, 1fr), text(size: {}, fill: {color}, {}), [{runs}])]",
                    pt((mar_l - hang).max(0.0)),
                    pt(hang),
                    pt(size),
                    string(mark),
                )
            }
            None => format!("#pad(left: {})[{runs}]", pt(mar_l.max(0.0))),
        };
        (
            format!(
                "#block(above: {}, below: 0pt, width: 100%)[#set par(leading: {leading:.3}em, justify: {justify}, first-line-indent: 0pt)\n#align({align})[{content}]]\n",
                pt(before)
            ),
            true,
        )
    }

    /// A paragraph's runs, as typst.
    fn runs(&self, p: Node, levels: &[Node], size: f64) -> String {
        let mut out = String::new();
        for c in p.children().filter(Node::is_element) {
            if is(c, "a:r") || is(c, "a:fld") {
                let text: String = children(c, "a:t").filter_map(|t| t.text()).collect();
                if !text.is_empty() {
                    out.push_str(&self.run(child(c, "a:rPr"), levels, size, &text));
                }
            } else if is(c, "a:br") {
                out.push_str("#linebreak()");
            } else if is(c, "a:pPr") || is(c, "a:endParaRPr") {
            } else if let Some(math) = c
                .descendants()
                .find(|d| is(*d, "m:oMathPara") || is(*d, "m:oMath"))
            {
                let tex = omml::latex(math);
                let set = crate::math::convert(&tex)
                    .map_or_else(|| format!("#raw({})", string(&tex)), |m| format!("${m}$"));
                let color = self.color_of(None, levels).typst();
                out.push_str(&format!("#text(size: {}, fill: {color})[{set}]", pt(size)));
            }
        }
        out
    }

    fn run(&self, rpr: Option<Node>, levels: &[Node], size: f64, text: &str) -> String {
        let own = |name: &str| rpr.and_then(|r| attr(r, name));
        let flag = |name: &str| {
            own(name)
                .or_else(|| run_default(levels, name))
                .is_some_and(|v| v == "1" || v == "true")
        };
        let size = own("sz")
            .and_then(|s| s.parse::<f64>().ok())
            .map_or(size, |s| s / 100.0 * self.scale);
        let mut args = vec![format!("size: {}", pt(size))];
        if flag("b") {
            args.push("weight: \"bold\"".into());
        }
        if flag("i") {
            args.push("style: \"italic\"".into());
        }
        args.push(format!("fill: {}", self.color_of(rpr, levels).typst()));
        let face = rpr
            .and_then(|r| child(r, "a:latin"))
            .and_then(|l| attr(l, "typeface"))
            .or_else(|| run_default_child(levels, "a:latin").and_then(|l| attr(l, "typeface")))
            .and_then(|f| self.theme.typeface(f))
            .or_else(|| {
                if self
                    .ph
                    .as_ref()
                    .is_some_and(|p| matches!(p.kind.as_str(), "title" | "ctrTitle"))
                {
                    self.theme.major.clone()
                } else {
                    self.theme.minor.clone()
                }
            });
        let fonts: Vec<String> = face
            .into_iter()
            .chain(FALLBACK_FONTS.iter().map(ToString::to_string))
            .map(|f| string(&f))
            .collect();
        args.push(format!("font: ({})", fonts.join(", ")));
        let mut s = format!("#text({}, {})", args.join(", "), string(text));
        let baseline = own("baseline")
            .and_then(|b| b.parse::<i64>().ok())
            .unwrap_or(0);
        if baseline > 0 {
            s = format!("#super({})", &s[1..]);
        } else if baseline < 0 {
            s = format!("#sub({})", &s[1..]);
        }
        if own("u").is_some_and(|u| u != "none") {
            s = format!("#underline({})", &s[1..]);
        }
        if own("strike").is_some_and(|u| u != "noStrike") {
            s = format!("#strike({})", &s[1..]);
        }
        let link = rpr
            .and_then(|r| child(r, "a:hlinkClick"))
            .and_then(|h| attr(h, "r:id"))
            .and_then(|id| self.rels.get(id))
            .filter(|r| r.external);
        if let Some(rel) = link {
            s = format!("#link({}, {})", string(&rel.target), &s[1..]);
        }
        s
    }

    /// A run's colour: its own, its level's, its shape's style's, or the
    /// theme's text colour.
    fn color_of(&self, rpr: Option<Node>, levels: &[Node]) -> Rgb {
        rpr.and_then(|r| child(r, "a:solidFill"))
            .and_then(|f| self.theme.color(f))
            .or_else(|| run_default_child(levels, "a:solidFill").and_then(|f| self.theme.color(f)))
            .or(self.color)
            .unwrap_or(Rgb(0, 0, 0))
    }
}
