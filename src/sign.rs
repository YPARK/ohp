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
use image::ImageDecoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, RgbaImage};
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, dictionary};
use std::path::{Path, PathBuf};

/// The box a signature is fitted in, its shape kept, in points: as wide
/// as Acrobat puts one, and as tall as a line written by hand.
pub const SIGNATURE_WIDTH: f32 = 150.;
pub const SIGNATURE_HEIGHT: f32 = 32.;
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

impl Mark {
    /// Whether `point`, on `page`, is on it.
    fn covers(&self, page: usize, (x, y): (f32, f32), signature: Option<&Signature>) -> bool {
        let [x0, y0, x1, y1] = bounds(self, signature);
        self.page == page && (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
    }
}

/// The box `mark` covers on its page, as `[left, top, right, bottom]` in
/// points from its top left as shown; text's as wide as Helvetica's
/// letters are, about.
pub fn bounds(mark: &Mark, signature: Option<&Signature>) -> [f32; 4] {
    let (u, v) = mark.at;
    match &mark.what {
        What::Signature => {
            let w = signature.map_or(SIGNATURE_WIDTH * mark.scale, |s| s.width_at(mark.scale));
            let h = signature.map_or(w / 3., |s| s.height_at(w));
            [u, v - h / 2., u + w, v + h / 2.]
        }
        What::Text(t) => {
            let size = TEXT_SIZE * mark.scale;
            let w = 0.55 * size * t.chars().count() as f32;
            [u, v - size / 2., u + w, v + size / 2.]
        }
        _ => {
            let half = MARK_SIZE * mark.scale / 2.;
            [u - half, v - half, u + half, v + half]
        }
    }
}

/// The box, as `bounds` has boxes, of the form field to sign or type in
/// that is at `point` on `page`, as shown, or near it, if one is: a signature's or a
/// text's, not a checkbox's or a button's.
pub fn field_at(original: &Original, page: usize, (u, v): (f32, f32)) -> Option<[f32; 4]> {
    let doc = original.0.get_prev_documents();
    let id = *doc.get_pages().get(&(u32::try_from(page).ok()? + 1))?;
    let view = page_view(doc, id);
    let annots = doc.get_dictionary(id).ok()?.get(b"Annots").ok()?;
    let (_, annots) = doc.dereference(annots).ok()?;
    // The field the point is in, else the nearest it is near.
    let away = |p: f32, from: f32, to: f32| (from - p).max(p - to).max(0.);
    let fields = annots.as_array().ok()?.iter().filter_map(|a| {
        let widget = doc.dereference(a).ok()?.1.as_dict().ok()?;
        let fillable = matches!(field_type(doc, widget)?, b"Sig" | b"Tx");
        let rect = widget.get(b"Rect").ok()?.as_array().ok()?;
        let n: Vec<f32> = rect.iter().filter_map(|n| n.as_float().ok()).collect();
        let [x0, y0, x1, y1] = <[f32; 4]>::try_from(n).ok()?;
        let (a, b) = (view.shown((x0, y0)), view.shown((x1, y1)));
        let shown = [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)];
        let far = away(u, shown[0], shown[2]).hypot(away(v, shown[1], shown[3]));
        (fillable && far <= NEAR).then_some((far, shown))
    });
    fields
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, shown)| shown)
}

/// How near a field a click is taken to be in it, in points: as on the
/// line a field is drawn above.
const NEAR: f32 = 8.;

/// A field's type, `/FT`, as its widget has it or a field above it does.
fn field_type<'a>(doc: &'a Document, widget: &'a Dictionary) -> Option<&'a [u8]> {
    let mut at = widget;
    for _ in 0..16 {
        if let Ok(kind) = at.get(b"FT").and_then(Object::as_name) {
            return Some(kind);
        }
        let parent = at.get(b"Parent").ok()?;
        at = doc.dereference(parent).ok()?.1.as_dict().ok()?;
    }
    None
}

