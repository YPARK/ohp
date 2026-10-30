//! Signing the PDF presented, as `S` begins it: the pages turned and zoomed
//! as ever, and each click puts what is picked there, a signature, text,
//! the date, or a ✓ ✕ ● mark, in a copy beside the PDF, shown in its place
//! as each is put. `u` takes the last back, `[` `]` make it smaller or
//! larger, and Shift and the arrows move it. `esc` asks what to call the
//! copy, its name with `-signed` added, the `signed` picked to type over.
//!
//! The signature is the one kept from the last picked, or `$OHP_SIGNATURE`;
//! with none, or on `N`, an image of one is picked, a PNG or a photo of it
//! on paper, and kept for next time.
//!
//! `$OHP_AS` names the PDF, where its file is named otherwise, as letterbox
//! keeps a message's files; and the copy kept is written to the file
//! `$OHP_SIGNED` names, if it does, for what ran ohp to find it.

use super::{App, Then, View};
use crate::browse::Browser;
use crate::render;
use crate::sign::{self, Mark, Signature, What};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    fn name(self) -> &'static str {
        match self {
            Tool::Signature => "signature",
            Tool::Text => "text",
            Tool::Date => "date",
            Tool::Check => "✓",
            Tool::Cross => "✕",
            Tool::Dot => "●",
        }
    }
}

/// A line being typed: its text, the cursor, in chars, and what is picked,
/// typed over, if anything is.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Field {
    pub(super) text: String,
    at: usize,
    picked: Option<(usize, usize)>,
}

impl Field {
    /// `text`, all of it picked, to be typed over or kept.
    fn offered(text: &str) -> Self {
        Field::picking(text, 0, text.chars().count())
    }

    /// `text`, the chars from `from` to `to` picked.
    fn picking(text: &str, from: usize, to: usize) -> Self {
        Field {
            text: text.to_string(),
            at: to,
            picked: Some((from, to)),
        }
    }

    fn byte(&self, c: usize) -> usize {
        self.text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i)
    }

    /// Cut what is picked; whether anything was.
    fn cut(&mut self) -> bool {
        let Some((a, b)) = self.picked.take() else {
            return false;
        };
        let (from, to) = (self.byte(a), self.byte(b));
        self.text.replace_range(from..to, "");
        self.at = a;
        true
    }

    fn key(&mut self, key: KeyEvent) {
        let len = self.text.chars().count();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('u') if ctrl => *self = Field::default(),
            KeyCode::Char(c) if !ctrl => {
                self.cut();
                let i = self.byte(self.at);
                self.text.insert(i, c);
                self.at += 1;
            }
            KeyCode::Backspace => {
                if !self.cut() && self.at > 0 {
                    self.at -= 1;
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Delete => {
                if !self.cut() && self.at < len {
                    let i = self.byte(self.at);
                    self.text.remove(i);
                }
            }
            KeyCode::Left => {
                self.at = self
                    .picked
                    .take()
                    .map_or(self.at.saturating_sub(1), |p| p.0)
            }
            KeyCode::Right => self.at = self.picked.take().map_or((self.at + 1).min(len), |p| p.1),
            KeyCode::Home => (self.at, self.picked) = (0, None),
            KeyCode::End => (self.at, self.picked) = (len, None),
            _ => {}
        }
    }

    /// The text, what is picked in brackets, else the cursor shown.
    pub(super) fn shown(&self) -> String {
        let chars: Vec<char> = self.text.chars().collect();
        let part = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
        match self.picked {
            Some((a, b)) => format!("{}[{}]{}", part(0, a), part(a, b), part(b, chars.len())),
            None => format!("{}▏{}", part(0, self.at), part(self.at, chars.len())),
        }
    }
}

/// Text being typed to put where it was clicked, and what Tab offers.
pub(super) struct Typing {
    page: usize,
    at: (f32, f32),
    pub(super) field: Field,
    offers: Vec<String>,
    offered: usize,
}

/// The PDF being signed.
pub(super) struct Signing {
    /// The PDF as it was before anything was put on it.
    pub(super) original: Arc<Vec<u8>>,
    /// The PDF's own file.
    pub(super) source: PathBuf,
    /// The copy what is put goes in, as it is put.
    pub(super) copy: PathBuf,
    /// What has been put, the last put last.
    pub(super) marks: Vec<Mark>,
    pub(super) tool: Tool,
    pub(super) signature: Option<Signature>,
    /// The signature is being picked, and a click waits to put it.
    pub(super) picking: Option<Option<(usize, (f32, f32))>>,
    pub(super) typing: Option<Typing>,
    /// What to call the copy, being typed, once signing is done.
    pub(super) naming: Option<Field>,
}

