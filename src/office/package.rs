//! An Office Open XML package: a zip of parts, which name each other
//! through relationships.

use std::collections::HashMap;
use std::io::Read as _;

/// No part is read past this, so a zip bomb runs out here.
const MOST_PART: u64 = 256 << 20;

pub(super) struct Package {
    zip: zip::ZipArchive<std::fs::File>,
}

/// A relationship: the part, or the address, it leads to.
#[derive(Clone, Debug)]
pub(super) struct Rel {
    pub target: String,
    pub external: bool,
    /// The last segment of its type, as `image`, `hyperlink` or `notesSlide`.
    pub kind: String,
}

/// A part's relationships, by their ids.
pub(super) type Rels = HashMap<String, Rel>;

impl Package {
    /// The Word or PowerPoint file at `path`.
    pub fn read(path: &std::path::Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let zip = zip::ZipArchive::new(file).map_err(|e| {
            anyhow::anyhow!("{} is not a Word or PowerPoint file: {e}", path.display())
        })?;
        Ok(Package { zip })
    }

    /// The part `name` written to the file `to`, streamed, not held.
    pub fn copy(&mut self, name: &str, to: &std::path::Path) -> Option<()> {
        let entry = self.zip.by_name(name).ok()?;
        let mut file = std::fs::File::create(to).ok()?;
        std::io::copy(&mut entry.take(MOST_PART), &mut file).ok()?;
        Some(())
    }

    /// The part `name`, as bytes.
    pub fn bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        let entry = self.zip.by_name(name).ok()?;
        let mut out = Vec::new();
        entry.take(MOST_PART).read_to_end(&mut out).ok()?;
        Some(out)
    }

    /// The part `name`, as text.
    pub fn text(&mut self, name: &str) -> Option<String> {
        let bytes = self.bytes(name)?;
        Some(match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        })
    }

    /// The relationships of the part `name`; of the package, for `""`.
    pub fn rels(&mut self, name: &str) -> Rels {
        let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
        let rels = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let Some(text) = self.text(&rels) else {
            return Rels::new();
        };
        let Ok(doc) = super::xml::parse(&text) else {
            return Rels::new();
        };
        doc.root_element()
            .children()
            .filter(|n| n.tag_name().name() == "Relationship")
            .filter_map(|n| {
                let id = n.attribute("Id")?;
                let target = n.attribute("Target")?;
                let external = n.attribute("TargetMode") == Some("External");
                let kind = n.attribute("Type")?.rsplit('/').next()?.to_string();
                let target = if external {
                    target.to_string()
                } else {
                    resolve(dir, target)
                };
                Some((
                    id.to_string(),
                    Rel {
                        target,
                        external,
                        kind,
                    },
                ))
            })
            .collect()
    }

    /// The package's main part, the document or the presentation, else
    /// `fallback`.
    pub fn main(&mut self, fallback: &str) -> String {
        self.rels("")
            .into_values()
            .find(|r| r.kind == "officeDocument" && !r.external)
            .map_or_else(|| fallback.to_string(), |r| r.target)
    }

    /// The title and author the package's properties give.
    pub fn core(&mut self) -> (Option<String>, Option<String>) {
        let Some(text) = self.text("docProps/core.xml") else {
            return (None, None);
        };
        let Ok(doc) = super::xml::parse(&text) else {
            return (None, None);
        };
        let field = |name: &str| {
            doc.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };
        (field("title"), field("creator"))
    }
}

/// The first internal relationship in `rels` of `kind`: the part it leads to.
pub(super) fn first(rels: &Rels, kind: &str) -> Option<String> {
    let mut found: Vec<(&String, &Rel)> = rels
        .iter()
        .filter(|(_, r)| r.kind == kind && !r.external)
        .collect();
    found.sort_by(|a, b| a.0.cmp(b.0));
    found.first().map(|(_, r)| r.target.clone())
}

/// `target`, named from the directory `dir`, as a part's name.
pub(super) fn resolve(dir: &str, target: &str) -> String {
    let target = target.split(['#', '?']).next().unwrap_or(target);
    let mut parts: Vec<&str> = Vec::new();
    let target = match target.strip_prefix('/') {
        Some(abs) => abs,
        None => {
            parts.extend(dir.split('/'));
            target
        }
    };
    parts.extend(target.split('/'));
    let mut out: Vec<&str> = Vec::new();
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    percent_encoding::percent_decode_str(&out.join("/"))
        .decode_utf8_lossy()
        .into_owned()
}
