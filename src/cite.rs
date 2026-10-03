//! Citations as pandoc's citeproc reads them, set by typst's bibliography.
//!
//! The bibliography is what front matter names: `bibliography:` files of
//! BibLaTeX, BibTeX, CSL JSON, CSL YAML or Hayagriva YAML, and items under
//! `references:`, cited in the style `csl:` names, a CSL file or a style
//! typst knows by name, Chicago author-date otherwise as for pandoc.
//! `nocite:`, `reference-section-title:` and `suppress-bibliography:` are
//! read as pandoc reads them.

use crate::csl;
use hayagriva::archive::ArchivedStyle;
use hayagriva::citationberg::IndependentStyle;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

/// Pandoc's default style.
const STYLE: &str = "chicago-author-date";

/// One work cited: `see @doe99, p. 33` in `[see @doe99, p. 33; @roe]`.
#[derive(Debug, Default, PartialEq)]
pub struct Item {
    pub key: String,
    pub prefix: String,
    /// What follows the key, a locator as `p. 33` most often.
    pub suffix: String,
    /// `-@doe99`: the year without the author.
    pub suppress_author: bool,
}

/// Text, with the citations in it.
#[derive(Debug, PartialEq)]
pub enum Piece<'a> {
    Text(&'a str),
    /// `[@doe99; @roe]` in brackets, or `@doe99` in the text.
    Cite {
        items: Vec<Item>,
        bracketed: bool,
    },
}

/// `text` split at its citations.
pub fn parse(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut last = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let found = if rest.starts_with('[') {
            closing(rest).and_then(|end| {
                let items = bracketed(&rest[1..end])?;
                Some((
                    Piece::Cite {
                        items,
                        bracketed: true,
                    },
                    end + 1,
                ))
            })
        } else if rest.starts_with('@')
            && !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '@' | '_'))
        {
            key(&rest[1..]).map(|(key, len)| {
                let mut taken = 1 + len;
                let mut suffix = String::new();
                // `@doe99 [p. 33]`: a locator after a citation in the text.
                let after = &rest[taken..];
                if let Some(locator) = after.strip_prefix(" [")
                    && let Some(end) = closing(&after[1..])
                    && !locator[..end - 1].contains('@')
                {
                    suffix = locator[..end - 1].trim().to_string();
                    taken += 2 + end;
                }
                let item = Item {
                    key,
                    suffix,
                    ..Item::default()
                };
                let items = vec![item];
                let cite = Piece::Cite {
                    items,
                    bracketed: false,
                };
                (cite, taken)
            })
        } else {
            None
        };
        match found {
            Some((cite, len)) => {
                if last < i {
                    pieces.push(Piece::Text(&text[last..i]));
                }
                pieces.push(cite);
                i += len;
                last = i;
            }
            None => i += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if last < text.len() {
        pieces.push(Piece::Text(&text[last..]));
    }
    pieces
}

/// Where the bracket `s` opens closes, counting brackets within.
fn closing(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// The items of a bracketed citation, if each of its `;` parts has a key.
fn bracketed(inside: &str) -> Option<Vec<Item>> {
    inside
        .split(';')
        .map(|part| {
            let at = part.char_indices().find_map(|(i, c)| {
                let before = part[..i].chars().next_back();
                (c == '@' && before.is_none_or(|b| b.is_whitespace() || b == '-')).then_some(i)
            })?;
            let (key, len) = key(&part[at + 1..])?;
            let mut prefix = &part[..at];
            let suppress_author = prefix.ends_with('-')
                && prefix[..prefix.len() - 1]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace);
            if suppress_author {
                prefix = &prefix[..prefix.len() - 1];
            }
            let suffix = part[at + 1 + len..].trim();
            let suffix = suffix.strip_prefix(',').unwrap_or(suffix).trim();
            Some(Item {
                key,
                prefix: prefix.trim().to_string(),
                suffix: suffix.to_string(),
                suppress_author,
            })
        })
        .collect()
}

