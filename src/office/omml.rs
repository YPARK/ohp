//! Office Math (OMML, ECMA-376 part 1, §22.1) as LaTeX, as markdown writes
//! it between dollars.
//!
//! Symbols are kept as the characters they are, which ohp's math sets as
//! LaTeX's names for them. What is not known is read for its text.

use super::xml::{attr, child, children, chosen, is, on};
use roxmltree::Node;

/// Deeper than this, an equation is read for its text alone.
const MOST_DEPTH: usize = 64;

/// The equations in `m:oMathPara`, or `m:oMath` on its own, as LaTeX.
pub(super) fn latex(n: Node) -> String {
    let eqs: Vec<String> = if is(n, "m:oMathPara") {
        children(n, "m:oMath").map(|m| args(m, 0)).collect()
    } else {
        vec![args(n, 0)]
    };
    let eqs: Vec<String> = eqs
        .into_iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    match eqs.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        many => format!(
            "\\begin{{gathered}}{}\\end{{gathered}}",
            many.join(" \\\\ ")
        ),
    }
}

/// The elements under `n`, one after another.
fn args(n: Node, depth: usize) -> String {
    let mut out = String::new();
    for c in n.children().filter(Node::is_element) {
        let s = node(c, depth + 1);
        // A command's name runs on into a letter after it.
        if out.ends_with(|c: char| c.is_ascii_alphabetic())
            && ends_in_command(&out)
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
        {
            out.push(' ');
        }
        out.push_str(&s);
    }
    out
}

fn ends_in_command(s: &str) -> bool {
    let word = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    word.ends_with('\\') && !word.ends_with("\\\\")
}

/// The argument `name` of `n`, as `m:num`.
fn arg(n: Node, name: &str, depth: usize) -> String {
    child(n, name).map_or_else(String::new, |a| args(a, depth))
}

/// The property `name` of `n`'s properties `pr`, as `m:chr`.
fn prop<'a>(n: Node<'a, '_>, pr: &str, name: &str) -> Option<&'a str> {
    let p = child(child(n, pr)?, name)?;
    attr(p, "m:val").or_else(|| attr(p, "w:val"))
}

fn flag(n: Node, pr: &str, name: &str) -> bool {
    on(child(n, pr).and_then(|p| child(p, name)))
}

/// A character property: `None` if not given, `Some("")` if given empty,
/// which is none.
fn chr<'a>(n: Node<'a, '_>, pr: &str, name: &str) -> Option<&'a str> {
    let p = child(child(n, pr)?, name)?;
    Some(attr(p, "m:val").unwrap_or(""))
}

fn node(n: Node, depth: usize) -> String {
    if depth > MOST_DEPTH {
        return text_of(n);
    }
    let a = |name: &str| arg(n, name, depth);
    let name = n.tag_name().name();
    let math = is(n, &format!("m:{name}"));
    if !math {
        return if is(n, "w:r") {
            run(n)
        } else if is(n, "mc:AlternateContent") {
            chosen(n).map_or_else(String::new, |c| args(c, depth))
        } else if is(n, "w:del") || is(n, "w:rPr") {
            String::new()
        } else {
            args(n, depth)
        };
    }
    match name {
        "r" => run(n),
        "f" => match prop(n, "m:fPr", "m:type") {
            Some("noBar") => format!("\\binom{{{}}}{{{}}}", a("m:num"), a("m:den")),
            Some("lin") => format!("{{{}}}/{{{}}}", a("m:num"), a("m:den")),
            _ => format!("\\frac{{{}}}{{{}}}", a("m:num"), a("m:den")),
        },
        "sSup" => format!("{}^{{{}}}", base(&a("m:e")), a("m:sup")),
        "sSub" => format!("{}_{{{}}}", base(&a("m:e")), a("m:sub")),
        "sSubSup" => format!("{}_{{{}}}^{{{}}}", base(&a("m:e")), a("m:sub"), a("m:sup")),
        "sPre" => format!(
            "{{}}_{{{}}}^{{{}}}{}",
            a("m:sub"),
            a("m:sup"),
            base(&a("m:e"))
        ),
        "rad" => {
            let deg = a("m:deg");
            if flag(n, "m:radPr", "m:degHide") || deg.trim().is_empty() {
                format!("\\sqrt{{{}}}", a("m:e"))
            } else {
                format!("\\sqrt[{deg}]{{{}}}", a("m:e"))
            }
        }
        "nary" => nary(n, depth),
        "d" => delimited(n, depth),
        "func" => func(n, depth),
        "limLow" => limit(n, depth, false),
        "limUpp" => limit(n, depth, true),
        "acc" => {
            let c = chr(n, "m:accPr", "m:chr").unwrap_or("\u{302}");
            match accent(c) {
                Some(cmd) => format!("\\{cmd}{{{}}}", a("m:e")),
                None => format!("\\overset{{{}}}{{{}}}", tex_text(c), a("m:e")),
            }
        }
        "bar" => match prop(n, "m:barPr", "m:pos") {
            Some("top") => format!("\\overline{{{}}}", a("m:e")),
            _ => format!("\\underline{{{}}}", a("m:e")),
        },
        "groupChr" => {
            let c = chr(n, "m:groupChrPr", "m:chr").unwrap_or("\u{23DF}");
            let top = prop(n, "m:groupChrPr", "m:pos") == Some("top");
            match (c, top) {
                ("\u{23DF}", _) => format!("\\underbrace{{{}}}", a("m:e")),
                ("\u{23DE}", _) => format!("\\overbrace{{{}}}", a("m:e")),
                (c, true) => format!("\\overset{{{}}}{{{}}}", tex_text(c), a("m:e")),
                (c, false) => format!("\\underset{{{}}}{{{}}}", tex_text(c), a("m:e")),
            }
        }
        "eqArr" => {
            let rows: Vec<String> = children(n, "m:e").map(|e| args(e, depth)).collect();
            format!("\\begin{{aligned}}{}\\end{{aligned}}", rows.join(" \\\\ "))
        }
        "m" => {
            let rows: Vec<String> = children(n, "m:mr")
                .map(|r| {
                    children(r, "m:e")
                        .map(|e| args(e, depth))
                        .collect::<Vec<_>>()
                        .join(" & ")
                })
                .collect();
            format!("\\begin{{matrix}}{}\\end{{matrix}}", rows.join(" \\\\ "))
        }
        "phant" => {
            if prop(n, "m:phantPr", "m:show").is_some_and(|v| matches!(v, "0" | "off" | "false")) {
                String::new()
            } else {
                a("m:e")
            }
        }
        // Properties, read where they belong, and the controls of a run.
        _ if name.ends_with("Pr") => String::new(),
        _ => args(n, depth),
    }
}

