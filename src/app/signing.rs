//! Annotating and signing the PDF presented, as `A` begins it: the pages
//! turned and zoomed as ever, and each click puts what is picked in the
//! palette beside the page, by its number or a click on it, there: a
//! signature, text, the date, or a ✓ ✕ ● mark, in a copy beside the PDF,
//! shown in its place as each is put. A click on what is put picks it, to
//! drag, size with the wheel or `[` `]`, nudge with Shift and the arrows,
//! or take back with `u`. Each kind is put at the size it was last made,
//! kept for next time; a signature or text clicked in a form field is
//! fitted to it. `enter` or `esc` asks what to call the copy, its name
//! with `-signed` added, the `signed` picked to type over.
//!
//! The signature is the one kept from the last picked, or `$OHP_SIGNATURE`;
//! with none, or on `N`, an image of one is picked, a PNG or a photo of it
//! on paper, and kept for next time.
//!
//! The copy kept is written to the file `$OHP_SIGNED` names, if it does,
//! for what ran ohp, as letterbox, to find it.

use super::draw::{row_line, side_block};
use super::field::Field;
use super::{App, View};
use crate::browse::Browser;
use crate::render;
use crate::sign::{self, Kind, Mark, Signature, Sizes, What};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What a click puts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tool {
    Signature,
    Text,
    Date,
    Check,
    Cross,
    Dot,
}

impl Tool {
    /// In the palette's order, each picked by its number, from 1.
    pub(super) const ALL: [Tool; 6] = [
        Tool::Signature,
        Tool::Text,
        Tool::Date,
        Tool::Check,
        Tool::Cross,
        Tool::Dot,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Tool::Signature => "signature",
            Tool::Text => "text",
            Tool::Date => "date",
            Tool::Check => "✓",
            Tool::Cross => "✕",
            Tool::Dot => "●",
        }
    }

    /// Which size it puts at.
    pub(super) fn kind(self) -> Kind {
        match self {
            Tool::Signature => Kind::Signature,
            Tool::Text | Tool::Date => Kind::Text,
            Tool::Check | Tool::Cross | Tool::Dot => Kind::Mark,
        }
    }
}

/// How much larger, or smaller, `[` and `]` make what they size.
const STEP: f32 = 1.15;

/// How long the wheel rests before what it sized is written to the copy.
const WHEEL_REST: Duration = Duration::from_millis(250);

/// The palette's first tool's row, below the top of the column: past its
/// border, what a click does and a blank line.
const PALETTE_TOP: u16 = 3;

/// The keys the palette lists below the tools, and what each does.
const KEYS: [(&str, &str); 9] = [
    ("drag", "move what is put"),
    ("wheel", "size what it is over"),
    ("[ ]", "smaller, larger"),
    ("⇧ arrows", "nudge"),
    ("u", "take back"),
    ("N", "new signature image"),
    ("P", "print"),
    ("enter", "done: save the copy"),
    ("n p + -", "turn, zoom"),
];

/// A page, and a point on it, in points from its top left as it is shown.
pub(super) type Spot = (usize, (f32, f32));

/// Text being typed to put where it was clicked, and what Tab offers.
pub(super) struct Typing {
    spot: Spot,
    pub(super) field: Field,
    offers: Vec<String>,
    offered: usize,
}

/// What was put and is being dragged: where the drag began, on the page,
/// and where what is dragged was then; whether it has moved since.
pub(super) struct Grab {
    from: (f32, f32),
    was: (f32, f32),
    moved: bool,
}

/// The PDF being signed.
pub(super) struct Signing {
    /// The PDF as it was before anything was put on it, read once.
    pub(super) original: sign::Original,
    /// The copy what is put goes in, beside the PDF, as it is put.
    pub(super) copy: PathBuf,
    /// What has been put, the last put last.
    pub(super) marks: Vec<Mark>,
    pub(super) tool: Tool,
    pub(super) signature: Option<Signature>,
    /// The list open is picking the signature.
    pub(super) picking: bool,
    /// Where a click waits for the signature being picked to be put.
    pub(super) waiting: Option<Spot>,
    pub(super) typing: Option<Typing>,
    /// What to call the copy, being typed, once signing is done.
    pub(super) naming: Option<Field>,
    /// Where the signature and what is typed are kept, if anywhere.
    pub(super) kept_in: Option<PathBuf>,
    /// How large each kind is put, as last set, kept in `kept_in`.
    pub(super) sizes: Sizes,
    /// What was put, and is last now, being dragged.
    pub(super) grab: Option<Grab>,
    /// When the wheel last sized what was put, the copy not yet written.
    pub(super) wheeled: Option<Instant>,
}

