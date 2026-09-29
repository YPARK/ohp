//! Small PDFs written for tests.

use crate::render::{Done, Renderer};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A beamer 16:9 frame, in points.
pub const FRAME: (f32, f32) = (453.54, 255.12);

/// A PDF of `pages` 16:9 pages, each with a filled square so it renders to
/// something other than white.
pub fn pdf(pages: usize) -> Vec<u8> {
    let contents: Vec<String> = (0..pages)
        .map(|i| format!("0 0 1 rg {} 20 60 60 re f", 20 + 20 * i))
        .collect();
    with(&contents)
}

/// A PDF of 16:9 pages drawn by `contents`, one content stream each, with
/// Helvetica as font `/F1`. PDF's y grows up from the bottom of the page.
pub fn with(contents: &[impl AsRef<str>]) -> Vec<u8> {
    let (w, h) = FRAME;
    let pages = contents.len();
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (i, content) in contents.iter().enumerate() {
        let content = content.as_ref();
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{object}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    let size = objects.len() + 1;
    out.extend(format!("xref\n0 {size}\n0000000000 65535 f \n").bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes(),
    );
    out
}

/// Write a `pages`-page PDF to `dir/deck.pdf`.
pub fn write(dir: &Path, pages: usize) -> PathBuf {
    write_pdf(dir, &pdf(pages))
}

pub fn write_pdf(dir: &Path, pdf: &[u8]) -> PathBuf {
    let path = dir.join("deck.pdf");
    std::fs::write(&path, pdf).expect("writing test PDF");
    path
}

/// A content stream writing `text` at (`x`, `y`) in `size`-point Helvetica.
pub fn text(x: f32, y: f32, size: f32, text: &str) -> String {
    format!("BT /F1 {size} Tf {x} {y} Td ({text}) Tj ET\n")
}

/// A one-page PDF that says "Hello".
pub fn hello() -> Vec<u8> {
    with(&[text(40., 200., 24., "Hello")])
}

/// The renderer's next finished job.
pub fn next(renderer: &Renderer) -> Done {
    renderer
        .done
        .recv_timeout(Duration::from_secs(10))
        .expect("a render")
}
