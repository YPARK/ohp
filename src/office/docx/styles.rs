//! What a document's styles and list numbering say about its paragraphs:
//! which are headings, code or quotes, and which lists are numbered.

use super::super::package::{Package, Rels, first};
use super::super::xml::{attr, child, children, is, parse, val};
use std::collections::HashMap;

/// Following `basedOn` no further than this.
const MOST_BASES: usize = 16;

struct Style {
    /// Its name, in lower case, as `heading 1`.
    name: String,
    based: Option<String>,
    /// An outline level, 1 for the top.
    outline: Option<u8>,
    /// The list it numbers its paragraphs in, and the level.
    list: Option<(String, u8)>,
}

/// What a paragraph's style makes it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Role {
    Heading(u8),
    Title,
    Subtitle,
    Code,
    Quote,
    Body,
}

#[derive(Default)]
pub(super) struct Styles {
    by_id: HashMap<String, Style>,
}

impl Styles {
    pub fn read(pkg: &mut Package, rels: &Rels) -> Self {
        let Some(text) = first(rels, "styles").and_then(|p| pkg.text(&p)) else {
            return Self::default();
        };
        let Ok(doc) = parse(&text) else {
            return Self::default();
        };
        let by_id = doc
            .root_element()
            .children()
            .filter(|n| is(*n, "w:style"))
            .filter_map(|n| {
                let id = attr(n, "w:styleId")?.to_string();
                let name = val(n, "w:name").unwrap_or(&id).to_ascii_lowercase();
                let ppr = child(n, "w:pPr");
                let outline = ppr
                    .and_then(|p| val(p, "w:outlineLvl"))
                    .and_then(|v| v.parse::<u8>().ok())
                    .filter(|v| *v < 9)
                    .map(|v| v + 1);
                let list = ppr.and_then(list_of);
                let based = val(n, "w:basedOn").map(str::to_string);
                Some((
                    id,
                    Style {
                        name,
                        based,
                        outline,
                        list,
                    },
                ))
            })
            .collect();
        Styles { by_id }
    }

    /// The styles `id` is, from itself to those it is based on.
    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut next = Some(id);
        while let Some(id) = next {
            let Some(style) = self.by_id.get(id) else {
                break;
            };
            out.push(style);
            if out.len() >= MOST_BASES {
                break;
            }
            next = style.based.as_deref();
        }
        out
    }

    /// What the paragraph style `id` makes a paragraph.
    pub fn role(&self, id: &str) -> Role {
        for style in self.chain(id) {
            let name = style.name.as_str();
            if let Some(level) = name
                .strip_prefix("heading ")
                .and_then(|n| n.trim().parse::<u8>().ok())
            {
                return Role::Heading(level.clamp(1, 6));
            }
            match name {
                "title" => return Role::Title,
                "subtitle" => return Role::Subtitle,
                _ => {}
            }
            if ["code", "source", "verbatim", "preformatted", "plain text"]
                .iter()
                .any(|w| name.contains(w))
            {
                return Role::Code;
            }
            if name.contains("quote") || name == "block text" {
                return Role::Quote;
            }
            if let Some(level) = style.outline {
                return Role::Heading(level.min(6));
            }
        }
        Role::Body
    }

    /// The list the style `id` numbers paragraphs in, and the level.
    pub fn list(&self, id: &str) -> Option<(String, u8)> {
        self.chain(id).into_iter().find_map(|s| s.list.clone())
    }

    /// How the character style `id` sets text: bold, italic, code.
    pub fn character(&self, id: &str) -> (bool, bool, bool) {
        let mut out = (false, false, false);
        for style in self.chain(id) {
            let name = style.name.as_str();
            out.0 |= name.contains("strong");
            out.1 |= name.contains("emphasis");
            out.2 |= name.contains("code") || name.contains("verbatim");
        }
        out
    }
}

/// The list a paragraph's properties number it in, and the level.
pub(super) fn list_of(ppr: roxmltree::Node) -> Option<(String, u8)> {
    let num = child(ppr, "w:numPr")?;
    let id = val(num, "w:numId")?.to_string();
    let level = val(num, "w:ilvl")
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(0)
        .min(8);
    Some((id, level))
}

/// Which levels of which lists are numbered, rather than bulleted.
#[derive(Default)]
pub(super) struct Numbering {
    /// By a list's id, its abstract list's.
    lists: HashMap<String, String>,
    /// By an abstract list and level, its number format.
    formats: HashMap<(String, u8), String>,
}

impl Numbering {
    pub fn read(pkg: &mut Package, rels: &Rels) -> Self {
        let Some(text) = first(rels, "numbering").and_then(|p| pkg.text(&p)) else {
            return Self::default();
        };
        let Ok(doc) = parse(&text) else {
            return Self::default();
        };
        let root = doc.root_element();
        let mut out = Self::default();
        for abs in children(root, "w:abstractNum") {
            let Some(id) = attr(abs, "w:abstractNumId") else {
                continue;
            };
            for lvl in children(abs, "w:lvl") {
                let level = attr(lvl, "w:ilvl").and_then(|v| v.parse::<u8>().ok());
                if let (Some(level), Some(fmt)) = (level, val(lvl, "w:numFmt")) {
                    out.formats.insert((id.to_string(), level), fmt.to_string());
                }
            }
        }
        for num in children(root, "w:num") {
            if let (Some(id), Some(abs)) = (attr(num, "w:numId"), val(num, "w:abstractNumId")) {
                out.lists.insert(id.to_string(), abs.to_string());
            }
        }
        out
    }

    /// Whether the level `level` of the list `id` is numbered.
    pub fn ordered(&self, id: &str, level: u8) -> bool {
        let Some(abs) = self.lists.get(id) else {
            return false;
        };
        self.formats
            .get(&(abs.clone(), level))
            .is_some_and(|f| f != "bullet" && f != "none")
    }
}
