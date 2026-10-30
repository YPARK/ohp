//! A PDF signed, as a Fill & Sign tool signs it: a saved picture of a
//! signature, text, as a name or the date, and ✓ ✕ ● marks, each put where
//! it was clicked on a page as it is shown, upright however the page is
//! turned.
//!
//! They are drawn into the pages themselves, as an image, text in
//! Helvetica, which every reader has, and lines, so Acrobat, a browser and
//! a printer show them alike, and a form flattened keeps them. The PDF
//! itself is never changed: they are appended to a copy of it as an
//! incremental update, its own bytes kept as they were, so a digital
//! signature it carries stays valid.

use anyhow::{Context, bail};
use image::{DynamicImage, GenericImageView, RgbaImage};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, dictionary};
use std::path::{Path, PathBuf};

/// How wide a signature is put, in points, as Acrobat puts one.
pub const SIGNATURE_WIDTH: f32 = 150.;
/// How large text is set, in points.
pub const TEXT_SIZE: f32 = 11.;
/// How large a ✓, ✕ or ● is set, in points.
pub const MARK_SIZE: f32 = 14.;
/// The most pixels the longer side of a signature is kept at.
const MAX_SIDE: u32 = 1200;
/// What is added to a file's name for its signed copy, unless asked.
pub const SIGNED: &str = "signed";

/// What is put on a page.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    Signature,
    Text(String),
    Check,
    Cross,
    Dot,
}

/// Something put on a page, where, and how large.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub page: usize,
    /// Where it was put, in points from the top left of the page as it is
    /// shown: a signature's or text's left edge, a mark's middle, each
    /// halfway up it.
    pub at: (f32, f32),
    pub what: What,
    /// How much larger than its own size it is drawn.
    pub scale: f32,
}

impl Mark {
    pub fn new(page: usize, at: (f32, f32), what: What) -> Self {
        Mark {
            page,
            at,
            what,
            scale: 1.,
        }
    }
}

/// A signature as a picture: each pixel's colour and how opaque it is, the
/// paper it was on made clear and cut away.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub width: u32,
    pub height: u32,
    rgb: Vec<u8>,
    alpha: Vec<u8>,
}

impl Signature {
    /// The signature in the image at `path`: a PNG with its own clear
    /// background, or a photo or scan of one on paper.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let image = image::ImageReader::open(path)
            .with_context(|| format!("cannot open {}", path.display()))?
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("cannot read {} as an image", path.display()))?;
        Signature::of(&image)
    }

    /// The signature in `image`. Where it has no clear pixels of its own,
    /// its paper is made clear: each pixel is as opaque as it is darker
    /// than the paper, the brightest part of the image.
    pub fn of(image: &DynamicImage) -> anyhow::Result<Self> {
        let mut rgba = image.to_rgba8();
        let clear = rgba.pixels().any(|p| p[3] < 250);
        if !clear {
            let luma = |p: &image::Rgba<u8>| {
                0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
            };
            let mut lumas: Vec<f32> = rgba.pixels().map(luma).collect();
            lumas.sort_by(f32::total_cmp);
            let paper = lumas[lumas.len() * 95 / 100].max(1.);
            for p in rgba.pixels_mut() {
                let dark = (paper - luma(p)) / paper;
                // Faint shadows and grain clear; ink from a sixth darker on.
                let alpha = ((dark - 0.08) / (0.25 - 0.08)).clamp(0., 1.);
                p[3] = (alpha * 255.).round() as u8;
            }
        }
        let rgba = crop(&rgba).context("no signature in the image: it is all paper")?;
        let rgba = smaller(rgba);
        let (width, height) = rgba.dimensions();
        let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
        let mut alpha = Vec::with_capacity(rgba.len() / 4);
        for p in rgba.pixels() {
            rgb.extend_from_slice(&p.0[..3]);
            alpha.push(p[3]);
        }
        Ok(Signature {
            width,
            height,
            rgb,
            alpha,
        })
    }

    /// The signature as a PNG, its paper clear.
    pub fn png(&self) -> anyhow::Result<Vec<u8>> {
        let mut rgba = RgbaImage::new(self.width, self.height);
        for (i, p) in rgba.pixels_mut().enumerate() {
            let rgb = &self.rgb[i * 3..i * 3 + 3];
            *p = image::Rgba([rgb[0], rgb[1], rgb[2], self.alpha[i]]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(rgba).write_to(&mut out, image::ImageFormat::Png)?;
        Ok(out.into_inner())
    }

    /// How tall it is put, `width` points wide.
    pub fn height_at(&self, width: f32) -> f32 {
        width * self.height as f32 / self.width as f32
    }
}

/// `image` cut to what is not clear in it, with a little room around.
fn crop(image: &RgbaImage) -> Option<RgbaImage> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in image.enumerate_pixels() {
        if p[3] > 24 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x0 > x1 {
        return None;
    }
    let room = ((x1 - x0).max(y1 - y0) / 50).max(1);
    let (x0, y0) = (x0.saturating_sub(room), y0.saturating_sub(room));
    let x1 = (x1 + room).min(image.width() - 1);
    let y1 = (y1 + room).min(image.height() - 1);
    Some(image.view(x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image())
}

/// `image` no larger than `MAX_SIDE` pixels a side.
fn smaller(image: RgbaImage) -> RgbaImage {
    let (w, h) = image.dimensions();
    let side = w.max(h);
    if side <= MAX_SIDE {
        return image;
    }
    let k = MAX_SIDE as f32 / side as f32;
    let (w, h) = (
        ((w as f32 * k).round() as u32).max(1),
        ((h as f32 * k).round() as u32).max(1),
    );
    image::imageops::resize(&image, w, h, image::imageops::FilterType::Triangle)
}

/// The copy of `path` its marks are put in, beside it: its name with
/// `-signed`, or what is asked for in its place, added.
pub fn copy_of(path: &Path, added: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "document".into(), |s| s.to_string_lossy().into_owned());
    let added = added.trim().trim_start_matches(['-', '_', ' ']);
    let added = if added.is_empty() { SIGNED } else { added };
    path.with_file_name(format!("{stem}-{added}.pdf"))
}

/// Where ohp keeps what it is given to sign with: `$XDG_CONFIG_HOME/ohp`,
/// else `~/.config/ohp`.
fn config() -> Option<PathBuf> {
    // Tests keep nothing of theirs among the user's own.
    if cfg!(test) {
        return None;
    }
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))?;
    Some(dir.join("ohp"))
}

