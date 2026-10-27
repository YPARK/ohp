//! Code, inline and in blocks, fenced as markdown fences it, in the
//! language the page names.

use super::Writer;
use super::media::diagram_lang;
use scraper::ElementRef;

impl Writer {
    pub(super) fn inline_code(&mut self, el: ElementRef) {
        let text: String = el.text().collect();
        if text.is_empty() {
            return;
        }
        self.inline(&code_span(&text));
    }

    pub(super) fn code_block(&mut self, el: ElementRef) {
        let lang = diagram_lang(el)
            .map(str::to_string)
            .unwrap_or_else(|| code_lang(el));
        let text: String = el.text().collect();
        let text = trim_code(&text);
        let fence = fence_for(text);
        self.block_break();
        self.raw_line(&format!("{fence}{lang}"));
        for line in text.lines() {
            self.raw_line(line);
        }
        self.raw_line(&fence);
        self.block_break();
    }
}

fn code_lang(el: ElementRef) -> String {
    lang_of(el)
        .or_else(|| {
            el.child_elements()
                .find(|c| c.value().name() == "code")
                .and_then(lang_of)
        })
        .unwrap_or_default()
}

fn lang_of(el: ElementRef) -> Option<String> {
    let classes: Vec<&str> = el.value().classes().collect();
    for class in &classes {
        if let Some(lang) = class
            .strip_prefix("language-")
            .or_else(|| class.strip_prefix("lang-"))
        {
            let lang = clean_lang(lang);
            if !lang.is_empty() {
                return Some(lang);
            }
        }
    }
    if !classes.contains(&"sourceCode") {
        return None;
    }
    const SKIP: &[&str] = &["sourceCode", "numberLines", "number-lines"];
    classes.iter().copied().find_map(|c| {
        if SKIP.contains(&c) {
            return None;
        }
        let lang = clean_lang(c);
        (!lang.is_empty()).then_some(lang)
    })
}

fn clean_lang(lang: &str) -> String {
    lang.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '-' | '_'))
        .collect()
}

pub(super) fn trim_code(s: &str) -> &str {
    let s = s
        .strip_prefix("\r\n")
        .or_else(|| s.strip_prefix('\n'))
        .unwrap_or(s);
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

fn fence_for(code: &str) -> String {
    "`".repeat(longest(code, '`').max(2) + 1)
}

fn code_span(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ");
    let fence = "`".repeat(longest(&text, '`') + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

fn longest(s: &str, c: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for ch in s.chars() {
        if ch == c {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}