/// Where to put `signature`, and at what scale, to fit the field `field`,
/// a box as `bounds` has boxes: as large as it is, a little inside it, its
/// left edge at the field's, halfway up it.
pub fn fit(signature: &Signature, [x0, y0, x1, y1]: [f32; 4]) -> ((f32, f32), f32) {
    let (w, h) = (x1 - x0, y1 - y0);
    let one = signature.width_at(1.);
    let scale = (0.94 * w / one).min(0.84 * h / signature.height_at(one));
    (
        (x0 + 0.03 * w, (y0 + y1) / 2.),
        scale.clamp(SMALLEST, LARGEST),
    )
}

/// Which of `marks` is at `point` on `page`: the last put of those there.
pub fn under(
    marks: &[Mark],
    page: usize,
    point: (f32, f32),
    signature: Option<&Signature>,
) -> Option<usize> {
    marks.iter().rposition(|m| m.covers(page, point, signature))
}

impl What {
    /// Which size it is put at.
    pub fn kind(&self) -> Kind {
        match self {
            What::Signature => Kind::Signature,
            What::Text(_) => Kind::Text,
            What::Check | What::Cross | What::Dot => Kind::Mark,
        }
    }
}

/// What is put at a size of its own: the signature, text and the date,
/// or a ✓ ✕ ●.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Signature,
    Text,
    Mark,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Signature, Kind::Text, Kind::Mark];

    fn name(self) -> &'static str {
        match self {
            Kind::Signature => "signature",
            Kind::Text => "text",
            Kind::Mark => "mark",
        }
    }
}

/// Smallest and largest a scale is, of each kind's own size.
pub const SMALLEST: f32 = 0.2;
pub const LARGEST: f32 = 6.;

/// How large each kind is put, as a scale of its own size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes([f32; 3]);

impl Default for Sizes {
    fn default() -> Self {
        Sizes([1.; 3])
    }
}

impl Sizes {
    pub fn of(&self, kind: Kind) -> f32 {
        self.0[kind as usize]
    }

    /// Set `kind`'s scale, kept within bounds.
    pub fn set(&mut self, kind: Kind, scale: f32) {
        self.0[kind as usize] = scale.clamp(SMALLEST, LARGEST);
    }
}

/// The sizes kept in `dir`, one kind a line, as `signature 1.32`; each
/// not kept, or not read, its own size.
pub fn sizes(dir: Option<&Path>) -> Sizes {
    let mut sizes = Sizes::default();
    let Some(dir) = dir else {
        return sizes;
    };
    let kept = std::fs::read_to_string(dir.join(SIZES)).unwrap_or_default();
    for (name, scale) in kept.lines().filter_map(|l| l.split_once(' ')) {
        let kind = Kind::ALL.into_iter().find(|k| k.name() == name.trim());
        if let (Some(kind), Ok(scale)) = (kind, scale.trim().parse::<f32>())
            && scale.is_finite()
        {
            sizes.set(kind, scale);
        }
    }
    sizes
}

/// Keep `sizes` in `dir`, to put each kind at next time.
pub fn keep_sizes(dir: Option<&Path>, sizes: &Sizes) {
    let Some(dir) = dir else {
        return;
    };
    let lines: Vec<String> = Kind::ALL
        .into_iter()
        .map(|k| format!("{} {}", k.name(), sizes.of(k)))
        .collect();
    let _ = std::fs::create_dir_all(dir)
        .and_then(|_| write(&dir.join(SIZES), lines.join("\n").as_bytes()));
}

/// A signature as a picture, the paper it was on made clear and cut away,
/// and as the image a page shows it by, compressed once.
#[derive(Clone, Debug)]
pub struct Signature {
    image: RgbaImage,
    /// Its colours, and how clear each is, if any is.
    colour: Stream,
    mask: Option<Stream>,
}