/// A citation key, as pandoc reads one after `@`, and the bytes it took:
/// letters, digits and `_`, with single marks of punctuation inside, or
/// anything in braces.
fn key(s: &str) -> Option<(String, usize)> {
    if let Some(inner) = s.strip_prefix('{') {
        let end = inner.find('}')?;
        let key = inner[..end].trim();
        return (!key.is_empty()).then(|| (key.to_string(), end + 2));
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut chars = s.char_indices().peekable();
    let mut end = 0;
    while let Some((i, c)) = chars.next() {
        if word(c) {
            end = i + c.len_utf8();
        } else if end > 0
            && ":.#$%&-+?<>~/".contains(c)
            && chars.peek().is_some_and(|&(_, n)| word(n))
        {
            continue;
        } else {
            break;
        }
    }
    (end > 0).then(|| (s[..end].to_string(), end))
}

/// What front matter asks of citations, with what it names read.
#[derive(Debug, Default)]
pub struct Bibliography {
    /// Files typst reads, by name: the bibliographies, and a CSL style.
    pub files: Vec<(String, Arc<[u8]>)>,
    /// The names of the bibliographies among `files`.
    pub sources: Vec<String>,
    /// A style typst knows by name, or the name of a CSL file in `files`.
    pub style: String,
    /// The keys the bibliographies have.
    pub keys: HashSet<String>,
    /// Works listed though not cited, by key.
    pub nocite: Vec<String>,
    /// `nocite: '@*'`: every work listed.
    pub all: bool,
    pub title: Option<String>,
    /// Citations set, but no list of the works.
    pub suppress: bool,
    /// What could not be read.
    pub warnings: Vec<String>,
}

impl Bibliography {
    /// The bibliography front matter `meta` names, with paths in `base`;
    /// none where it names none.
    pub fn load(meta: &Value, base: &Path) -> Option<Self> {
        let files = match meta.get("bibliography") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        };
        let references = meta.get("references").and_then(Value::as_array);
        if files.is_empty() && references.is_none() {
            return None;
        }
        let mut bib = Bibliography::default();
        let mut csl_items: Vec<Value> = references.cloned().unwrap_or_default();
        for (i, file) in files.iter().enumerate() {
            let path = base.join(file);
            let data = match std::fs::read_to_string(&path) {
                Ok(data) => data,
                Err(e) => {
                    bib.warnings.push(format!("cannot read {file}: {e}"));
                    continue;
                }
            };
            let ext = path
                .extension()
                .map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
            match ext.as_str() {
                "bib" | "bibtex" => bib.add(&format!("/bib/{i}.bib"), data, file),
                "json" => match serde_json::from_str::<Value>(&data) {
                    Ok(Value::Array(items)) => csl_items.extend(items),
                    Ok(_) => bib.warnings.push(format!("{file} is not a CSL JSON list")),
                    Err(e) => bib.warnings.push(format!("cannot read {file}: {e}")),
                },
                "yaml" | "yml" => match yaml(&data) {
                    // CSL YAML is a list of items, or one under `references`.
                    Ok(Value::Array(items)) => csl_items.extend(items),
                    Ok(Value::Object(map)) if map.contains_key("references") => {
                        if let Some(items) = map["references"].as_array() {
                            csl_items.extend(items.iter().cloned());
                        }
                    }
                    Ok(_) => bib.add(&format!("/bib/{i}.yml"), data, file),
                    Err(e) => bib.warnings.push(format!("cannot read {file}: {e}")),
                },
                _ => bib
                    .warnings
                    .push(format!("cannot read {file}: not BibLaTeX, CSL or YAML")),
            }
        }
        if !csl_items.is_empty() {
            bib.add("/bib/csl.bib", csl::biblatex(&csl_items), "CSL references");
        }
        bib.style = bib.style(meta.get("csl").and_then(Value::as_str), base);
        match meta.get("nocite") {
            Some(Value::String(s)) => bib.nocite_from(s),
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .for_each(|s| bib.nocite_from(s)),
            _ => {}
        }
        bib.title = meta
            .get("reference-section-title")
            .and_then(Value::as_str)
            .map(str::to_string);
        bib.suppress = meta.get("suppress-bibliography").and_then(Value::as_bool) == Some(true);
        Some(bib)
    }

    /// A bibliography as typst reads it, by `name`, if it can be read.
    fn add(&mut self, name: &str, data: String, file: &str) {
        let library = if name.ends_with(".yml") {
            hayagriva::io::from_yaml_str(&data).map_err(|e| e.to_string())
        } else {
            hayagriva::io::from_biblatex_str(&data).map_err(|errors| match errors.first() {
                Some(e) => format!("{e:?}"),
                None => "not BibLaTeX".to_string(),
            })
        };
        match library {
            Ok(library) => {
                self.keys.extend(library.keys().map(str::to_string));
                self.sources.push(name.to_string());
                self.files
                    .push((name.to_string(), Arc::from(data.into_bytes())));
            }
            Err(e) => self.warnings.push(format!("cannot read {file}: {e}")),
        }
    }

    /// The style `csl` names: a CSL file in `base`, or a style typst knows
    /// by the name of the file or URL, as `apa` for `apa.csl`.
    fn style(&mut self, csl: Option<&str>, base: &Path) -> String {
        let Some(csl) = csl else {
            return STYLE.to_string();
        };
        let path = base.join(csl);
        if !csl.contains("://")
            && let Ok(xml) = std::fs::read_to_string(&path)
        {
            match IndependentStyle::from_xml(&xml) {
                Ok(_) => {
                    let name = "/bib/style.csl".to_string();
                    self.files.push((name.clone(), Arc::from(xml.into_bytes())));
                    return name;
                }
                Err(e) => self
                    .warnings
                    .push(format!("cannot use the style {csl}: {e}")),
            }
        }
        let stem = csl.trim_end_matches('/').rsplit('/').next().unwrap_or(csl);
        let stem = stem.strip_suffix(".csl").unwrap_or(stem);
        if ArchivedStyle::by_name(stem).is_some() {
            return stem.to_string();
        }
        self.warnings
            .push(format!("unknown style {csl}: citing in {STYLE}"));
        STYLE.to_string()
    }

    fn nocite_from(&mut self, s: &str) {
        for piece in parse(s) {
            if let Piece::Cite { items, .. } = piece {
                self.nocite.extend(items.into_iter().map(|i| i.key));
            }
        }
        // `@*` is no key: every work.
        if s.contains("@*") {
            self.all = true;
        }
    }
}

/// YAML as JSON's values, to be read alike.
pub fn yaml(text: &str) -> Result<Value, String> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "tests/cite.rs"]
mod tests;
