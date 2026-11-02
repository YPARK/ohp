//! Beamer's notes on a second screen: built with `\setbeameroption{show
//! notes on second screen=right}`, each page is a slide with its note
//! beside it, twice as wide. Presenting to a second window, the slide half
//! goes to the audience, and the note half to the presenter.
//!
//! Each page is cut by its crop box, which the renderer draws within: no
//! page is drawn anew, so the halves are as sharp as the whole.

use anyhow::{Context, anyhow};
use lopdf::{Dictionary, Document, Object, ObjectId};

/// Whether every page of a deck of these sizes is a slide with its note
/// beside it: twice as wide as a 4:3 to a 16:9 slide.
pub fn noted(sizes: &[(f32, f32)]) -> bool {
    !sizes.is_empty()
        && sizes
            .iter()
            .all(|&(w, h)| h > 0. && (2.4..=3.7).contains(&(w / h)))
}

/// A noted deck, cut.
pub struct Split {
    /// The slides alone, for the audience.
    pub slides: Vec<u8>,
    /// The slides, then each one's note, for the presenter.
    pub both: Vec<u8>,
}

/// Cut each page of `data` into its slide, on the left, and its note.
pub fn split(data: &[u8]) -> anyhow::Result<Split> {
    let mut doc = Document::load_mem(data).context("cannot read the PDF")?;
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let root = doc
        .catalog()?
        .get(b"Pages")?
        .as_reference()
        .map_err(|_| anyhow!("the PDF's pages are not where they should be"))?;
    let mut notes = Vec::with_capacity(pages.len());
    for &id in &pages {
        let media = inherited(&doc, id, b"MediaBox")
            .and_then(|o| rect(&doc, &o))
            .ok_or_else(|| anyhow!("a page has no size"))?;
        let [x0, y0, x1, y1] = inherited(&doc, id, b"CropBox")
            .and_then(|o| rect(&doc, &o))
            .unwrap_or(media);
        let mid = (x0 + x1) / 2.;
        // What a page takes from those above it, as its note will be put
        // straight under the root.
        let resources = inherited(&doc, id, b"Resources");
        let rotate = inherited(&doc, id, b"Rotate");
        let page = doc.get_dictionary_mut(id)?;
        page.set("MediaBox", boxed(media));
        page.set("CropBox", boxed([x0, y0, mid, y1]));
        if let Some(resources) = resources {
            page.set("Resources", resources);
        }
        if let Some(rotate) = rotate {
            page.set("Rotate", rotate);
        }
        let mut note: Dictionary = page.clone();
        note.set("CropBox", boxed([mid, y0, x1, y1]));
        note.set("Parent", Object::Reference(root));
        // Links and the like belong to the page they are on.
        note.remove(b"Annots");
        note.remove(b"StructParents");
        notes.push(note);
    }
    let slides = save(&mut doc.clone())?;
    let mut ids = Vec::with_capacity(notes.len());
    for note in notes {
        ids.push(doc.add_object(note));
    }
    let tree = doc.get_dictionary_mut(root)?;
    let count = tree.get(b"Count").and_then(Object::as_i64).unwrap_or(0);
    tree.set("Count", count + ids.len() as i64);
    let mut kids = tree
        .get(b"Kids")
        .and_then(Object::as_array)
        .cloned()
        .unwrap_or_default();
    kids.extend(ids.into_iter().map(Object::Reference));
    tree.set("Kids", kids);
    let both = save(&mut doc)?;
    Ok(Split { slides, both })
}

fn save(doc: &mut Document) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    doc.save_to(&mut out).context("cannot write the PDF")?;
    Ok(out)
}

/// `key` of the page `id`, or of the nearest node above it that has it.
fn inherited(doc: &Document, id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut dict = doc.get_dictionary(id).ok()?;
    for _ in 0..64 {
        if let Ok(value) = dict.get(key) {
            return Some(value.clone());
        }
        let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
        dict = doc.get_dictionary(parent).ok()?;
    }
    None
}

/// A rectangle, `[x0 y0 x1 y1]`, ordered.
fn rect(doc: &Document, o: &Object) -> Option<[f32; 4]> {
    let o = match o {
        Object::Reference(id) => doc.get_object(*id).ok()?,
        o => o,
    };
    let numbers: Vec<f32> = o
        .as_array()
        .ok()?
        .iter()
        .filter_map(|n| match n {
            Object::Integer(i) => Some(*i as f32),
            Object::Real(r) => Some(*r),
            _ => None,
        })
        .collect();
    let [a, b, c, d] = numbers[..] else {
        return None;
    };
    Some([a.min(c), b.min(d), a.max(c), b.max(d)])
}

fn boxed(r: [f32; 4]) -> Object {
    Object::Array(r.iter().map(|&n| Object::Real(n)).collect())
}

#[cfg(test)]
#[path = "tests/split.rs"]
mod tests;
