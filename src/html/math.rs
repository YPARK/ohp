//! Formulas in the page, as the TeX they are written in or were drawn
//! from by KaTeX or MathJax.

use super::class_in;
use scraper::ElementRef;

pub(super) fn formula_of(el: ElementRef) -> Option<(String, bool)> {
    let name = el.value().name();
    let math_class = el
        .value()
        .classes()
        .any(|c| c.eq_ignore_ascii_case("math") || ascii_starts(c, "math-"));
    let mathml = name == "math";
    let script = name == "script" && el.attr("type").is_some_and(|t| ascii_starts(t, "math/tex"));
    // The drawing is what the page shows. The TeX is an annotation or a
    // script beside it, and the drawing is not readable as the formula.
    if !math_class && !mathml && !script && !rendered_math(el) {
        return None;
    }
    let display = math_display(el);
    if let Some(tex) = annotation_tex(el) {
        return bare_tex(&tex, display);
    }
    if let Some((tex, disp)) = tex_script(el) {
        return bare_tex(&tex, display || disp);
    }
    if !math_class && !mathml {
        return None;
    }
    let visible = visible_text(el);
    delimited(&visible).map(|(tex, disp)| (tex, display || disp))
}

fn rendered_math(el: ElementRef) -> bool {
    el.value().name() == "mjx-container"
        || class_in(
            el,
            &[
                "katex",
                "katex-display",
                "mathjax",
                "mathjax_display",
                "mjx-container",
            ],
        )
}

fn math_display(el: ElementRef) -> bool {
    let mut cur = Some(el);
    for _ in 0..4 {
        let Some(node) = cur else {
            break;
        };
        if node.value().classes().any(|c| {
            c.eq_ignore_ascii_case("display")
                || c.eq_ignore_ascii_case("katex-display")
                || c.eq_ignore_ascii_case("mathjax_display")
                || ascii_ends(c, "-display")
                || ascii_ends(c, "_display")
        }) || node.attr("display").is_some_and(|d| {
            let d = d.trim();
            d.eq_ignore_ascii_case("block") || d.eq_ignore_ascii_case("true")
        }) {
            return true;
        }
        cur = node.parent().and_then(ElementRef::wrap);
    }
    el.attr("type")
        .is_some_and(|t| t.to_ascii_lowercase().contains("display"))
}

fn tex_script(el: ElementRef) -> Option<(String, bool)> {
    for script in el.descendent_elements() {
        if script.value().name() != "script" {
            continue;
        }
        let typ = script.attr("type").unwrap_or("");
        if !ascii_starts(typ, "math/tex") {
            continue;
        }
        let tex: String = script.text().collect();
        let tex = tex.trim();
        if tex.is_empty() {
            continue;
        }
        let display = typ.to_ascii_lowercase().contains("display");
        return Some((tex.to_string(), display));
    }
    None
}

fn annotation_tex(el: ElementRef) -> Option<String> {
    for ann in el.descendent_elements() {
        if ann.value().name() != "annotation" {
            continue;
        }
        let enc = ann.attr("encoding").unwrap_or("").to_ascii_lowercase();
        if !enc.contains("tex") {
            continue;
        }
        let tex: String = ann.text().collect();
        let tex = tex.trim();
        if !tex.is_empty() {
            return Some(tex.to_string());
        }
    }
    None
}

fn visible_text(el: ElementRef) -> String {
    let mut out = String::new();
    append_visible(el, &mut out);
    out
}

fn append_visible(el: ElementRef, out: &mut String) {
    for child in el.children() {
        if let Some(inner) = ElementRef::wrap(child) {
            if hide_math(inner) {
                continue;
            }
            append_visible(inner, out);
        } else if let Some(text) = child.value().as_text() {
            out.push_str(text);
        }
    }
}

fn hide_math(el: ElementRef) -> bool {
    if el
        .attr("aria-hidden")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        || matches!(el.value().name(), "script" | "style" | "svg")
    {
        return true;
    }
    class_in(el, &["katex", "katex-html", "mathjax", "mjx-container"])
}

pub(super) fn ascii_starts(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn ascii_ends(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.as_bytes()[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn delimited(s: &str) -> Option<(String, bool)> {
    for (open, close, display) in [
        ("\\[", "\\]", true),
        ("\\(", "\\)", false),
        ("$$", "$$", true),
    ] {
        if let Some(tex) = between(s, open, close) {
            return Some((tex, display));
        }
    }
    single_dollar(s).map(|tex| (tex, false))
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let rest = &s[s.find(open)? + open.len()..];
    let tex = rest[..rest.find(close)?].trim();
    (!tex.is_empty()).then(|| tex.to_string())
}

fn single_dollar(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && bytes.get(i + 1) != Some(&b'$') {
            let rest = &s[i + 1..];
            let j = rest.find('$')?;
            if rest.as_bytes().get(j + 1) == Some(&b'$') {
                return None;
            }
            let tex = rest[..j].trim();
            if !tex.is_empty() && !tex.contains('\n') {
                return Some(tex.to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// TeX, and whether it is displayed. One pair of delimiters is taken off
/// when an annotation or a script included them.
pub(super) fn bare_tex(tex: &str, display: bool) -> Option<(String, bool)> {
    let tex = tex.trim();
    if tex.is_empty() {
        return None;
    }
    for (open, close, disp) in [
        ("\\[", "\\]", true),
        ("$$", "$$", true),
        ("\\(", "\\)", false),
    ] {
        if let Some(inner) = tex.strip_prefix(open).and_then(|s| s.strip_suffix(close)) {
            let inner = inner.trim();
            if !inner.is_empty() {
                return Some((inner.to_string(), display || disp));
            }
        }
    }
    if let Some(inner) = tex.strip_prefix('$').and_then(|s| s.strip_suffix('$')) {
        let inner = inner.trim();
        if !inner.is_empty() && !inner.contains('$') {
            return Some((inner.to_string(), display));
        }
    }
    Some((tex.to_string(), display))
}
