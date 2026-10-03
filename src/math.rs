//! LaTeX math, as markdown writes it between dollars, in typst's notation.
//!
//! Only the common part of LaTeX is known: a formula using anything else is
//! not converted, and is shown as its source instead. Symbols are written as
//! the characters they stand for, which typst sets as LaTeX does, so no
//! typst symbol names are relied on.
//!
//! Every atom is set apart by a space: typst reads `xy` as one name, where
//! LaTeX means two letters, and spaces between atoms are not shown.

use crate::markdown::string;

/// `latex` in typst's math notation, or `None` if it uses what is not known.
pub fn convert(latex: &str) -> Option<String> {
    let chars: Vec<char> = latex.chars().collect();
    let mut p = Parser { s: &chars, i: 0 };
    let (out, end) = p.seq(Context::Top)?;
    (end == End::Eof).then(|| out.trim().to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Context {
    Top,
    /// Inside braces, until the closing one.
    Group,
    /// Inside an environment, whose cells end at `&` and rows at `\\`.
    Env,
}

#[derive(Debug, PartialEq)]
enum End {
    Eof,
    Brace,
    Cell,
    Row,
    /// `\end{name}`.
    Env(String),
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    /// Atoms up to what ends `ctx`, and what ended them.
    fn seq(&mut self, ctx: Context) -> Option<(String, End)> {
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return (ctx == Context::Top).then_some((out, End::Eof));
            };
            self.i += 1;
            match c {
                '}' if ctx == Context::Group => return Some((out, End::Brace)),
                '}' => return None,
                '&' if ctx == Context::Env => return Some((out, End::Cell)),
                '\\' if self.peek() == Some('\\') => {
                    self.i += 1;
                    self.optional_arg();
                    if ctx == Context::Env {
                        return Some((out, End::Row));
                    }
                    out.push_str(" \\ ");
                }
                '\\' => {
                    let name = self.name();
                    if name == "end" {
                        let env = self.raw_group()?;
                        return (ctx == Context::Env).then_some((out, End::Env(env)));
                    }
                    let atom = self.command(&name)?;
                    push(&mut out, &atom);
                }
                '^' | '_' => {
                    let arg = self.arg()?;
                    out.push(c);
                    out.push('(');
                    out.push_str(arg.trim());
                    out.push(')');
                }
                '{' => {
                    let (inner, _) = self.seq(Context::Group)?;
                    push(&mut out, &inner);
                }
                '%' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '\'' => out.push('\''),
                c if c.is_whitespace() || c == '~' => {}
                c if c.is_ascii_digit() => {
                    let mut num = c.to_string();
                    while let Some(d) = self.peek() {
                        let decimal =
                            d == '.' && self.s.get(self.i + 1).is_some_and(char::is_ascii_digit);
                        if !(d.is_ascii_digit() || decimal) {
                            break;
                        }
                        num.push(d);
                        self.i += 1;
                    }
                    push(&mut out, &num);
                }
                c => push(&mut out, &char_atom(c)),
            }
        }
    }

    /// One argument: a braced group, a command or a character.
    fn arg(&mut self) -> Option<String> {
        self.skip_space();
        let c = self.peek()?;
        self.i += 1;
        match c {
            '{' => self.seq(Context::Group).map(|(s, _)| s),
            '\\' => {
                let name = self.name();
                self.command(&name)
            }
            c if c.is_ascii_digit() => Some(c.to_string()),
            '}' | '^' | '_' | '&' => None,
            c => Some(char_atom(c)),
        }
    }

    /// An argument of a typst function call, where a comma or semicolon
    /// would separate arguments.
    fn call_arg(&mut self) -> Option<String> {
        self.arg().map(|a| quote_separators(&a))
    }

    /// A command's name: letters, or one other character.
    fn name(&mut self) -> String {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i == start && self.peek().is_some() {
            self.i += 1;
        }
        self.s[start..self.i].iter().collect()
    }

    /// The text of a braced group, unparsed, as `\text` takes it.
    fn raw_group(&mut self) -> Option<String> {
        self.skip_space();
        if self.peek() != Some('{') {
            return None;
        }
        self.i += 1;
        let (mut depth, mut text) = (0, String::new());
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => return Some(text),
                '}' => depth -= 1,
                _ => {}
            }
            text.push(c);
        }
    }

    /// A bracketed optional argument, if there is one.
    fn optional_arg(&mut self) -> Option<String> {
        let at = self.i;
        self.skip_space();
        if self.peek() != Some('[') {
            self.i = at;
            return None;
        }
        self.i += 1;
        let start = self.i;
        while self.peek()? != ']' {
            self.i += 1;
        }
        self.i += 1;
        Some(self.s[start..self.i - 1].iter().collect())
    }

    fn command(&mut self, name: &str) -> Option<String> {
        if let Some(sym) = symbol(name) {
            return Some(sym.to_string());
        }
        if let Some(op) = operator(name) {
            return Some(op);
        }
        let call = |f: &str, arg: String| format!("{f}({arg})");
        Some(match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let (a, b) = (self.call_arg()?, self.call_arg()?);
                format!("frac({a}, {b})")
            }
            "binom" | "dbinom" | "tbinom" => {
                let (a, b) = (self.call_arg()?, self.call_arg()?);
                format!("binom({a}, {b})")
            }
            "sqrt" => match self.optional_arg() {
                Some(n) => {
                    let n = convert(&n)?;
                    format!("root({}, {})", quote_separators(&n), self.call_arg()?)
                }
                None => call("sqrt", self.call_arg()?),
            },
            "overset" | "stackrel" => {
                let (over, base) = (self.arg()?, self.arg()?);
                format!("limits({})^({})", quote_separators(&base), over.trim())
            }
            "underset" => {
                let (under, base) = (self.arg()?, self.arg()?);
                format!("limits({})_({})", quote_separators(&base), under.trim())
            }
            "text" | "textrm" | "textnormal" | "textup" | "mbox" => string(&self.raw_group()?),
            "mathbf" => format!("bold(upright({}))", self.call_arg()?),
            "textit" => call("italic", string(&self.raw_group()?)),
            "textbf" => call("bold", string(&self.raw_group()?)),
            "operatorname" => format!("op({})", string(&self.raw_group()?)),
            "left" | "right" | "middle" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr"
            | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" => {
                self.skip_space();
                if self.peek() == Some('.') {
                    self.i += 1;
                    String::new()
                } else {
                    self.arg()?
                }
            }
            "not" => match self.arg()?.as_str() {
                "=" => "≠".into(),
                "∈" => "∉".into(),
                "⊂" => "⊄".into(),
                "⊆" => "⊈".into(),
                "≡" => "≢".into(),
                _ => return None,
            },
            "begin" => {
                let env = self.raw_group()?;
                self.env(&env)?
            }
            "displaystyle" | "textstyle" | "scriptstyle" | "limits" | "nolimits" | "nonumber"
            | "notag" => String::new(),
            "label" | "tag" | "color" => {
                self.raw_group()?;
                String::new()
            }
            _ => {
                let f = accent(name).or_else(|| font(name))?;
                call(f, self.call_arg()?)
            }
        })
    }

    /// The body of `\begin{env}`, through its `\end`.
    fn env(&mut self, env: &str) -> Option<String> {
        let delim = match env {
            "matrix" | "smallmatrix" | "array" => Some("#none"),
            "pmatrix" => Some("\"(\""),
            "bmatrix" => Some("\"[\""),
            "Bmatrix" => Some("\"{\""),
            "vmatrix" => Some("\"|\""),
            "Vmatrix" => Some("\"‖\""),
            _ => None,
        };
        if env == "array" {
            self.raw_group()?;
        }
        let rows = self.rows(env)?;
        let cells = |sep: &str| -> Vec<String> {
            rows.iter()
                .map(|row| {
                    let cells: Vec<String> = row.iter().map(|c| quote_separators(c)).collect();
                    cells.join(sep)
                })
                .collect()
        };
        if let Some(delim) = delim {
            return Some(format!("mat(delim: {delim}, {})", cells(", ").join("; ")));
        }
        Some(match env {
            "cases" | "dcases" => format!("cases({})", cells(" & ").join(", ")),
            "aligned" | "align" | "align*" | "alignat" | "alignat*" | "split" | "gathered"
            | "gather" | "gather*" | "eqnarray" | "eqnarray*" | "equation" | "equation*"
            | "multline" | "multline*" => {
                let rows: Vec<String> = rows.iter().map(|row| row.join(" & ")).collect();
                rows.join(" \\ ")
            }
            _ => return None,
        })
    }

    /// Rows of cells up to `\end{env}`; a last row left empty is dropped.
    fn rows(&mut self, env: &str) -> Option<Vec<Vec<String>>> {
        let (mut rows, mut row) = (Vec::new(), Vec::new());
        loop {
            let (cell, end) = self.seq(Context::Env)?;
            row.push(cell.trim().to_string());
            match end {
                End::Cell => {}
                End::Row => rows.push(std::mem::take(&mut row)),
                End::Env(name) if name == env => {
                    if !(row.len() == 1 && row[0].is_empty()) {
                        rows.push(row);
                    }
                    return Some(rows);
                }
                _ => return None,
            }
        }
    }
}