impl App {
    /// Whether the PDF presented is being signed.
    pub(super) fn signing(&self) -> bool {
        self.signing.is_some()
    }

    /// Begin signing the PDF presented.
    pub(super) fn sign(&mut self) {
        // Begun again on the copy, what is put could no longer be moved or
        // taken back.
        if self.signing.is_some() {
            self.notice = Some("annotating: enter when done".into());
            return;
        }
        if !render::pdf(&self.deck.paths) || !self.links.is_empty() {
            self.notice = Some("only a PDF is annotated: A on one".into());
            return;
        }
        let original = match sign::Original::read(&self.deck.data) {
            Ok(original) => original,
            Err(e) => {
                self.notice = Some(format!("cannot sign it: {e:#}"));
                return;
            }
        };
        let kept_in = sign::config();
        self.view = View::Present;
        self.notice = Some(match sign::kept(kept_in.as_deref()) {
            Some(_) => "click the page where your signature goes".into(),
            None => "click the page where your signature goes, and pick an image of it".into(),
        });
        self.signing = Some(Box::new(Signing {
            original,
            copy: sign::copy_of(&self.deck.paths[0], sign::SIGNED),
            marks: Vec::new(),
            tool: Tool::Signature,
            signature: None,
            picking: false,
            waiting: None,
            typing: None,
            naming: None,
            sizes: sign::sizes(kept_in.as_deref()),
            grab: None,
            wheeled: None,
            kept_in,
        }));
    }

    fn pick_tool(&mut self, tool: Tool) {
        if let Some(s) = &mut self.signing {
            s.tool = tool;
            self.notice = Some(format!("click where the {} goes", tool.name()));
        }
    }

