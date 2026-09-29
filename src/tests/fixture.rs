//! Small PDFs written for tests.

use std::path::{Path, PathBuf};

/// A beamer 16:9 frame, in points.
pub const FRAME: (f32, f32) = (453.54, 255.12);

/// A PDF of `pages` 16:9 pages, each with a filled square so it renders to
/// something other than white.
pub fn pdf(pages: usize) -> Vec<u8> {
    let (w, h) = FRAME;
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")),
    ];
    for i in 0..pages {
        let content = format!("0 0 1 rg {} 20 60 60 re f", 20 + 20 * i);
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>",
            4 + 2 * i
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
    out.extend(format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    out
}

/// Write a `pages`-page PDF to `dir/deck.pdf`.
pub fn write(dir: &Path, pages: usize) -> PathBuf {
    let path = dir.join("deck.pdf");
    std::fs::write(&path, pdf(pages)).expect("writing test PDF");
    path
}
