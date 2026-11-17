//! A PowerPoint deck laid out as its slides are: each slide one page of the
//! deck's own size, each picture, text box, table, shape and line where it
//! is and as large as it is, text at its sizes, in its fonts and colours,
//! over the shapes its master and layout put on every slide.
//!
//! - [`theme`]: the theme's colours and fonts.
//! - [`inherit`]: what a slide's placeholders take from its layout and
//!   master, as DeckCraft's craft-pptx has it.
//! - [`shapes`]: the shapes, placed.
//! - [`text`]: their text.
//!
//! Charts and SmartArt are not drawn: an outline stands where they are.

mod inherit;
mod shapes;
mod text;
mod theme;

use super::media::Media;
use super::package::{Package, Rels, first};
use super::xml::{attr, child, find, parse};
use crate::markdown::{NOTE_LABEL, Picture};
use crate::render::Stopped;
use inherit::{Parents, background, decorations, hides_master};
use roxmltree::{Document, Node};
use shapes::{Map, Painter};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use theme::{Rgb, Theme};

/// EMU, Office's unit, in a point.
const EMU_PER_PT: f64 = 12_700.0;

/// A slide's size when the presentation gives none: 16:9.
const DEFAULT_SIZE: (f64, f64) = (12_192_000.0, 6_858_000.0);

/// `v` points, as typst writes it.
fn pt(v: f64) -> String {
    format!("{v:.2}pt")
}

/// A deck laid out as typst.
pub struct Laid {
    pub source: String,
    /// The pictures `source` shows, by the names it gives them.
    pub pictures: Vec<Picture>,
    /// The slides' speaker notes, each marked on its slide by its number.
    pub speaker: Vec<String>,
    /// What was left out, for the status line.
    pub notes: Vec<String>,
    /// Where the pictures were written, kept while they are typeset.
    _images: Option<tempfile::TempDir>,
}

/// The pictures shown, each written once and named once.
#[derive(Default)]
struct Pictures {
    by_path: HashMap<String, Option<String>>,
    list: Vec<Picture>,
    /// Pictures whose files cannot be decoded.
    broken: usize,
}

impl Pictures {
    /// The picture relationship `id` leads to, by the name typst reads it
    /// by, or `None` if typst cannot read it.
    fn add(
        &mut self,
        pkg: &mut Package,
        media: &mut Media,
        rels: &Rels,
        id: &str,
    ) -> Option<String> {
        let path = media.file(pkg, rels, id)?;
        if let Some(name) = self.by_path.get(&path) {
            return name.clone();
        }
        let ext = Path::new(&path)
            .extension()
            .map_or(String::new(), |e| e.to_string_lossy().into_owned());
        // A picture typst could not decode would fail the whole deck: it is
        // looked at first, and left out if it cannot be read.
        let readable = ext == "svg"
            || image::ImageReader::open(&path)
                .and_then(image::ImageReader::with_guessed_format)
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .is_some();
        let name = readable.then(|| format!("/office-{}.{ext}", self.list.len() + 1));
        match &name {
            Some(name) => self.list.push(Picture {
                name: name.clone(),
                path: PathBuf::from(&path),
                at: 0..0,
            }),
            None => self.broken += 1,
        }
        self.by_path.insert(path, name.clone());
        name
    }
}

/// The parts slides share, layouts, masters and themes, and their
/// relationships, each read from the package once.
#[derive(Default)]
struct Shared {
    texts: HashMap<String, String>,
    rels: HashMap<String, Rels>,
}

impl Shared {
    fn load(&mut self, pkg: &mut Package, part: &str) {
        if !self.texts.contains_key(part)
            && let Some(text) = pkg.text(part)
        {
            self.texts.insert(part.to_string(), text);
            self.rels.insert(part.to_string(), pkg.rels(part));
        }
    }

    fn rels(&self, part: Option<&str>) -> Rels {
        part.and_then(|p| self.rels.get(p))
            .cloned()
            .unwrap_or_default()
    }
}

/// A slide's part, and its layout's, master's and theme's.
struct Parts {
    slide: String,
    layout: Option<String>,
    master: Option<String>,
    theme: Option<String>,
}