/// The signature kept to sign with: `$OHP_SIGNATURE`, else the one last
/// picked, if there is one.
pub fn kept() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OHP_SIGNATURE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    Some(config()?.join("signature.png")).filter(|p| p.is_file())
}

/// Keep `signature` to sign with from now on, its paper clear; where.
pub fn keep(signature: &Signature) -> anyhow::Result<PathBuf> {
    let dir = config().context("no home to keep the signature in")?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("signature.png");
    write(&path, &signature.png()?)?;
    Ok(path)
}

/// The texts typed before, the latest first: a name, an address.
pub fn typed() -> Vec<String> {
    let Some(path) = config().map(|dir| dir.join("typed")) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Keep `text` first among the texts typed, the ten latest kept.
pub fn remember(text: &str) {
    let text = text.trim();
    let Some(dir) = config() else {
        return;
    };
    if text.is_empty() || text.contains('\n') {
        return;
    }
    let mut all = typed();
    all.retain(|t| t != text);
    all.insert(0, text.to_string());
    all.truncate(10);
    let _ = std::fs::create_dir_all(&dir)
        .and_then(|_| write(&dir.join("typed"), all.join("\n").as_bytes()));
}

/// The initials of `name`: the first letter of each of its words.
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Write `bytes` to `path` whole: a reader never finds it half written.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    let mut file = tempfile::NamedTempFile::new_in(dir.unwrap_or(Path::new(".")))?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Today, as `2026-10-30`, here.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = civil((secs + offset(secs)).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// How far ahead of UTC the time here is, in seconds, at `secs`.
#[cfg(unix)]
fn offset(secs: i64) -> i64 {
    // SAFETY: localtime_r writes only into `tm`, which it is given.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let t = secs as libc::time_t;
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff
    }
}

#[cfg(not(unix))]
fn offset(_: i64) -> i64 {
    0
}

/// The year, month and day `days` after 1 January 1970.
fn civil(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant's days_from_civil, backwards.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Whether Helvetica, as PDFs set text without a font of their own, has
/// every letter of `text`.
pub fn settable(text: &str) -> bool {
    text.chars().all(|c| win_ansi(c).is_some())
}

/// `c` as the WinAnsi encoding has it.
fn win_ansi(c: char) -> Option<u8> {
    let code = u32::from(c);
    match c {
        ' '..='~' => Some(code as u8),
        '\u{a0}'..='\u{ff}' => Some(code as u8),
        '€' => Some(0x80),
        '‚' => Some(0x82),
        '„' => Some(0x84),
        '…' => Some(0x85),
        '‘' => Some(0x91),
        '’' => Some(0x92),
        '“' => Some(0x93),
        '”' => Some(0x94),
        '•' => Some(0x95),
        '–' => Some(0x96),
        '—' => Some(0x97),
        _ => None,
    }
}

/// `text` as a PDF string in WinAnsi, a letter Helvetica lacks as `?`.
fn pdf_string(text: &str) -> Vec<u8> {
    let mut out = vec![b'('];
    for c in text.chars() {
        let b = win_ansi(c).unwrap_or(b'?');
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

/// How a page as it is shown, in points from its top left, maps to the
/// PDF's own space: `a b c d e f`, as `cm` takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View([f32; 6]);

impl View {
    /// The view of a page whose crop box is `[x0, y0, x1, y1]`, turned
    /// `rotate` degrees clockwise as it is shown.
    fn new(cropped: [f32; 4], rotate: i64) -> Self {
        let [x0, y0, x1, y1] = cropped;
        View(match rotate.rem_euclid(360) {
            90 => [0., 1., 1., 0., x0, y0],
            180 => [-1., 0., 0., 1., x1, y0],
            270 => [0., -1., -1., 0., x1, y1],
            _ => [1., 0., 0., -1., x0, y1],
        })
    }

    /// The point shown at `(u, v)`.
    fn at(self, (u, v): (f32, f32)) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * u + c * v + e, b * u + d * v + f)
    }

    /// The matrix that draws a `w` by `h` box upright, as shown, its bottom
    /// left shown at `(u, v)`.
    fn upright(self, (u, v): (f32, f32), w: f32, h: f32) -> String {
        let [a, b, c, d, ..] = self.0;
        let (e, f) = self.at((u, v));
        numbers(&[w * a, w * b, -h * c, -h * d, e, f])
    }
}

/// `numbers` as a content stream writes them.
fn numbers(numbers: &[f32]) -> String {
    let each = numbers.iter().map(|n| {
        let s = format!("{:.3}", if n.abs() < 0.0005 { 0. } else { *n });
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    });
    each.collect::<Vec<_>>().join(" ")
}

/// A page's attribute, its own or one it inherits from the pages above it.
fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut dict = doc.get_dictionary(page).ok()?;
    for _ in 0..64 {
        if let Ok(value) = dict.get(key) {
            return doc.dereference(value).ok().map(|(_, v)| v);
        }
        let parent = dict.get(b"Parent").and_then(Object::as_reference).ok()?;
        dict = doc.get_dictionary(parent).ok()?;
    }
    None
}

/// The box of `page` that is shown: its crop box within its media box.
fn shown_box(doc: &Document, page: ObjectId) -> [f32; 4] {
    let read = |key: &[u8]| -> Option<[f32; 4]> {
        let array = inherited(doc, page, key)?.as_array().ok()?;
        let n: Vec<f32> = array
            .iter()
            .filter_map(|o| doc.dereference(o).ok()?.1.as_float().ok())
            .collect();
        let [a, b, c, d] = n[..] else {
            return None;
        };
        Some([a.min(c), b.min(d), a.max(c), b.max(d)])
    };
    let media = read(b"MediaBox").unwrap_or([0., 0., 612., 792.]);
    match read(b"CropBox") {
        Some(c) => [
            c[0].max(media[0]),
            c[1].max(media[1]),
            c[2].min(media[2]),
            c[3].min(media[3]),
        ],
        None => media,
    }
}

/// `pdf` with `marks` drawn on its pages, `signature` as each signature,
/// appended to it as an incremental update.
pub fn stamp(pdf: &[u8], marks: &[Mark], signature: Option<&Signature>) -> anyhow::Result<Vec<u8>> {
    let mut update = IncrementalDocument::load_from(pdf).context("cannot read the PDF")?;
    if update.get_prev_documents().is_encrypted() {
        bail!("the PDF is locked with a password");
    }
    let pages = update.get_prev_documents().get_pages();
    let doc = &mut update.new_document;
    let signs = marks.iter().any(|m| m.what == What::Signature);
    let image = match signature {
        Some(s) if signs => Some(add_image(doc, s)?),
        None if signs => bail!("no signature to put"),
        _ => None,
    };
    let texts = marks.iter().any(|m| matches!(m.what, What::Text(_)));
    let helvetica = texts.then(|| {
        doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        })
    });
    for (&n, &page_id) in &pages {
        let on: Vec<&Mark> = marks.iter().filter(|m| m.page + 1 == n as usize).collect();
        if on.is_empty() {
            continue;
        }
        let prev = update.get_prev_documents();
        let rotate = inherited(prev, page_id, b"Rotate")
            .and_then(|r| r.as_i64().ok())
            .unwrap_or(0);
        let view = View::new(shown_box(prev, page_id), rotate);
        let mut ops: Vec<u8> = b"Q\n".to_vec();
        for mark in on {
            draw(&mut ops, view, mark, signature);
        }
        let mut resources = inherited(prev, page_id, b"Resources")
            .and_then(|r| r.as_dict().ok())
            .cloned()
            .unwrap_or_default();
        let mut add = |kind: &[u8], name: &str, id: Option<ObjectId>| {
            let Some(id) = id else {
                return;
            };
            let mut named = resources
                .get(kind)
                .ok()
                .and_then(|o| prev.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok())
                .cloned()
                .unwrap_or_default();
            named.set(name, Object::Reference(id));
            resources.set(kind, Object::Dictionary(named));
        };
        add(b"XObject", "OhpSignature", image);
        add(b"Font", "OhpHelvetica", helvetica);
        let mut contents: Vec<Object> = match prev.get_dictionary(page_id)?.get(b"Contents") {
            Ok(Object::Reference(id)) => match prev.get_object(*id) {
                Ok(Object::Array(parts)) => parts.clone(),
                _ => vec![Object::Reference(*id)],
            },
            Ok(Object::Array(parts)) => parts.clone(),
            _ => Vec::new(),
        };
        let doc = &mut update.new_document;
        let before = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
        let mut after = Stream::new(Dictionary::new(), ops);
        let _ = after.compress();
        let after = doc.add_object(after);
        contents.insert(0, Object::Reference(before));
        contents.push(Object::Reference(after));
        update.opt_clone_object_to_new_document(page_id)?;
        let page = update.new_document.get_object_mut(page_id)?.as_dict_mut()?;
        page.set("Resources", Object::Dictionary(resources));
        page.set("Contents", Object::Array(contents));
    }
    let mut out = Vec::new();
    update.save_to(&mut out)?;
    Ok(out)
}

