//! CSL items, as pandoc reads them from CSL JSON, CSL YAML or `references:`
//! in front matter, as BibLaTeX: typst reads its bibliographies from
//! BibLaTeX or Hayagriva YAML alone.

use serde_json::Value;

/// The items as BibLaTeX entries; an item without an `id` is left out.
pub fn biblatex(items: &[Value]) -> String {
    let mut out = String::new();
    for item in items {
        let Some(id) = item.get("id").and_then(text) else {
            continue;
        };
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
        let (entry, container) = match kind {
            "article" | "article-journal" | "article-magazine" | "article-newspaper" | "review"
            | "review-book" => ("article", "journaltitle"),
            "book" | "classic" => ("book", "booktitle"),
            "chapter" | "entry" | "entry-dictionary" | "entry-encyclopedia" => {
                ("incollection", "booktitle")
            }
            "paper-conference" => ("inproceedings", "booktitle"),
            "report" => ("report", "booktitle"),
            "thesis" => ("thesis", "booktitle"),
            "webpage" | "post" | "post-weblog" => ("online", "organization"),
            "dataset" => ("dataset", "booktitle"),
            "software" => ("software", "booktitle"),
            "manuscript" => ("unpublished", "booktitle"),
            _ => ("misc", "howpublished"),
        };
        let mut fields: Vec<(&str, String)> = Vec::new();
        for (csl, bib) in [("author", "author"), ("editor", "editor")] {
            if let Some(names) = item.get(csl).and_then(names) {
                fields.push((bib, names));
            }
        }
        if let Some(translator) = item.get("translator").and_then(names) {
            fields.push(("translator", translator));
        }
        for (csl, bib) in [
            ("title", "title"),
            ("title-short", "shorttitle"),
            ("container-title", container),
            ("collection-title", "series"),
            ("event-title", "eventtitle"),
            ("event", "eventtitle"),
            ("volume", "volume"),
            ("issue", "number"),
            ("number", "number"),
            ("collection-number", "number"),
            ("chapter-number", "chapter"),
            ("page", "pages"),
            ("number-of-pages", "pagetotal"),
            ("edition", "edition"),
            ("publisher", "publisher"),
            ("publisher-place", "location"),
            ("genre", "type"),
            ("ISBN", "isbn"),
            ("ISSN", "issn"),
            ("note", "note"),
        ] {
            if fields.iter().any(|(f, _)| *f == bib) {
                continue;
            }
            if let Some(value) = item.get(csl).and_then(text) {
                fields.push((bib, escape(&value)));
            }
        }
        for (csl, bib) in [("issued", "date"), ("accessed", "urldate")] {
            if let Some(date) = item.get(csl).and_then(date) {
                fields.push((bib, date));
            }
        }
        // Verbatim in BibLaTeX: not escaped.
        for (csl, bib) in [("DOI", "doi"), ("URL", "url")] {
            if let Some(value) = item.get(csl).and_then(text) {
                fields.push((bib, value.replace(['{', '}'], "")));
            }
        }
        out.push_str(&format!("@{entry}{{{id},\n"));
        for (field, value) in fields {
            out.push_str(&format!("  {field} = {{{value}}},\n"));
        }
        out.push_str("}\n\n");
    }
    out
}

/// A string or a number, as text.
fn text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Names joined by `and`, each as `von Last, Jr, First`, or kept whole in
/// braces where it is a literal, as an organisation's name is.
fn names(value: &Value) -> Option<String> {
    let names: Vec<String> = value
        .as_array()?
        .iter()
        .filter_map(|name| {
            if let Some(literal) = name.get("literal").and_then(text) {
                return Some(format!("{{{}}}", escape(&literal)));
            }
            let part = |key| name.get(key).and_then(text).map(|s| escape(&s));
            let particle = [part("dropping-particle"), part("non-dropping-particle")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let family = match part("family") {
                Some(f) if particle.is_empty() => f,
                Some(f) => format!("{particle} {f}"),
                None => return part("given").map(|g| format!("{{{g}}}")),
            };
            Some(
                [Some(family), part("suffix"), part("given")]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
        .collect();
    (!names.is_empty()).then(|| names.join(" and "))
}

/// A date as BibLaTeX writes one, `2020-03-15` or `2020/2021`, from any of
/// CSL's forms: `date-parts`, a `raw` or `literal` string, or the EDTF
/// string or bare year CSL YAML allows.
fn date(value: &Value) -> Option<String> {
    let parts = |parts: &Value| -> Option<String> {
        let parts: Vec<i64> = parts
            .as_array()?
            .iter()
            .filter_map(|p| p.as_i64().or_else(|| p.as_str()?.trim().parse().ok()))
            .collect();
        let (year, rest) = parts.split_first()?;
        let mut date = format!("{year:04}");
        for p in rest.iter().take(2) {
            date.push_str(&format!("-{p:02}"));
        }
        Some(date)
    };
    match value {
        Value::Number(n) => n.as_i64().map(|y| format!("{y:04}")),
        Value::String(s) => edtf(s),
        Value::Array(dates) => {
            let first = dates.first()?;
            if first.is_object() {
                return date(first);
            }
            parts(value)
        }
        Value::Object(map) => {
            if let Some(ranges) = map.get("date-parts").and_then(Value::as_array) {
                let dates: Vec<String> = ranges.iter().filter_map(parts).take(2).collect();
                return (!dates.is_empty()).then(|| dates.join("/"));
            }
            if let Some(year) = map.get("year") {
                let ymd = ["year", "month", "day"].map(|k| map.get(k).cloned());
                let ymd: Vec<Value> = ymd.into_iter().map_while(|p| p).collect();
                return parts(&Value::Array(ymd)).or_else(|| text(year));
            }
            map.get("raw")
                .or_else(|| map.get("literal"))
                .and_then(Value::as_str)
                .and_then(edtf)
        }
        _ => None,
    }
}

/// A date string BibLaTeX can read: an ISO date, or a range of them.
fn edtf(s: &str) -> Option<String> {
    let s = s.trim();
    let iso = |d: &str| {
        let parts: Vec<&str> = d.split('-').collect();
        !parts.is_empty()
            && parts.len() <= 3
            && parts[0].len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    };
    if s.split('/').all(iso) {
        return Some(s.to_string());
    }
    // A year somewhere in it, as in a raw `March 2020`.
    s.split(|c: char| !c.is_ascii_digit())
        .find(|w| w.len() == 4)
        .map(str::to_string)
}

/// Text in a BibLaTeX field, with what BibLaTeX would read as markup
/// escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '{' | '}' | '$' | '%' | '&' | '#' | '_' => {
                out.push('\\');
                out.push(c);
            }
            '\\' => out.push_str("\\textbackslash{}"),
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "tests/csl.rs"]
mod tests;