/// The deck at `path`, laid out, until `stop` is set.
pub fn lay_out(path: &Path, stop: &AtomicBool) -> anyhow::Result<Laid> {
    let mut pkg = Package::read(path)?;
    let (main, text) = super::pptx::presentation(&mut pkg)?;
    let doc = parse(&text).map_err(|e| anyhow::anyhow!("its {main} cannot be read: {e}"))?;
    let root = doc.root_element();
    let (w, h) = child(root, "p:sldSz")
        .and_then(|s| {
            let cx = attr(s, "cx")?.parse::<f64>().ok()?;
            let cy = attr(s, "cy")?.parse::<f64>().ok()?;
            (cx > 0.0 && cy > 0.0).then_some((cx, cy))
        })
        .unwrap_or(DEFAULT_SIZE);

    // Each slide's layout, master and theme, read once for all.
    let mut shared = Shared::default();
    let mut slides = Vec::new();
    for slide in super::pptx::slide_parts(&mut pkg, &main, root) {
        let rels = pkg.rels(&slide);
        let layout = first(&rels, "slideLayout");
        if let Some(l) = &layout {
            shared.load(&mut pkg, l);
        }
        let master = first(&shared.rels(layout.as_deref()), "slideMaster");
        if let Some(m) = &master {
            shared.load(&mut pkg, m);
        }
        let theme = first(&shared.rels(master.as_deref()), "theme");
        if let Some(t) = &theme {
            shared.load(&mut pkg, t);
        }
        slides.push(Parts {
            slide,
            layout,
            master,
            theme,
        });
    }
    let docs: HashMap<&str, Document> = shared
        .texts
        .iter()
        .filter_map(|(part, text)| Some((part.as_str(), parse(text).ok()?)))
        .collect();
    let root_of = |part: &Option<String>| {
        part.as_deref()
            .and_then(|p| docs.get(p))
            .map(Document::root_element)
    };

    let none = Rels::new();
    let rels_of = |part: &Option<String>| {
        part.as_deref()
            .and_then(|p| shared.rels.get(p))
            .unwrap_or(&none)
    };
    let mut deck = Deck {
        pkg: &mut pkg,
        media: Media::new(super::Pictures::Written),
        pictures: Pictures::default(),
        decorations: HashMap::new(),
        charts: 0,
        diagrams: 0,
    };
    let default = child(root, "p:defaultTextStyle");
    let mut speaker = Vec::new();
    let mut source = format!(
        "#set page(width: {}, height: {}, margin: 0pt)\n#set text(top-edge: \"ascender\", bottom-edge: \"descender\", hyphenate: false)\n",
        pt(w / EMU_PER_PT),
        pt(h / EMU_PER_PT)
    );
    for parts in &slides {
        if stop.load(Ordering::Relaxed) {
            return Err(Stopped.into());
        }
        let slide = Slide {
            default,
            layout: root_of(&parts.layout),
            master: root_of(&parts.master),
            theme: root_of(&parts.theme),
            layout_part: parts.layout.as_deref().unwrap_or(""),
            master_part: parts.master.as_deref().unwrap_or(""),
            layout_rels: rels_of(&parts.layout),
            master_rels: rels_of(&parts.master),
        };
        let Some((page, notes)) = deck.page(&slide, &parts.slide) else {
            continue;
        };
        source.push_str(&page);
        if !notes.is_empty() {
            // The mark, on the slide's page, says whose notes these are.
            let n = speaker.len();
            speaker.push(notes.join("\n\n"));
            let at = source.rfind(']').unwrap_or(source.len());
            source.insert_str(at, &format!("#metadata({n})<{NOTE_LABEL}>"));
        }
    }
    let mut notes = deck.media.notes();
    for (n, what) in [
        (deck.pictures.broken, "picture that cannot be read left out"),
        (deck.charts, "chart not drawn"),
        (deck.diagrams, "diagram not drawn"),
    ] {
        if n > 0 {
            let (noun, rest) = what.split_once(' ').unwrap_or((what, ""));
            notes.push(format!("{} {rest}", super::count(n, noun)));
        }
    }
    Ok(Laid {
        source,
        pictures: deck.pictures.list,
        speaker,
        notes,
        _images: deck.media.into_dir(),
    })
}