    /// A key while signing; whether it was signing's.
    pub(super) fn sign_key(&mut self, key: KeyEvent) -> bool {
        let Some(s) = &mut self.signing else {
            return false;
        };
        if let Some(name) = &mut s.naming {
            match key.code {
                KeyCode::Enter => self.finish(),
                KeyCode::Esc => s.naming = None,
                _ => name.key(key),
            }
            return true;
        }
        if let Some(typing) = &mut s.typing {
            match key.code {
                KeyCode::Enter => {
                    let typing = s.typing.take().expect("typing");
                    let text = typing.field.text.trim().to_string();
                    if !text.is_empty() {
                        if !sign::settable(&text) {
                            self.notice = Some(
                                "only Latin letters are set in Helvetica: the rest show as ?"
                                    .into(),
                            );
                        }
                        if text != sign::today() {
                            sign::remember(s.kept_in.as_deref(), &text);
                        }
                        let (page, at) = typing.spot;
                        self.put(Mark::new(page, at, What::Text(text)));
                    }
                }
                KeyCode::Esc => s.typing = None,
                KeyCode::Tab | KeyCode::BackTab if !typing.offers.is_empty() => {
                    let n = typing.offers.len();
                    // Begun empty, with nothing typed before: the first offer.
                    let shown = typing.field.text == typing.offers[typing.offered];
                    typing.offered = match key.code {
                        KeyCode::Tab if !shown => typing.offered,
                        KeyCode::Tab => (typing.offered + 1) % n,
                        _ => (typing.offered + n - 1) % n,
                    };
                    typing.field = Field::offered(&typing.offers[typing.offered]);
                }
                _ => typing.field.key(key),
            }
            return true;
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc | KeyCode::Enter if s.marks.is_empty() => {
                self.signing = None;
                self.notice = Some("nothing put".into());
            }
            KeyCode::Esc | KeyCode::Enter => {
                let name = render::name(&s.copy);
                let stem = name.trim_end_matches(".pdf").chars().count();
                let added = sign::SIGNED.chars().count();
                s.naming = Some(Field::picking(&name, stem - added, stem));
            }
            KeyCode::Char(c @ '1'..='6') => {
                let n = c as usize - '1' as usize;
                self.pick_tool(Tool::ALL[n]);
            }
            KeyCode::Char('N') => self.pick_signature(None),
            KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) => self.take_back(),
            KeyCode::Char('u') | KeyCode::Backspace | KeyCode::Delete => self.take_back(),
            KeyCode::Char('[') => self.resize(1. / STEP),
            KeyCode::Char(']') => self.resize(STEP),
            KeyCode::Left if shift => self.change_last(|m| m.at.0 -= 2.),
            KeyCode::Right if shift => self.change_last(|m| m.at.0 += 2.),
            KeyCode::Up if shift => self.change_last(|m| m.at.1 -= 2.),
            KeyCode::Down if shift => self.change_last(|m| m.at.1 += 2.),
            _ => return false,
        }
        true
    }

    /// Take back what was put, or picked, last.
    fn take_back(&mut self) {
        let Some(s) = &mut self.signing else {
            return;
        };
        match s.marks.pop() {
            Some(_) => self.write_copy(),
            None => self.notice = Some("nothing put to take back".into()),
        }
    }

    /// Change what was put last, and the copy with it.
    fn change_last(&mut self, change: impl FnOnce(&mut Mark)) {
        let Some(s) = &mut self.signing else {
            return;
        };
        match s.marks.last_mut() {
            Some(mark) => {
                change(mark);
                self.write_copy();
            }
            None => self.notice = Some("nothing put yet: click on the page".into()),
        }
    }

    /// Make what was put last `by` larger, and what is put next of its
    /// kind as large; with nothing put, what is picked to put. The size is
    /// kept for next time.
    fn resize(&mut self, by: f32) {
        if self.resize_only(by) {
            self.write_copy();
        }
    }

    /// Make what was put last `by` larger, as `resize` does, not writing
    /// the copy: whether there was one to. The size is kept for next time
    /// with the copy, or at once, with nothing put.
    fn resize_only(&mut self, by: f32) -> bool {
        let Some(s) = &mut self.signing else {
            return false;
        };
        let (kind, scale) = match s.marks.last() {
            Some(m) => (m.what.kind(), m.scale),
            None => (s.tool.kind(), s.sizes.of(s.tool.kind())),
        };
        s.sizes.set(kind, scale * by);
        let scale = s.sizes.of(kind);
        match s.marks.last_mut() {
            Some(mark) => {
                mark.scale = scale;
                true
            }
            None => {
                sign::keep_sizes(s.kept_in.as_deref(), &s.sizes);
                let name = s.tool.name();
                self.notice = Some(format!("the {name} is put at {}%", percent(scale)));
                false
            }
        }
    }

    /// The copy written, once the wheel has stopped sizing what was put
    /// for `WHEEL_REST`, or at once if `now`: a turn of the wheel sends
    /// many steps.
    pub(super) fn settle_wheel(&mut self, now: bool) {
        let due = self
            .signing
            .as_ref()
            .and_then(|s| s.wheeled)
            .is_some_and(|at| now || at.elapsed() >= WHEEL_REST);
        if due {
            self.write_copy();
        }
    }

    /// A press on the screen while signing: on the palette, the tool there
    /// picked; on what was put, it picked, last now, to drag, size or take
    /// back; elsewhere on the page, what is picked put there.
    pub(super) fn sign_click(&mut self, at: Position) {
        if let Some(tool) = self.palette_tool(at) {
            return self.pick_tool(tool);
        }
        let Some(spot @ (page, point)) = self.page_point(at) else {
            return;
        };
        let Some(s) = &mut self.signing else {
            return;
        };
        if s.typing.is_some() || s.naming.is_some() {
            return;
        }
        if let Some(i) = sign::under(&s.marks, page, point, s.signature.as_ref()) {
            let mark = s.marks.remove(i);
            s.grab = Some(Grab {
                from: point,
                was: mark.at,
                moved: false,
            });
            s.marks.push(mark);
            self.notice =
                Some("picked · drag to move · wheel or [ ] to size · u takes it back".into());
            return;
        }
        let what = match s.tool {
            Tool::Signature => {
                if s.signature.is_none() {
                    s.signature = sign::kept(s.kept_in.as_deref())
                        .and_then(|path| Signature::read(&path).ok());
                }
                if s.signature.is_none() {
                    return self.pick_signature(Some(spot));
                }
                What::Signature
            }
            Tool::Date => What::Text(sign::today()),
            Tool::Check => What::Check,
            Tool::Cross => What::Cross,
            Tool::Dot => What::Dot,
            Tool::Text => {
                let mut offers = sign::typed(s.kept_in.as_deref());
                let mut field = Field::default();
                if let Some(first) = offers.first() {
                    // What was typed last, picked: typed over, or kept on ⏎.
                    field = Field::offered(first);
                    let initials = sign::initials(first);
                    if initials.chars().count() > 1 && !offers.contains(&initials) {
                        offers.push(initials);
                    }
                }
                offers.push(sign::today());
                s.typing = Some(Typing {
                    spot,
                    field,
                    offers,
                    offered: 0,
                });
                self.notice = None;
                return;
            }
        };
        self.put(Mark::new(page, point, what));
    }

    /// Put `mark`, at the size of its kind.
    /// The pointer dragged to `at`, with what was put picked: it moved
    /// there, outlined until let go.
    pub(super) fn sign_drag(&mut self, at: Position) {
        let point = self.page_point(at);
        let Some(s) = &mut self.signing else {
            return;
        };
        let (Some(grab), Some(mark), Some((_, (x, y)))) = (&mut s.grab, s.marks.last_mut(), point)
        else {
            return;
        };
        mark.at = (grab.was.0 + x - grab.from.0, grab.was.1 + y - grab.from.1);
        grab.moved = true;
    }

    /// The pointer let go: what was dragged put where it is now.
    pub(super) fn sign_release(&mut self) {
        let moved = self
            .signing
            .as_mut()
            .and_then(|s| s.grab.take())
            .is_some_and(|g| g.moved);
        if moved {
            self.write_copy();
        }
    }

    /// The wheel at `at` while signing: over what was put, it picked and
    /// made larger, `up`, or smaller; whether it was over one.
    pub(super) fn sign_wheel(&mut self, at: Position, up: bool) -> bool {
        let Some((page, point)) = self.page_point(at) else {
            return false;
        };
        let Some(s) = &mut self.signing else {
            return false;
        };
        let Some(i) = sign::under(&s.marks, page, point, s.signature.as_ref()) else {
            return false;
        };
        let mark = s.marks.remove(i);
        s.marks.push(mark);
        if self.resize_only(if up { STEP } else { 1. / STEP })
            && let Some(s) = &mut self.signing
        {
            s.wheeled = Some(Instant::now());
        }
        true
    }

    /// Where what is being dragged is now, to outline: its page and its box
    /// on it, as `sign::bounds` has it.
    pub(super) fn dragged(&self) -> Option<(usize, [f32; 4])> {
        let s = self.signing.as_ref()?;
        let mark = s
            .marks
            .last()
            .filter(|_| s.grab.as_ref().is_some_and(|g| g.moved))?;
        Some((mark.page, sign::bounds(mark, s.signature.as_ref())))
    }

    /// The tool whose row of the palette is at `at`, if one is.
    fn palette_tool(&self, at: Position) -> Option<Tool> {
        let side = self.side.get();
        if !side.contains(at) {
            return None;
        }
        let row = at.y.checked_sub(side.y + PALETTE_TOP)?;
        Tool::ALL.get(usize::from(row)).copied()
    }

    /// The palette, in the column beside the page: the tools, the one
    /// picked lit, each at the size it is put, and the keys.
    pub(super) fn draw_palette(&self, f: &mut Frame, area: Rect) {
        let Some(s) = &self.signing else {
            return;
        };
        let dim = Style::new().fg(Color::DarkGray);
        let mut lines = vec![Line::styled("click the page to put", dim), Line::raw("")];
        for (n, tool) in Tool::ALL.into_iter().enumerate() {
            let size = format!("{:>4}%", percent(s.sizes.of(tool.kind())));
            let text = format!("{}  {:<10}{size}", n + 1, tool.name());
            lines.push(row_line(tool == s.tool, &text));
        }
        lines.push(Line::raw(""));
        for (keys, what) in KEYS {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{keys:<8} "),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::styled(what, dim),
            ]));
        }
        f.render_widget(Paragraph::new(lines).block(side_block(" annotate ")), area);
    }

    fn put(&mut self, mark: Mark) {
        if let Some(s) = &mut self.signing {
            let mark = placed(s, mark);
            s.marks.push(mark);
            self.write_copy();
        }
    }

    /// Pick an image of a signature, from home, where one's own files are,
    /// not beside the PDF, as a mail's are kept; then put it where
    /// `waiting` was clicked, if it was.
    fn pick_signature(&mut self, waiting: Option<Spot>) {
        let Some(s) = &mut self.signing else {
            return;
        };
        let dir = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        match Browser::images(&dir) {
            Ok(browser) => {
                (s.picking, s.waiting) = (true, waiting);
                self.browser = Some(browser);
                self.notice =
                    Some("pick an image of your signature: a PNG, or a photo of it".into());
            }
            Err(e) => self.notice = Some(format!("cannot list {}: {e}", dir.display())),
        }
    }

    /// Whether the list open is picking a signature.
    pub(super) fn picking_signature(&self) -> bool {
        self.signing.as_ref().is_some_and(|s| s.picking)
    }

    /// The list picking a signature closed: `paths` picked, if any were.
    pub(super) fn picked_signature(&mut self, paths: Option<Vec<PathBuf>>) {
        let Some(s) = &mut self.signing else {
            return;
        };
        s.picking = false;
        let waiting = s.waiting.take();
        let Some(path) = paths.and_then(|p| p.into_iter().last()) else {
            return;
        };
        let signature = match Signature::read(&path) {
            Ok(signature) => signature,
            Err(e) => {
                self.notice = Some(format!("{e:#}"));
                return;
            }
        };
        let kept = match sign::keep(s.kept_in.as_deref(), &signature) {
            Ok(kept) => format!("signature kept for next time in {}", kept.display()),
            Err(e) => format!("signature not kept for next time: {e:#}"),
        };
        s.signature = Some(signature);
        s.tool = Tool::Signature;
        match waiting {
            // Put, as the page shows: where it is kept is what is said.
            Some((page, at)) => {
                let mark = placed(s, Mark::new(page, at, What::Signature));
                s.marks.push(mark);
                self.write_copy_saying(Some(kept));
            }
            None => self.notice = Some(kept),
        }
    }

    /// Write the copy with all that is put, and show it.
    fn write_copy(&mut self) {
        self.write_copy_saying(None);
    }

    /// Write the copy with all that is put, and show it, saying `said`, else
    /// how many are put.
    fn write_copy_saying(&mut self, said: Option<String>) {
        if let Err(e) = self.write_stamped() {
            self.notice = Some(format!("cannot sign it: {e:#}"));
            return;
        }
        let Some(s) = &self.signing else {
            return;
        };
        // Read in place of what is shown, the page and zoom kept.
        let said = said.unwrap_or_else(|| "click it to move or size it · u takes it back".into());
        self.notice = Some(said.clone());
        self.read(vec![s.copy.clone()], self.deck.options.clone(), Some(said));
    }

    /// The copy written with all that is put, and the sizes kept for next
    /// time; the wheel's sizing written with it.
    fn write_stamped(&mut self) -> anyhow::Result<()> {
        let Some(s) = &mut self.signing else {
            return Ok(());
        };
        sign::keep_sizes(s.kept_in.as_deref(), &s.sizes);
        let pdf = sign::stamp(&s.original, &s.marks, s.signature.as_ref())?;
        sign::write(&s.copy, &pdf)?;
        s.wheeled = None;
        Ok(())
    }

    /// The PDF as annotated now: the original with all that is put, though
    /// the copy last written is still being read.
    pub(super) fn annotated(&self) -> Option<Vec<u8>> {
        let s = self.signing.as_ref()?;
        sign::stamp(&s.original, &s.marks, s.signature.as_ref()).ok()
    }

    /// Done signing: the copy under the name typed, and said where it is.
    fn finish(&mut self) {
        // Sized by the wheel and not yet written: written here, and read
        // once named, not before, as it is renamed at once.
        let wheeled = self.signing.as_ref().is_some_and(|s| s.wheeled.is_some());
        if wheeled && let Err(e) = self.write_stamped() {
            self.notice = Some(format!("cannot sign it: {e:#}"));
            return;
        }
        let Some(mut s) = self.signing.take() else {
            return;
        };
        let name = s
            .naming
            .take()
            .map(|f| f.text.trim().to_string())
            .unwrap_or_default();
        let name = match name.to_lowercase().ends_with(".pdf") {
            _ if name.is_empty() => render::name(&s.copy),
            true => name,
            false => format!("{name}.pdf"),
        };
        let kept = s.copy.with_file_name(name.replace('/', "-"));
        if kept != s.copy
            && let Err(e) = std::fs::rename(&s.copy, &kept)
        {
            self.notice = Some(format!("cannot name it {name}: {e}"));
            self.signing = Some(s);
            return;
        }
        report(&kept);
        let said = format!("signed: {}", kept.display());
        self.notice = Some(said.clone());
        if kept != s.copy || wheeled {
            self.read(vec![kept], self.deck.options.clone(), Some(said));
        }
    }

    /// Signing cut short by quitting: the copy, if anything is in it, kept
    /// under the name it has.
    pub(super) fn quit_signing(&mut self) {
        if let Some(s) = self.signing.take()
            && !s.marks.is_empty()
        {
            report(&s.copy);
        }
    }

    /// What the status line says while signing: what is being done, and the
    /// keys for it.
    pub(super) fn sign_status(&self) -> Option<(String, &'static str)> {
        let s = self.signing.as_ref()?;
        if let Some(name) = &s.naming {
            return Some((
                format!("save the signed copy as {}", name.shown()),
                "type to rename · enter keep · esc back to signing",
            ));
        }
        if let Some(typing) = &s.typing {
            return Some((
                format!("text: {}", typing.field.shown()),
                "type · tab your name, initials, the date · enter put · esc cancel",
            ));
        }
        Some((
            format!(
                "annotate · click puts the {} at {}% · {} put",
                s.tool.name(),
                percent(s.sizes.of(s.tool.kind())),
                s.marks.len()
            ),
            "1-6 tool · drag move · wheel or [ ] size · u take back · P print · enter done",
        ))
    }
}