impl Signature {
    /// The signature in the image at `path`: a PNG with its own clear
    /// background, or a photo or scan of one on paper, upright as a phone
    /// says it took it.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let mut decoder = image::ImageReader::open(path)
            .with_context(|| format!("cannot open {}", path.display()))?
            .with_guessed_format()?
            .into_decoder()
            .with_context(|| format!("cannot read {} as an image", path.display()))?;
        let orientation = decoder.orientation()?;
        let mut image = DynamicImage::from_decoder(decoder)?;
        image.apply_orientation(orientation);
        Signature::of(&image)
    }

    /// The signature in `image`. Where it has no clear pixels of its own,
    /// its paper is made clear: each pixel is as opaque as it is darker
    /// than the paper, the brightest part of the image.
    pub fn of(image: &DynamicImage) -> anyhow::Result<Self> {
        // A phone's photo, ten times the size kept, made smaller first.
        let most = 2 * MAX_SIDE;
        let mut rgba = if image.width().max(image.height()) > most {
            image.resize(most, most, FilterType::Triangle).to_rgba8()
        } else {
            image.to_rgba8()
        };
        let clear = rgba.pixels().any(|p| p[3] < 250);
        if !clear {
            let luma = |p: &image::Rgba<u8>| {
                0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])
            };
            let mut counts = [0usize; 256];
            for p in rgba.pixels() {
                counts[luma(p) as usize] += 1;
            }
            let mut below = rgba.len() / 4 * 95 / 100;
            let paper = counts.iter().position(|&n| {
                let past = n > below;
                below = below.saturating_sub(n);
                past
            });
            let paper = (paper.unwrap_or(255) as f32).max(1.);
            for p in rgba.pixels_mut() {
                let dark = (paper - luma(p)) / paper;
                // Faint shadows and grain clear; ink from a sixth darker on.
                let alpha = ((dark - 0.08) / (0.25 - 0.08)).clamp(0., 1.);
                p[3] = (alpha * 255.).round() as u8;
            }
        }
        // What is clear, one colour: the paper's grain, unseen, would be
        // kept, and compressed, in every copy signed.
        for p in rgba.pixels_mut().filter(|p| p[3] == 0) {
            *p = image::Rgba([0; 4]);
        }
        let mut rgba = crop(&rgba).context("no signature in the image: it is all paper")?;
        if rgba.width().max(rgba.height()) > MAX_SIDE {
            rgba = DynamicImage::ImageRgba8(rgba)
                .resize(MAX_SIDE, MAX_SIDE, FilterType::Triangle)
                .to_rgba8();
        }
        let image = |space: &str, bytes: Vec<u8>| -> anyhow::Result<Stream> {
            let mut stream = Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => i64::from(rgba.width()),
                    "Height" => i64::from(rgba.height()),
                    "ColorSpace" => space,
                    "BitsPerComponent" => 8,
                },
                bytes,
            );
            stream.compress()?;
            Ok(stream)
        };
        let rgb = rgba.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let colour = image("DeviceRGB", rgb)?;
        let mask = match rgba.pixels().any(|p| p[3] < 255) {
            true => Some(image("DeviceGray", rgba.pixels().map(|p| p[3]).collect())?),
            false => None,
        };
        Ok(Signature {
            image: rgba,
            colour,
            mask,
        })
    }

    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }

    /// The signature as a PNG, its paper clear.
    pub fn png(&self) -> anyhow::Result<Vec<u8>> {
        let mut out = std::io::Cursor::new(Vec::new());
        self.image.write_to(&mut out, image::ImageFormat::Png)?;
        Ok(out.into_inner())
    }

    /// How tall it is put, `width` points wide.
    pub fn height_at(&self, width: f32) -> f32 {
        width * self.height() as f32 / self.width() as f32
    }

    /// How wide it is put at `scale`: fitted in the box a signature is,
    /// `SIGNATURE_WIDTH` by `SIGNATURE_HEIGHT`, at 1.
    pub fn width_at(&self, scale: f32) -> f32 {
        let wide = self.width() as f32 / self.height() as f32;
        scale * SIGNATURE_WIDTH.min(SIGNATURE_HEIGHT * wide)
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
pub fn config() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))?;
    Some(dir.join("ohp"))
}