/// What lasts from slide to slide: the package, the pictures written and
/// named, the master's and layouts' shapes drawn, and what was not drawn.
struct Deck<'p> {
    pkg: &'p mut Package,
    media: Media,
    pictures: Pictures,
    /// A master's or layout's own shapes, drawn once for the slides of a
    /// layout, by the part and the layout.
    decorations: HashMap<String, String>,
    charts: usize,
    diagrams: usize,
}

/// A slide's layout, master and theme, read already, and the
/// presentation's default text style.
struct Slide<'c, 'a, 'i> {
    default: Option<Node<'a, 'i>>,
    layout: Option<Node<'a, 'i>>,
    master: Option<Node<'a, 'i>>,
    theme: Option<Node<'a, 'i>>,
    layout_part: &'c str,
    master_part: &'c str,
    layout_rels: &'c Rels,
    master_rels: &'c Rels,
}

/// A part's override of its master's colour map.
fn color_override<'a, 'i>(root: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    child(child(root, "p:clrMapOvr")?, "a:overrideClrMapping")
}

impl Deck<'_> {
    /// The slide in `part` as a page of typst, and its speaker notes; `None`
    /// for a hidden slide.
    fn page(&mut self, s: &Slide, part: &str) -> Option<(String, Vec<String>)> {
        let text = self.pkg.text(part)?;
        let doc = parse(&text).ok()?;
        let root = doc.root_element();
        if attr(root, "show") == Some("0") {
            return None;
        }
        let rels = self.pkg.rels(part);
        let (layout, master) = (s.layout, s.master);
        let own_colors = color_override(root);
        let theme = s.theme.map_or_else(Theme::default, Theme::read).mapped(&[
            master.and_then(|m| child(m, "p:clrMap")),
            layout.and_then(color_override),
            own_colors,
        ]);
        let parents = Parents::new(layout, master, s.default);
        let roots: Vec<_> = [Some(root), layout, master].into_iter().flatten().collect();
        let fill = background(&roots, &theme).unwrap_or(Rgb(255, 255, 255));
        let mut typst = format!("#page(fill: {})[\n", fill.typst());
        let hidden_by_slide = hides_master(root);
        let show_master = !hidden_by_slide && !layout.is_some_and(hides_master);
        let mut painter = Painter {
            pkg: self.pkg,
            media: &mut self.media,
            pictures: &mut self.pictures,
            theme: &theme,
            parents: &parents,
            rels: &rels,
            slide: false,
            out: String::new(),
            charts: 0,
            diagrams: 0,
        };
        // The master's and layout's shapes are the same on every slide of
        // a layout, unless the slide maps its colours its own way.
        let behind = [
            (
                show_master.then_some(master).flatten(),
                s.master_part,
                s.master_rels,
            ),
            (
                (!hidden_by_slide).then_some(layout).flatten(),
                s.layout_part,
                s.layout_rels,
            ),
        ];
        for (root, part, rels) in behind {
            let Some(root) = root else { continue };
            let key = format!("{part}|{}", s.layout_part);
            let cached = own_colors
                .is_none()
                .then(|| self.decorations.get(&key))
                .flatten();
            if let Some(drawn) = cached {
                typst.push_str(drawn);
                continue;
            }
            painter.rels = rels;
            painter.out.clear();
            for n in decorations(root) {
                painter.shape(n, Map::default());
            }
            typst.push_str(&painter.out);
            if own_colors.is_none() {
                self.decorations.insert(key, painter.out.clone());
            }
        }
        if let Some(tree) = find(root, "p:spTree") {
            painter.rels = &rels;
            painter.slide = true;
            painter.out.clear();
            for n in tree.children().filter(Node::is_element) {
                painter.shape(n, Map::default());
            }
            typst.push_str(&painter.out);
        }
        self.charts += painter.charts;
        self.diagrams += painter.diagrams;
        typst.push_str("]\n");
        let notes = first(&rels, "notesSlide")
            .map(|n| super::pptx::speaker_notes(self.pkg, &n))
            .unwrap_or_default();
        Some((typst, notes))
    }
}

#[cfg(test)]
#[path = "../tests/office/deck.rs"]
mod tests;
