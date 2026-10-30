//! Pictures in the page: images, those embedded in it written beside it,
//! and diagrams, drawn already or as a mermaid flowchart's source.

use super::code::trim_code;
use super::math::ascii_starts;
use super::{Writer, clean_url, escape_to_string, skipped, words};
use base64::Engine;
use scraper::ElementRef;

impl Writer {
    pub(super) fn image(&mut self, el: ElementRef) {
        let alt = words(
            el.attr("alt")
                .or_else(|| el.attr("aria-label"))
                .unwrap_or(""),
        );
        let Some(src) = image_src(el) else {
            self.plain(&alt);
            return;
        };
        let dest = if src.starts_with("data:") {
            let Some(path) = self.embed(src) else {
                self.plain(&alt);
                return;
            };
            path
        } else if alt.is_empty() && remote(src) {
            // Never fetched, and nothing to say of it: a banner, a spacer, a
            // pixel that tells the sender it was seen.
            return;
        } else {
            let src = src.split(['?', '#']).next().unwrap_or(src);
            clean_url(src)
        };
        if dest.is_empty() {
            self.plain(&alt);
            return;
        }
        self.inline(&format!("![{}](<{dest}>)", escape_to_string(&alt)));
    }

    /// A diagram already drawn, or a mermaid flowchart drawn from its source.
    /// Anything else is left as code: its script never runs here.
    pub(super) fn diagram(&mut self, el: ElementRef) {
        let Some(picture) = diagram_picture(el) else {
            let owned = el.text().collect::<String>();
            let text = trim_code(&owned);
            if text.is_empty() {
                return;
            }
            if let Some(svg) = crate::mermaid::svg(text)
                && let Some(path) = self.embed_bytes(svg.as_bytes(), "svg")
            {
                self.block_break();
                self.inline(&format!("![](<{path}>)"));
                self.block_break();
                return;
            }
            self.code_block(el);
            return;
        };
        if picture.value().name() == "svg" {
            self.drawing(picture);
            return;
        }
        self.block_break();
        self.image(picture);
        self.block_break();
    }

    /// An SVG drawn in the page, written out so typst can read it.
    pub(super) fn drawing(&mut self, el: ElementRef) {
        if !drawn(el) {
            return;
        }
        let markup = svg_markup(el);
        let Some(path) = self.embed_bytes(markup.as_bytes(), "svg") else {
            return;
        };
        let alt = words(
            el.attr("aria-label")
                .or_else(|| el.attr("title"))
                .unwrap_or(""),
        );
        self.block_break();
        self.inline(&format!("![{}](<{path}>)", escape_to_string(&alt)));
        self.block_break();
    }

    /// Write an embedded image aside and return the path the markdown uses.
    fn embed(&mut self, src: &str) -> Option<String> {
        let (bytes, ext) = data_image(src)?;
        self.embed_bytes(&bytes, ext)
    }

    fn embed_bytes(&mut self, bytes: &[u8], ext: &str) -> Option<String> {
        if self.images.is_none() {
            self.images = tempfile::tempdir().ok();
        }
        let dir = self.images.as_ref()?;
        let path = dir.path().join(format!("image{}.{ext}", self.nimage));
        self.nimage += 1;
        std::fs::write(&path, bytes).ok()?;
        Some(path.to_string_lossy().into_owned())
    }
}

pub(super) fn shows_image(el: ElementRef) -> bool {
    match el.value().name() {
        "svg" => !skipped(el) && drawn(el),
        "img" | "object" | "embed" => image_src(el).is_some(),
        _ => false,
    }
}

/// Whether `src` is fetched from elsewhere, not a file beside the page.
fn remote(src: &str) -> bool {
    ["http:", "https:", "//", "cid:"]
        .iter()
        .any(|p| ascii_starts(src, p))
}

pub(super) fn image_src(el: ElementRef<'_>) -> Option<&str> {
    let name = el.value().name();
    let src = match name {
        "img" => el
            .attr("src")
            .or_else(|| el.attr("data-src"))
            .or_else(|| srcset_url(el)),
        "object" => el.attr("data").or_else(|| el.attr("src")),
        "embed" => el.attr("src").or_else(|| el.attr("data")),
        _ => None,
    }?;
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    if name == "img" || looks_like_image(src, el.attr("type").unwrap_or("")) {
        Some(src)
    } else {
        None
    }
}

fn srcset_url(el: ElementRef<'_>) -> Option<&str> {
    let url = el
        .attr("srcset")?
        .split(',')
        .next()?
        .split_whitespace()
        .next()?;
    (!url.is_empty()).then_some(url)
}

fn looks_like_image(src: &str, typ: &str) -> bool {
    if ascii_starts(src, "data:image") || ascii_starts(typ, "image/") {
        return true;
    }
    let name = src
        .split(['?', '#'])
        .next()
        .unwrap_or(src)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(src);
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
    )
}

pub(super) fn diagram_lang(el: ElementRef) -> Option<&'static str> {
    const NAMES: &[&str] = &["mermaid", "plantuml", "graphviz", "d2", "nomnoml", "pikchr"];
    el.value().classes().find_map(|class| {
        let bare = strip_prefix_ignore(class, "language-")
            .or_else(|| strip_prefix_ignore(class, "lang-"))
            .unwrap_or(class);
        NAMES
            .iter()
            .copied()
            .find(|name| bare.eq_ignore_ascii_case(name))
    })
}

fn strip_prefix_ignore<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    ascii_starts(s, prefix).then(|| &s[prefix.len()..])
}

/// The picture a diagram widget drew, rather than its source.
fn diagram_picture(el: ElementRef<'_>) -> Option<ElementRef<'_>> {
    el.descendent_elements().find(|child| {
        *child != el
            && match child.value().name() {
                "svg" => drawn(*child),
                "img" => image_src(*child).is_some(),
                _ => false,
            }
    })
}

fn drawn(el: ElementRef) -> bool {
    el.descendent_elements().any(|child| child != el)
}

fn svg_markup(el: ElementRef) -> String {
    let markup = el.html();
    // The HTML parser does not keep the SVG namespace, which typst wants.
    if markup.to_ascii_lowercase().contains("xmlns") {
        markup
    } else if let Some(rest) = markup.strip_prefix("<svg") {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\"{rest}")
    } else {
        markup
    }
}

fn data_image(src: &str) -> Option<(Vec<u8>, &'static str)> {
    let rest = src.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let ext = image_ext(meta);
    let bytes = if meta
        .split(';')
        .any(|p| p.trim().eq_ignore_ascii_case("base64"))
    {
        decode_base64(data)?
    } else {
        percent_encoding::percent_decode_str(data).collect()
    };
    (!bytes.is_empty()).then_some((bytes, ext))
}

fn image_ext(meta: &str) -> &'static str {
    let meta = meta.to_ascii_lowercase();
    if meta.contains("jpeg") || meta.contains("/jpg") {
        "jpg"
    } else if meta.contains("gif") {
        "gif"
    } else if meta.contains("webp") {
        "webp"
    } else if meta.contains("svg") {
        "svg"
    } else {
        "png"
    }
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    let mut clean: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    // The standard engine rejects a missing `=`, which embedded images omit.
    let pad = (4 - clean.len() % 4) % 4;
    clean.extend(std::iter::repeat_n('=', pad));
    base64::engine::general_purpose::STANDARD.decode(clean).ok()
}