/// A script's base, braced when more than one atom, as `{x+1}^2`.
fn base(e: &str) -> String {
    let e = e.trim();
    if e.chars().count() == 1 || e.starts_with('\\') && !e[1..].contains(['\\', ' ', '{']) {
        e.to_string()
    } else {
        format!("{{{e}}}")
    }
}

fn nary(n: Node, d: usize) -> String {
    let c = chr(n, "m:naryPr", "m:chr")
        .filter(|c| !c.is_empty())
        .unwrap_or("∫");
    let op = match c {
        "∑" => "\\sum",
        "∏" => "\\prod",
        "∐" => "\\coprod",
        "∫" => "\\int",
        "∬" => "\\iint",
        "∭" => "\\iiint",
        "∮" => "\\oint",
        "⋃" => "\\bigcup",
        "⋂" => "\\bigcap",
        "⋁" => "\\bigvee",
        "⋀" => "\\bigwedge",
        "⨁" => "\\bigoplus",
        "⨂" => "\\bigotimes",
        other => return format!("{}{}", tex_text(other), limits(n, d)),
    };
    format!("{op}{}", limits(n, d))
}

/// An n-ary's limits and operand.
fn limits(n: Node, d: usize) -> String {
    let mut out = String::new();
    let sub = arg(n, "m:sub", d);
    let sup = arg(n, "m:sup", d);
    if !flag(n, "m:naryPr", "m:subHide") && !sub.trim().is_empty() {
        out.push_str(&format!("_{{{sub}}}"));
    }
    if !flag(n, "m:naryPr", "m:supHide") && !sup.trim().is_empty() {
        out.push_str(&format!("^{{{sup}}}"));
    }
    out.push(' ');
    out.push_str(&arg(n, "m:e", d));
    out
}

fn delimited(n: Node, d: usize) -> String {
    let beg = chr(n, "m:dPr", "m:begChr").unwrap_or("(");
    let end = chr(n, "m:dPr", "m:endChr").unwrap_or(")");
    let sep = chr(n, "m:dPr", "m:sepChr").unwrap_or("|");
    let sep = format!(" {} ", fence(sep).unwrap_or_else(|| tex_text(sep)));
    let elems: Vec<String> = children(n, "m:e").map(|e| args(e, d)).collect();
    let open = fence(beg).unwrap_or_else(|| ".".into());
    let close = fence(end).unwrap_or_else(|| ".".into());
    format!("\\left{open} {} \\right{close}", elems.join(&sep))
}