/// The signature kept in `dir`, as `keep` keeps it.
const SIGNATURE: &str = "signature.png";
/// The texts typed, kept in `dir`, one a line, the latest first.
const TYPED: &str = "typed";
/// How large each kind is put, kept in `dir`, as `sizes` reads them.
const SIZES: &str = "sizes";

/// The signature kept to sign with: `$OHP_SIGNATURE`, else the one last
/// picked, kept in `dir`, if there is one.
pub fn kept(dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OHP_SIGNATURE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(path));
    }
    Some(dir?.join(SIGNATURE)).filter(|p| p.is_file())
}

/// Keep `signature` in `dir` to sign with from now on, its paper clear;
/// where.
pub fn keep(dir: Option<&Path>, signature: &Signature) -> anyhow::Result<PathBuf> {
    let dir = dir.context("no home to keep the signature in")?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(SIGNATURE);
    write(&path, &signature.png()?)?;
    Ok(path)
}

/// The texts typed before, kept in `dir`, the latest first: a name, an
/// address.
pub fn typed(dir: Option<&Path>) -> Vec<String> {
    let Some(path) = dir.map(|dir| dir.join(TYPED)) else {
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

/// Keep `text` first among the texts typed, in `dir`, the ten latest kept.
pub fn remember(dir: Option<&Path>, text: &str) {
    let text = text.trim();
    let Some(dir) = dir else {
        return;
    };
    if text.is_empty() || text.contains('\n') {
        return;
    }
    let mut all = typed(Some(dir));
    all.retain(|t| t != text);
    all.insert(0, text.to_string());
    all.truncate(10);
    let _ = std::fs::create_dir_all(dir)
        .and_then(|_| write(&dir.join(TYPED), all.join("\n").as_bytes()));
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

/// Today, as `2026-10-31`, in the time zone here, as jiff reads it.
pub fn today() -> String {
    jiff::Zoned::now().date().to_string()
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

    /// Where the point `(x, y)` of the PDF is shown, as `at` has it turned.
    fn shown(self, (x, y): (f32, f32)) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        let (x, y) = (x - e, y - f);
        let det = a * d - b * c;
        ((d * x - c * y) / det, (a * y - b * x) / det)
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

/// How page `page` of `doc` is shown: its crop box, turned as it says.
fn page_view(doc: &Document, page: ObjectId) -> View {
    let rotate = inherited(doc, page, b"Rotate")
        .and_then(|r| r.as_i64().ok())
        .unwrap_or(0);
    View::new(shown_box(doc, page), rotate)
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

/// A PDF to sign, read once, as it was before anything was put on it.
#[derive(Clone, Debug)]
pub struct Original(IncrementalDocument);

impl Original {
    /// The PDF `pdf`, unless it needs a password to open.
    pub fn read(pdf: &[u8]) -> anyhow::Result<Self> {
        let pdf = IncrementalDocument::load_from(pdf).context("cannot read the PDF")?;
        if pdf.get_prev_documents().is_encrypted() {
            bail!("the PDF is locked with a password");
        }
        Ok(Original(pdf))
    }
}

/// `original` with `marks` drawn on its pages, `signature` as each
/// signature, appended to it as an incremental update.
pub fn stamp(
    original: &Original,
    marks: &[Mark],
    signature: Option<&Signature>,
) -> anyhow::Result<Vec<u8>> {
    let mut update = original.0.clone();
    let pages = update.get_prev_documents().get_pages();
    let doc = &mut update.new_document;
    let signs = marks.iter().any(|m| m.what == What::Signature);
    let image = match signature {
        Some(s) if signs => Some(add_image(doc, s)),
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
        let view = page_view(prev, page_id);
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
            let w = s.width_at(mark.scale);
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
fn add_image(doc: &mut Document, signature: &Signature) -> ObjectId {
    let mut colour = signature.colour.clone();
    if let Some(mask) = &signature.mask {
        let mask = doc.add_object(mask.clone());
        colour.dict.set("SMask", Object::Reference(mask));
    }
    doc.add_object(colour)
}

#[cfg(test)]
#[path = "tests/sign.rs"]
mod tests;