/// `mark` as it is put: in a form field to sign or type in, if it was put
/// in one, the signature fitted to it, and text at its left edge, halfway
/// up it, no taller than it; else at the size its kind is put at.
fn placed(s: &Signing, mark: Mark) -> Mark {
    let scale = s.sizes.of(mark.what.kind());
    let fits = matches!(mark.what, What::Signature | What::Text(_));
    let field = fits
        .then(|| sign::field_at(&s.original, mark.page, mark.at))
        .flatten();
    match (&mark.what, field, &s.signature) {
        (What::Signature, Some(field), Some(signature)) => {
            let (at, scale) = sign::fit(signature, field);
            Mark { at, scale, ..mark }
        }
        (What::Text(_), Some([x0, y0, _, y1]), _) => {
            let tallest = 0.7 * (y1 - y0) / sign::TEXT_SIZE;
            let at = (x0 + 2., (y0 + y1) / 2.);
            Mark {
                at,
                scale: scale.min(tallest),
                ..mark
            }
        }
        _ => Mark { scale, ..mark },
    }
}

/// A scale as a percent, as `115`.
fn percent(scale: f32) -> i32 {
    (scale * 100.).round() as i32
}

/// Tell what ran ohp where the signed copy is: written to the file
/// `$OHP_SIGNED` names.
fn report(copy: &Path) {
    if let Some(to) = std::env::var_os("OHP_SIGNED").filter(|t| !t.is_empty()) {
        let _ = std::fs::write(to, format!("{}\n", copy.display()));
    }
}