/// Append `atom` to `out`, set apart from what is before it.
fn push(out: &mut String, atom: &str) {
    if atom.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    out.push_str(atom);
}

/// A character as typst math reads it as itself.
fn char_atom(c: char) -> String {
    match c {
        '/' | '"' | '#' | '$' | '@' => format!("\\{c}"),
        _ => c.to_string(),
    }
}

/// Commas and semicolons separate arguments in a typst function call; ones
/// that belong to the formula are quoted. Those within a nested call or a
/// string are left as they are.
fn quote_separators(s: &str) -> String {
    let (mut out, mut depth, mut in_str, mut escaped) = (String::new(), 0i32, false, false);
    for c in s.chars() {
        if in_str {
            in_str = !(c == '"' && !escaped);
            escaped = c == '\\' && !escaped;
            out.push(c);
            continue;
        }
        match c {
            '"' if !escaped => in_str = true,
            '(' if !escaped => depth += 1,
            ')' if !escaped => depth -= 1,
            ',' | ';' if depth == 0 && !escaped => {
                out.push_str(&format!("\"{c}\""));
                escaped = false;
                continue;
            }
            _ => {}
        }
        escaped = c == '\\' && !escaped;
        out.push(c);
    }
    out
}

fn operator(name: &str) -> Option<String> {
    const PLAIN: [&str; 25] = [
        "log", "ln", "lg", "exp", "sin", "cos", "tan", "sec", "csc", "cot", "arcsin", "arccos",
        "arctan", "sinh", "cosh", "tanh", "coth", "det", "dim", "ker", "hom", "deg", "arg", "gcd",
        "Pr",
    ];
    const LIMITS: [&str; 9] = [
        "lim", "liminf", "limsup", "max", "min", "sup", "inf", "argmax", "argmin",
    ];
    if PLAIN.contains(&name) {
        Some(format!("op(\"{name}\")"))
    } else if LIMITS.contains(&name) {
        let shown = match name {
            "liminf" => "lim inf",
            "limsup" => "lim sup",
            "argmax" => "arg max",
            "argmin" => "arg min",
            _ => name,
        };
        Some(format!("op(\"{shown}\", limits: #true)"))
    } else {
        None
    }
}

