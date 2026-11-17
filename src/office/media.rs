//! Pictures in a package, written aside for the markdown to name, or named
//! by their parts.

use super::Pictures;
use super::inline::escape;
use super::package::{Package, Rels};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The pictures written aside, and those left out.
pub(super) struct Media {
    pictures: Pictures,
    dir: Option<tempfile::TempDir>,
    /// By part, the path each was written to.
    written: HashMap<String, String>,
    /// Parts left out, each counted once however often it is shown.
    unread: HashSet<String>,
    /// Formats left out, as `emf`.
    formats: Vec<String>,
}

impl Media {
    pub fn new(pictures: Pictures) -> Self {
        Media {
            pictures,
            dir: None,
            written: HashMap::new(),
            unread: HashSet::new(),
            formats: Vec::new(),
        }
    }

    /// The picture in the part `name`: the path the markdown names it by,
    /// where it was written or its part, or `None` if typst cannot read it.
    fn save(&mut self, pkg: &mut Package, name: &str) -> Option<String> {
        if let Some(path) = self.written.get(name) {
            return Some(path.clone());
        }
        if !crate::render::image(Path::new(name)) {
            if self.unread.insert(name.to_string())
                && let Some((_, ext)) = name.rsplit_once('.')
            {
                let ext = ext.to_ascii_lowercase();
                if !self.formats.contains(&ext) {
                    self.formats.push(ext);
                }
            }
            return None;
        }
        if self.pictures == Pictures::Named {
            return Some(name.to_string());
        }
        if self.dir.is_none() {
            self.dir = tempfile::tempdir().ok();
        }
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        let path = self.dir.as_ref()?.path().join(format!(
            "image{}.{}",
            self.written.len() + 1,
            ext.to_ascii_lowercase()
        ));
        pkg.copy(name, &path)?;
        let path = path.to_string_lossy().into_owned();
        self.written.insert(name.to_string(), path.clone());
        Some(path)
    }

    /// The picture relationship `id` of `rels` leads to: the path the
    /// markdown names it by, or `None` if typst cannot read it.
    pub fn file(&mut self, pkg: &mut Package, rels: &Rels, id: &str) -> Option<String> {
        let rel = rels.get(id).filter(|r| !r.external)?;
        self.save(pkg, &rel.target)
    }

    /// The picture relationship `id` of `rels` leads to, as markdown, or
    /// empty if it cannot be shown.
    pub fn image(&mut self, pkg: &mut Package, rels: &Rels, id: &str, alt: &str) -> String {
        self.file(pkg, rels, id).map_or_else(String::new, |path| {
            format!("![{}](<{path}>)", escape(alt.trim()))
        })
    }

    /// What was left out, for the status line.
    pub fn notes(&self) -> Vec<String> {
        if self.unread.is_empty() {
            return Vec::new();
        }
        let what = if self.formats.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.formats.join(", "))
        };
        let pictures = super::count(self.unread.len(), "picture");
        vec![format!(
            "{pictures} in a format typst cannot read left out{what}"
        )]
    }

    /// Where the pictures were written, kept while it is.
    pub fn into_dir(self) -> Option<tempfile::TempDir> {
        self.dir
    }
}