/// Draw `mark` as `view` shows its page.
fn draw(ops: &mut Vec<u8>, view: View, mark: &Mark, signature: Option<&Signature>) {
    let (u, v) = mark.at;
    match &mark.what {
        What::Signature => {
            let Some(s) = signature else {
                return;
            };
            let w = SIGNATURE_WIDTH * mark.scale;
            let h = s.height_at(w);
            let cm = view.upright((u, v + h / 2.), w, h);
            ops.extend(format!("q {cm} cm /OhpSignature Do Q\n").bytes());
        }
        What::Text(t) => {
            let size = TEXT_SIZE * mark.scale;
            // Halfway up a capital is about a third of the size above its foot.
            let tm = view.upright((u, v + size * 0.35), size, size);
            ops.extend(format!("q 0 g BT /OhpHelvetica 1 Tf {tm} Tm ").bytes());
            ops.extend_from_slice(&pdf_string(t));
            ops.extend(b" Tj ET Q\n");
        }
        tick => {
            // Drawn, not set in ZapfDingbats, which not every reader has:
            // in a box a unit square, upright and centred where it was put.
            let size = MARK_SIZE * mark.scale;
            let cm = view.upright((u - size / 2., v + size / 2.), size, size);
            let path: &[u8] = match tick {
                What::Check => b"0.12 w 1 J 1 j 0.12 0.5 m 0.4 0.18 l 0.9 0.88 l S",
                What::Cross => b"0.12 w 1 J 0.15 0.15 m 0.85 0.85 l 0.15 0.85 m 0.85 0.15 l S",
                _ => {
                    b"0.5 0.82 m 0.68 0.82 0.82 0.68 0.82 0.5 c 0.82 0.32 0.68 0.18 0.5 0.18 c \
                       0.32 0.18 0.18 0.32 0.18 0.5 c 0.18 0.68 0.32 0.82 0.5 0.82 c f"
                }
            };
            ops.extend(format!("q {cm} cm 0 g 0 G ").bytes());
            ops.extend_from_slice(path);
            ops.extend(b" Q\n");
        }
    }
}

/// `signature` as an image the pages can show, its clear parts a soft mask.
fn add_image(doc: &mut Document, signature: &Signature) -> anyhow::Result<ObjectId> {
    let image = |space: &str, bytes: Vec<u8>| -> anyhow::Result<Stream> {
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(signature.width),
                "Height" => i64::from(signature.height),
                "ColorSpace" => space,
                "BitsPerComponent" => 8,
            },
            bytes,
        );
        stream.compress()?;
        Ok(stream)
    };
    let mut colour = image("DeviceRGB", signature.rgb.clone())?;
    if signature.alpha.iter().any(|&a| a < 255) {
        let mask = doc.add_object(image("DeviceGray", signature.alpha.clone())?);
        colour.dict.set("SMask", Object::Reference(mask));
    }
    Ok(doc.add_object(colour))
}

#[cfg(test)]
#[path = "tests/sign.rs"]
mod tests;
