//! The front matter of a markdown file: its title, authors and date, what
//! sets it as slides, and the LaTeX it includes, in YAML between `---`
//! lines, or in TOML between `+++` lines.

use crate::cite;
use serde_json::Value;

/// YAML's markers of a block scalar, whose lines follow.
const BLOCK_SCALAR: [&str; 5] = ["", "|", ">", "|-", ">-"];

/// How slides are told apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Split {
    /// At rules and headings of the slide level, as pandoc does.
    Headings,
    /// At rules only, as Marp does.
    Rules,
}

#[derive(Debug, Default)]
pub(super) struct Front {
    pub(super) title: Option<String>,
    pub(super) subtitle: Option<String>,
    pub(super) author: Option<String>,
    pub(super) date: Option<String>,
    pub(super) slides: Option<Split>,
    pub(super) slide_level: Option<u8>,
    /// `header-includes`, one line of LaTeX each.
    pub(super) header: Vec<String>,
    /// All of it, as YAML reads it, for what is read no other way.
    pub(super) meta: Value,
}

/// The front matter, and what follows it.
pub(super) fn front_matter(text: &str) -> (Front, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if let Some(toml) = toml_front_matter(text) {
        return toml;
    }
    let mut front = Front::default();
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (front, text);
    };
    let mut end = None;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if matches!(line.trim_end(), "---" | "...") {
            end = Some((at, at + line.len()));
            break;
        }
        at += line.len();
    }
    let Some((yaml_end, body_start)) = end else {
        return (front, text);
    };
    let yaml = &rest[..yaml_end];
    front.meta = cite::yaml(yaml).unwrap_or_default();

    // Top-level keys, each with its value and the indented lines under it.
    let mut entries: Vec<(&str, String, Vec<&str>)> = Vec::new();
    for line in yaml.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t', '-'])
            && let Some((key, value)) = line.split_once(':')
        {
            entries.push((key.trim(), value.trim().to_string(), Vec::new()));
        } else if let Some(last) = entries.last_mut() {
            last.2.push(line);
        }
    }
    const SLIDE_FORMATS: [&str; 7] = [
        "beamer",
        "ioslides",
        "slidy",
        "revealjs",
        "xaringan",
        "powerpoint",
        "pptx",
    ];
    for (key, value, nested) in &entries {
        let all = std::iter::once(value.as_str()).chain(nested.iter().copied());
        match *key {
            "title" => front.title = scalar(value, nested),
            "subtitle" => front.subtitle = scalar(value, nested),
            "date" => front.date = scalar(value, nested),
            "author" => front.author = authors(value, nested),
            "header-includes" => {
                front.header = all
                    .map(|l| {
                        let l = l.trim();
                        unquote(l.strip_prefix("- ").unwrap_or(l))
                    })
                    .filter(|l| !BLOCK_SCALAR.contains(&l.as_str()))
                    .collect();
            }
            "marp" if value == "true" => front.slides = Some(Split::Rules),
            "output" | "format" => {
                let text: Vec<&str> = all.collect();
                let text = text.join("\n");
                if SLIDE_FORMATS.iter().any(|f| text.contains(f)) {
                    front.slides = front.slides.or(Some(Split::Headings));
                }
                front.slide_level = text.lines().find_map(|l| {
                    let (_, n) = l.trim().split_once("slide_level:")?;
                    n.trim().parse().ok()
                });
            }
            _ => {}
        }
    }
    (front, &rest[body_start..])
}

/// Front matter between `+++` lines, in TOML, as Hugo and Zola write it,
/// and what follows it. Only its title and date are read: none of it is
/// pandoc's.
fn toml_front_matter(text: &str) -> Option<(Front, &str)> {
    let rest = text
        .strip_prefix("+++\n")
        .or_else(|| text.strip_prefix("+++\r\n"))?;
    let mut front = Front::default();
    let mut at = 0;
    // Keys before the first `[table]` are the document's own.
    let mut top = true;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "+++" {
            return Some((front, &rest[at + line.len()..]));
        }
        at += line.len();
        top &= !trimmed.starts_with('[');
        let Some((key, value)) = trimmed.split_once('=').filter(|_| top) else {
            continue;
        };
        let value = Some(toml_value(value)).filter(|v| !v.is_empty());
        match key.trim() {
            "title" => front.title = value,
            "date" => front.date = value,
            _ => {}
        }
    }
    None
}

/// A TOML value on one line: within its quotes, or as it is, less any
/// comment after it.
fn toml_value(value: &str) -> String {
    let value = value.trim();
    for q in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(q) {
            // A basic string's quote may be escaped; a literal's may not.
            let mut escaped = false;
            let end = rest.find(|c| {
                let close = c == q && !escaped;
                escaped = q == '"' && c == '\\' && !escaped;
                close
            });
            return rest[..end.unwrap_or(rest.len())].to_string();
        }
    }
    value
        .split_once('#')
        .map_or(value, |(v, _)| v)
        .trim()
        .to_string()
}

/// A YAML scalar: quoted, plain, or a block under `|` or `>`.
fn scalar(value: &str, nested: &[&str]) -> Option<String> {
    let value = if BLOCK_SCALAR.contains(&value) {
        let lines: Vec<&str> = nested.iter().map(|l| l.trim()).collect();
        lines.join(" ")
    } else {
        value.to_string()
    };
    let value = unquote(&value);
    (!value.is_empty()).then_some(value)
}

/// Authors as one line, from a scalar or a list of names.
fn authors(value: &str, nested: &[&str]) -> Option<String> {
    if !value.is_empty() {
        return scalar(value, nested);
    }
    let names: Vec<String> = nested
        .iter()
        .filter_map(|l| {
            let item = l.trim_start().strip_prefix('-')?.trim();
            let name = item.strip_prefix("name:").unwrap_or(item);
            Some(unquote(name.trim()))
        })
        .filter(|n| !n.is_empty())
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The character and length of the code fence `line` opens, if it opens
/// one: a backtick fence's info has no backtick, so ```` ```x``` ```` is
/// inline code.
pub(super) fn open_fence(line: &str) -> Option<(char, usize)> {
    let c = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = line.chars().take_while(|&x| x == c).count();
    (len >= 3 && !(c == '`' && line[len..].contains('`'))).then_some((c, len))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    for q in ['"', '\''] {
        if let Some(inner) = s.strip_prefix(q).and_then(|s| s.strip_suffix(q)) {
            return inner.to_string();
        }
    }
    s.to_string()
}