/// The name the PDF at `path` is known by: `$OHP_AS`, else its own.
fn known_as(path: &Path) -> PathBuf {
    match std::env::var_os("OHP_AS").filter(|n| !n.is_empty()) {
        Some(name) => path.with_file_name(Path::new(&name).file_name().unwrap_or(&name)),
        None => path.to_path_buf(),
    }
}

impl App {
    /// Begin signing the PDF presented.
    pub(super) fn sign(&mut self) {
        if self.signing.is_some() {
            self.pick_tool(Tool::Signature);
            return;
        }
        if !render::pdf(&self.deck.paths) || !self.links.is_empty() {
            self.notice = Some("only a PDF here is signed: S on one".into());
            return;
        }
        let source = self.deck.paths[0].clone();
        let copy = sign::copy_of(&known_as(&source), sign::SIGNED);
        self.view = View::Present;
        self.signing = Some(Box::new(Signing {
            original: self.deck.data.clone(),
            source,
            copy,
            marks: Vec::new(),
            tool: Tool::Signature,
            signature: None,
            picking: None,
            typing: None,
            naming: None,
        }));
        self.notice = Some(match sign::kept() {
            Some(_) => "signing: click where your signature goes".into(),
            None => "signing: click where your signature goes, and pick an image of it".into(),
        });
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
                            sign::remember(&text);
                        }
                        self.put(Mark::new(typing.page, typing.at, What::Text(text)));
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
            KeyCode::Esc if s.marks.is_empty() => {
                self.signing = None;
                self.notice = Some("nothing signed".into());
            }
            KeyCode::Esc => {
                let name = render::name(&s.copy);
                let stem = name.trim_end_matches(".pdf").chars().count();
                let added = sign::SIGNED.chars().count();
                s.naming = Some(Field::picking(&name, stem - added, stem));
            }
            KeyCode::Char('S') => self.pick_tool(Tool::Signature),
            KeyCode::Char('T') => self.pick_tool(Tool::Text),
            KeyCode::Char('D') => self.pick_tool(Tool::Date),
            KeyCode::Char('C') => self.pick_tool(Tool::Check),
            KeyCode::Char('X') => self.pick_tool(Tool::Cross),
            KeyCode::Char('O') => self.pick_tool(Tool::Dot),
            KeyCode::Char('N') => self.pick_signature(None),
            KeyCode::Char('u') => match s.marks.pop() {
                Some(_) => self.write_copy(),
                None => self.notice = Some("nothing put to take back".into()),
            },
            KeyCode::Char('[') => self.change_last(|m| m.scale = (m.scale / 1.15).max(0.2)),
            KeyCode::Char(']') => self.change_last(|m| m.scale = (m.scale * 1.15).min(6.)),
            KeyCode::Left if shift => self.change_last(|m| m.at.0 -= 2.),
            KeyCode::Right if shift => self.change_last(|m| m.at.0 += 2.),
            KeyCode::Up if shift => self.change_last(|m| m.at.1 -= 2.),
            KeyCode::Down if shift => self.change_last(|m| m.at.1 += 2.),
            _ => return false,
        }
        true
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

    /// A click on the presented page while signing: what is picked put there.
    pub(super) fn sign_click(&mut self, at: Position) {
        let Some((page, point)) = self.page_point(at) else {
            return;
        };
        let Some(s) = &mut self.signing else {
            return;
        };
        if s.typing.is_some() || s.naming.is_some() {
            return;
        }
        let what = match s.tool {
            Tool::Signature => {
                if s.signature.is_none() {
                    s.signature = sign::kept().and_then(|path| Signature::read(&path).ok());
                }
                if s.signature.is_none() {
                    return self.pick_signature(Some((page, point)));
                }
                What::Signature
            }
            Tool::Date => What::Text(sign::today()),
            Tool::Check => What::Check,
            Tool::Cross => What::Cross,
            Tool::Dot => What::Dot,
            Tool::Text => {
                let mut offers = sign::typed();
                // What was typed last, picked: typed over, or kept on ⏎.
                let field = offers
                    .first()
                    .map_or_else(Field::default, |t| Field::offered(t));
                if let Some(first) = offers.first() {
                    let initials = sign::initials(first);
                    if initials.chars().count() > 1 && !offers.contains(&initials) {
                        offers.push(initials);
                    }
                }
                offers.push(sign::today());
                s.typing = Some(Typing {
                    page,
                    at: point,
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

    fn put(&mut self, mark: Mark) {
        if let Some(s) = &mut self.signing {
            s.marks.push(mark);
            self.write_copy();
        }
    }

    /// Pick an image of a signature, from the PDF's folder; then put it
    /// where `waiting` was clicked, if it was.
    fn pick_signature(&mut self, waiting: Option<(usize, (f32, f32))>) {
        let Some(s) = &mut self.signing else {
            return;
        };
        let dir = s.source.parent().filter(|d| !d.as_os_str().is_empty());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = match (dir, home) {
            // A message's files are kept where its signature never is.
            (_, Some(home)) if std::env::var_os("OHP_AS").is_some() => home,
            (Some(dir), _) => dir.to_path_buf(),
            (None, _) => PathBuf::from("."),
        };
        match Browser::images(&dir) {
            Ok(browser) => {
                s.picking = Some(waiting);
                self.browser = Some(browser);
                self.notice =
                    Some("pick an image of your signature: a PNG, or a photo of it".into());
            }
            Err(e) => self.notice = Some(format!("cannot list {}: {e}", dir.display())),
        }
    }

    /// Whether the list open is picking a signature.
    pub(super) fn picking_signature(&self) -> bool {
        self.signing.as_ref().is_some_and(|s| s.picking.is_some())
    }

    /// The list picking a signature closed: `paths` picked, if any were.
    pub(super) fn picked_signature(&mut self, paths: Option<Vec<PathBuf>>) {
        let Some(s) = &mut self.signing else {
            return;
        };
        let waiting = s.picking.take().flatten();
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
        self.notice = Some(match sign::keep(&signature) {
            Ok(kept) => format!("signature kept for next time in {}", kept.display()),
            Err(e) => format!("signature not kept for next time: {e:#}"),
        });
        s.signature = Some(signature);
        s.tool = Tool::Signature;
        if let Some((page, at)) = waiting {
            let notice = self.notice.take();
            self.put(Mark::new(page, at, What::Signature));
            self.notice = self.notice.take().or(notice);
        }
    }

    /// Write the copy with all that is put, and show it.
    fn write_copy(&mut self) {
        let Some(s) = &self.signing else {
            return;
        };
        let written = sign::stamp(&s.original, &s.marks, s.signature.as_ref())
            .and_then(|pdf| Ok(sign::write(&s.copy, &pdf)?));
        if let Err(e) = written {
            self.notice = Some(format!("cannot sign it: {e:#}"));
            return;
        }
        self.notice = Some(format!("{} put · u takes it back", s.marks.len()));
        // Written here, and read next: not a change on disk to watch settle.
        if self.deck.paths == [s.copy.clone()] {
            self.stamp = self.deck.stamps();
            self.settling = None;
        }
        if self.loading.is_some() {
            self.then = Some(Then::Signed);
        } else {
            self.show_copy();
        }
    }

    /// Read the copy, in place of what is shown, the page and zoom kept.
    pub(super) fn show_copy(&mut self) {
        if let Some(s) = &self.signing {
            self.read(vec![s.copy.clone()], self.deck.options.clone());
        }
    }

    /// Done signing: the copy under the name typed, and said where it is.
    fn finish(&mut self) {
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
        if kept != s.copy {
            if let Err(e) = std::fs::rename(&s.copy, &kept) {
                self.notice = Some(format!("cannot name it {name}: {e}"));
                self.signing = Some(s);
                return;
            }
            self.read(vec![kept.clone()], self.deck.options.clone());
        }
        report(&kept);
        let said = format!("signed: {}", kept.display());
        if self.loading.is_some() {
            self.read_said = Some(said);
        } else {
            self.notice = Some(said);
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

    /// The page shown at `at`, and the point of it, in points from its top
    /// left, as it is shown.
    fn page_point(&self, at: Position) -> Option<(usize, (f32, f32))> {
        let (job, area) = self.drawn.get()?;
        if !area.contains(at) {
            return None;
        }
        let (dx, dy) = (at.x - area.x, at.y - area.y);
        let font = self.picker.font_size();
        let (fw, fh) = (f32::from(font.width), f32::from(font.height));
        let key = job.key;
        let size = *self.deck.sizes.get(key.page)?;
        let scale = render::fit(key, (fw, fh), size) * key.zoom.scale();
        let point = |pan: u16, cell: u16, size: f32| {
            (f32::from(pan) + f32::from(cell) + 0.5) * size / scale
        };
        Some((
            key.page,
            (point(key.zoom.x, dx, fw), point(key.zoom.y, dy, fh)),
        ))
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
                "signing · click puts the {} · {} put",
                s.tool.name(),
                s.marks.len()
            ),
            "S signature · T text · D date · C ✓ · X ✕ · O ● · N new signature · u undo · [ ] size · shift-arrows move · n/p +/- turn, zoom · esc done",
        ))
    }
}

/// Tell what ran ohp where the signed copy is: written to the file
/// `$OHP_SIGNED` names.
fn report(copy: &Path) {
    if let Some(to) = std::env::var_os("OHP_SIGNED").filter(|t| !t.is_empty()) {
        let _ = std::fs::write(to, format!("{}\n", copy.display()));
    }
}