fn accent(name: &str) -> Option<&'static str> {
    Some(match name {
        "hat" | "widehat" => "hat",
        "tilde" | "widetilde" => "tilde",
        "bar" => "macron",
        "overline" => "overline",
        "underline" => "underline",
        "overbrace" => "overbrace",
        "underbrace" => "underbrace",
        "vec" | "overrightarrow" => "arrow",
        "dot" => "dot",
        "ddot" => "dot.double",
        "acute" => "acute",
        "grave" => "grave",
        "breve" => "breve",
        "check" => "caron",
        _ => return None,
    })
}

fn font(name: &str) -> Option<&'static str> {
    Some(match name {
        "boldsymbol" | "bm" => "bold",
        "mathrm" => "upright",
        "mathit" => "italic",
        "mathcal" => "cal",
        "mathscr" => "scr",
        "mathbb" => "bb",
        "mathsf" => "sans",
        "mathtt" => "mono",
        "mathfrak" => "frak",
        _ => return None,
    })
}

/// The character a symbol command stands for, or a typst spacing.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Greek: LaTeX's \epsilon and \phi are the lunate and open forms.
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "ϕ",
        "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        // Large operators.
        "sum" => "∑",
        "prod" => "∏",
        "coprod" => "∐",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "bigcup" => "⋃",
        "bigcap" => "⋂",
        "bigoplus" => "⨁",
        "bigotimes" => "⨂",
        "bigvee" => "⋁",
        "bigwedge" => "⋀",
        // Binary operators and relations.
        "cdot" => "⋅",
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "∙",
        "oplus" => "⊕",
        "ominus" => "⊖",
        "otimes" => "⊗",
        "odot" => "⊙",
        "cup" => "∪",
        "cap" => "∩",
        "setminus" => "∖",
        "wedge" | "land" => "∧",
        "vee" | "lor" => "∨",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "leqslant" => "⩽",
        "geqslant" => "⩾",
        "ne" | "neq" => "≠",
        "ll" => "≪",
        "gg" => "≫",
        "approx" => "≈",
        "sim" => "∼",
        "simeq" => "≃",
        "cong" => "≅",
        "equiv" => "≡",
        "propto" => "∝",
        "asymp" => "≍",
        "doteq" => "≐",
        "prec" => "≺",
        "succ" => "≻",
        "preceq" => "⪯",
        "succeq" => "⪰",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "perp" => "⊥",
        "parallel" => "∥",
        "mid" => "∣",
        "models" => "⊨",
        "vdash" => "⊢",
        // Arrows.
        "to" | "rightarrow" => "→",
        "gets" | "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔",
        "implies" => "⟹",
        "impliedby" => "⟸",
        "iff" => "⟺",
        "mapsto" => "↦",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "leadsto" => "⇝",
        "rightleftharpoons" => "⇌",
        // Delimiters.
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "vert" | "lvert" | "rvert" => "|",
        "Vert" | "lVert" | "rVert" | "|" => "‖",
        "{" | "lbrace" => "\\{",
        "}" | "rbrace" => "\\}",
        // Other symbols.
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "neg" | "lnot" => "¬",
        "emptyset" | "varnothing" => "∅",
        "ell" => "ℓ",
        "hbar" => "ℏ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "aleph" => "ℵ",
        "angle" => "∠",
        "triangle" => "△",
        "top" => "⊤",
        "bot" => "⊥",
        "intercal" => "⊺",
        "dagger" => "†",
        "ddagger" => "‡",
        "prime" => "′",
        "degree" => "°",
        "therefore" => "∴",
        "because" => "∵",
        "ldots" | "dots" | "dotsc" | "dotsb" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        "colon" => ":",
        "%" => "\"%\"",
        "&" => "\"&\"",
        "#" => "\"#\"",
        "_" => "\"_\"",
        "$" => "\"$\"",
        // Spacing.
        "," | "thinspace" => "thin",
        ":" | ">" | "medspace" => "med",
        ";" | "thickspace" => "thick",
        " " => "thin",
        "quad" => "quad",
        "qquad" => "wide",
        "!" | "negthinspace" => "",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "tests/math.rs"]
mod tests;