/// A delimiter as LaTeX writes it after `\left`, or `None` for none.
fn fence(c: &str) -> Option<String> {
    Some(
        match c {
            "" => return None,
            "{" => "\\{",
            "}" => "\\}",
            "⟨" | "〈" => "\\langle",
            "⟩" | "〉" => "\\rangle",
            "⌊" => "\\lfloor",
            "⌋" => "\\rfloor",
            "⌈" => "\\lceil",
            "⌉" => "\\rceil",
            "‖" => "\\|",
            other => other,
        }
        .to_string(),
    )
}

/// The functions LaTeX names, as `\sin`.
const FUNCTIONS: [&str; 32] = [
    "sin", "cos", "tan", "sec", "csc", "cot", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "coth", "log", "ln", "lg", "exp", "det", "dim", "ker", "deg", "arg", "gcd", "Pr", "lim",
    "liminf", "limsup", "max", "min", "sup", "inf", "hom",
];

fn func(n: Node, d: usize) -> String {
    let name = child(n, "m:fName");
    let e = arg(n, "m:e", d);
    // A limit's name is in a lower limit: lim under n→∞.
    if let Some(low) = name.and_then(|f| child(f, "m:limLow")) {
        return format!("{} {e}", limit(low, d, false));
    }
    let text = name.map_or_else(String::new, |f| text_of(f).trim().to_string());
    if FUNCTIONS.contains(&text.as_str()) {
        format!("\\{text} {e}")
    } else if !text.is_empty() && text.chars().all(char::is_alphabetic) {
        format!("\\operatorname{{{text}}} {e}")
    } else {
        format!("{} {e}", name.map_or_else(String::new, |f| args(f, d)))
    }
}

fn limit(n: Node, d: usize, upper: bool) -> String {
    let e = arg(n, "m:e", d);
    let lim = arg(n, "m:lim", d);
    let name = child(n, "m:e").map_or_else(String::new, |e| text_of(e).trim().to_string());
    if !upper && FUNCTIONS.contains(&name.as_str()) {
        format!("\\{name}_{{{lim}}}")
    } else if upper {
        format!("\\overset{{{lim}}}{{{e}}}")
    } else {
        format!("\\underset{{{lim}}}{{{e}}}")
    }
}

/// An accent's character as LaTeX's command for it.
fn accent(c: &str) -> Option<&'static str> {
    Some(match c {
        "\u{302}" | "^" | "ˆ" => "hat",
        "\u{303}" | "~" | "˜" => "tilde",
        "\u{307}" | "˙" => "dot",
        "\u{308}" | "¨" => "ddot",
        "\u{20D7}" | "→" | "\u{20D1}" => "vec",
        "\u{304}" | "\u{305}" | "¯" => "bar",
        "\u{301}" | "´" => "acute",
        "\u{300}" | "`" => "grave",
        "\u{306}" | "˘" => "breve",
        "\u{30C}" | "ˇ" => "check",
        _ => return None,
    })
}

/// A run's text: normal text, or upright, as text LaTeX keeps apart.
fn run(r: Node) -> String {
    let text = text_of(r);
    if text.is_empty() {
        return text;
    }
    let pr = child(r, "m:rPr");
    let normal = pr.is_some_and(|p| on(child(p, "m:nor"))) || is(r, "w:r");
    let sty = pr
        .and_then(|p| child(p, "m:sty"))
        .and_then(|s| attr(s, "m:val"));
    let words = text.chars().any(char::is_whitespace)
        || text.chars().filter(|c| c.is_alphabetic()).count() > 1;
    if normal && words {
        return format!("\\text{{{}}}", text_escape(&text));
    }
    let tex = tex_text(&text);
    match sty {
        Some("p") if text.chars().any(char::is_alphabetic) => format!("\\mathrm{{{tex}}}"),
        Some("b") => format!("\\mathbf{{{tex}}}"),
        _ => tex,
    }
}

/// The text of the runs under `n`, `m:t` and `w:t`.
fn text_of(n: Node) -> String {
    n.descendants()
        .filter(|t| is(*t, "m:t") || is(*t, "w:t"))
        .filter_map(|t| t.text())
        .collect()
}

/// Text in math, its characters LaTeX reads as commands escaped; `&`, which
/// aligns an equation array's rows, is kept.
fn tex_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\backslash "),
            '{' | '}' | '#' | '$' | '%' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '^' => out.push_str("\\hat{}"),
            '~' => out.push_str("\\sim "),
            // A minus Word writes as a hyphen, and the other way.
            '−' => out.push('-'),
            // Word's zero-width spaces and joiners, which set nothing.
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Text in `\text{}`.
fn text_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // A dollar would end markdown's formula; the rest are LaTeX's.
            '$' | '%' | '#' | '&' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\\' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "../tests/office/omml.rs"]
mod tests;
