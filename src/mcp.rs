//! `ohp mcp`: a Model Context Protocol server, over stdio, a JSON-RPC
//! message a line. Through it an assistant shows a file in the ohp running
//! here, or opens one in a window of its own, turns its slides, and lists,
//! renders and reads the pages of any deck, PDF, markdown or the rest, and
//! reads and writes their speaker notes.
//!
//! What is shown is asked of the running ohp through [`crate::control`];
//! the pages themselves are read here, so they can be read with no ohp
//! running, and a deck is read once while its files are unchanged.

use crate::control::{self, Request, Status};
use crate::notes;
use crate::render::{self, Deck, Options, Stamp};
use base64::Engine;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The protocol spoken where the client names none.
const PROTOCOL: &str = "2025-06-18";

/// Most text one answer holds, so a long paper does not flood the client.
const MOST_TEXT: usize = 200_000;

/// Serve the client on stdin and stdout until it goes.
pub fn serve() -> anyhow::Result<()> {
    let mut server = Server::default();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(&message),
            Err(e) => Some(error(Value::Null, -32700, &format!("not JSON: {e}"))),
        };
        if let Some(answer) = answer {
            writeln!(out, "{answer}")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// What the ohp running here shows, if one is, and answers.
fn running() -> Option<Status> {
    control::ask(&Request::Status)
        .map(|lines| Status::parse(&lines))
        .filter(|s| !s.files.is_empty())
}

/// A slide or page counted from 1, from 0.
fn from_one(n: u64) -> Result<usize, String> {
    (n as usize)
        .checked_sub(1)
        .ok_or_else(|| "slides and pages are counted from 1".into())
}

/// `args`' whole number `key`, if given; an error if it is anything else.
fn number(args: &Value, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a whole number, not {v}")),
    }
}

#[derive(Default)]
pub struct Server {
    kept: Option<Kept>,
    /// What the running ohp showed, asked once a tool call.
    shown: Option<Option<Status>>,
}

/// The deck last read, when its files were changed then, and its pages'
/// titles, once they are found.
struct Kept {
    paths: Vec<PathBuf>,
    stamps: Vec<Stamp>,
    deck: Arc<Deck>,
    titles: Option<Arc<Vec<Option<String>>>>,
}

impl Server {
    /// The answer to `message`, if it wants one.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // A notification, or an answer to something not asked: no reply.
        let id = id?;
        let result = match method {
            "initialize" => {
                let version = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL);
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "ohp", "title": "ohp, an overhead projector", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "ohp shows slides and documents (PDF, markdown, R Markdown, HTML, images) in the terminal. Use `open` to put a file in front of the user, `goto` to turn to a slide while you talk about it, `slides`, `text` and `render` to read pages yourself, and `notes` for speaker notes. Pages are counted from 1.",
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools()}),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call(name, &args) {
                    Ok(content) => json!({"content": content, "isError": false}),
                    Err(why) => {
                        json!({"content": [{"type": "text", "text": why}], "isError": true})
                    }
                }
            }
            _ => return Some(error(id, -32601, &format!("no method {method}"))),
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        let text = |t: String| Ok(vec![json!({"type": "text", "text": t})]);
        // Asked anew each call: the screen changes between them.
        self.shown = None;
        match name {
            "status" => text(status(self.running())),
            "open" => text(open(args)?),
            "goto" => text(self.goto(args)?),
            "slides" => text(self.slides(args)?),
            "text" => text(self.text(args)?),
            "render" => self.render(args),
            "notes" => text(self.notes(args)?),
            _ => Err(format!("ohp has no tool {name}")),
        }
    }

    /// What the ohp running here shows, if one is, and answers: asked
    /// once a call, however many times it is wanted.
    fn running(&mut self) -> Option<Status> {
        self.shown.get_or_insert_with(running).clone()
    }

    /// The deck in `path`, else the one the running ohp shows.
    fn deck(&mut self, args: &Value) -> Result<Arc<Deck>, String> {
        // A file named is read with no code run: its R chunks are shown,
        // not knitted. The deck on screen is read as the ohp showing it
        // reads it, so its pages are the ones on screen.
        let (paths, options) = match args.get("path").and_then(Value::as_str) {
            Some(path) => (
                vec![resolve(path)?],
                Options {
                    knit: false,
                    ..Options::default()
                },
            ),
            None => {
                let shown = self.running().ok_or(
                    "no ohp is running here, or it does not answer: name the file with `path`, or `open` one",
                )?;
                let options = Options {
                    paper: shown.paper.clone(),
                    knit: shown.knit,
                    screen: shown.screen,
                    ..Options::default()
                };
                (shown.files, options)
            }
        };
        let stamps: Vec<Stamp> = paths.iter().map(|p| render::stamp(p)).collect();
        if let Some(kept) = &self.kept
            && kept.paths == paths
            && kept.stamps == stamps
        {
            return Ok(kept.deck.clone());
        }
        let deck = Arc::new(
            Deck::open_with(&paths, options)
                .map_err(|e| format!("cannot read {}: {e:#}", render::shown(&paths)))?,
        );
        self.kept = Some(Kept {
            paths,
            stamps,
            deck: deck.clone(),
            titles: None,
        });
        Ok(deck)
    }

    /// Each page's title of `deck`, as its text has it: found once a deck.
    fn titles(&mut self, deck: &Arc<Deck>) -> Arc<Vec<Option<String>>> {
        if let Some(kept) = &self.kept
            && Arc::ptr_eq(&kept.deck, deck)
            && let Some(titles) = &kept.titles
        {
            return titles.clone();
        }
        let pages: Vec<usize> = (0..deck.pages).collect();
        let titles: Arc<Vec<Option<String>>> = Arc::new(
            crate::text::of_each(deck.data.clone(), &pages)
                .into_iter()
                .map(|t| t.and_then(|t| t.title()))
                .collect(),
        );
        if let Some(kept) = &mut self.kept
            && Arc::ptr_eq(&kept.deck, deck)
        {
            kept.titles = Some(titles.clone());
        }
        titles
    }

    /// The page, from 0, `args` name from 1; else the running ohp's, where
    /// it shows the deck asked about; else the first.
    fn page(&mut self, args: &Value, deck: &Deck) -> Result<usize, String> {
        let page = match number(args, "page")? {
            Some(n) => from_one(n)?,
            None => self
                .running()
                .filter(|s| s.files == deck.paths)
                .map_or(0, |s| s.page),
        };
        if page >= deck.pages {
            return Err(format!(
                "there is no page {}: it has {}",
                page + 1,
                deck.pages
            ));
        }
        Ok(page)
    }

    fn goto(&mut self, args: &Value) -> Result<String, String> {
        let shown = self
            .running()
            .ok_or("no ohp is running here: `open` a file first")?;
        let page = match (number(args, "page")?, args.get("title")) {
            (Some(n), _) => from_one(n)?,
            (None, Some(Value::String(title))) => {
                let deck = self.deck(&json!({}))?;
                let titles = self.titles(&deck);
                let want = title.to_lowercase();
                titles
                    .iter()
                    .position(|t| {
                        t.as_deref()
                            .is_some_and(|t| t.to_lowercase().contains(&want))
                    })
                    .ok_or_else(|| format!("no slide's title has {title:?} in it"))?
            }
            _ => return Err("say which slide: `page`, from 1, or words of its `title`".into()),
        };
        let answer = control::ask(&Request::Goto(page)).unwrap_or_default();
        reply(&answer).map(|ok| {
            let name = shown
                .files
                .first()
                .map(|p| render::name(p))
                .unwrap_or_default();
            format!("{ok} ({name})")
        })
    }

    fn slides(&mut self, args: &Value) -> Result<String, String> {
        let deck = self.deck(args)?;
        let titles = self.titles(&deck);
        let notes = all_notes(&deck);
        let shown = self.running().filter(|s| s.files == deck.paths);
        let mut out = format!("{}: {} pages\n", render::shown(&deck.paths), deck.pages);
        for (i, title) in titles.iter().enumerate() {
            let (w, h) = deck.sizes[i];
            let here = if shown.as_ref().is_some_and(|s| s.page == i) {
                "  ← on screen"
            } else {
                ""
            };
            let noted = if notes.contains_key(&i) {
                " · notes"
            } else {
                ""
            };
            let title = title.as_deref().unwrap_or("(no text)");
            out.push_str(&format!(
                "{:>3}. {title}  [{w:.0}×{h:.0} pt{noted}]{here}\n",
                i + 1
            ));
        }
        Ok(out)
    }

    fn text(&mut self, args: &Value) -> Result<String, String> {
        let deck = self.deck(args)?;
        let pages: Vec<usize> = match number(args, "page")? {
            Some(_) => vec![self.page(args, &deck)?],
            None => (0..deck.pages).collect(),
        };
        let mut out = String::new();
        // Read some pages at a time, so no more are read than are given.
        for chunk in pages.chunks(8) {
            let texts = crate::text::of_each(deck.data.clone(), chunk);
            for (page, text) in chunk.iter().zip(texts) {
                if pages.len() > 1 {
                    out.push_str(&format!("--- page {} ---\n", page + 1));
                }
                out.push_str(&text.map(|t| t.plain()).unwrap_or_default());
                out.push('\n');
                if out.len() > MOST_TEXT {
                    out.push_str(&format!(
                        "… cut here, after page {}: ask for a `page` to read on",
                        page + 1
                    ));
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    fn render(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let deck = self.deck(args)?;
        let page = self.page(args, &deck)?;
        let width = number(args, "width")?.unwrap_or(1200).clamp(100, 4000) as u32;
        let png = render::png(deck.data.clone(), page, width).map_err(|e| format!("{e:#}"))?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        Ok(vec![
            json!({"type": "image", "data": data, "mimeType": "image/png"}),
            json!({"type": "text", "text": format!(
                "page {} of {} of {}, {width} pixels across",
                page + 1,
                deck.pages,
                render::shown(&deck.paths)
            )}),
        ])
    }

    fn notes(&mut self, args: &Value) -> Result<String, String> {
        let deck = self.deck(args)?;
        if let Some(text) = args.get("text").and_then(Value::as_str) {
            if number(args, "page")?.is_none() {
                return Err("say which slide's notes to write: `page`, from 1".into());
            }
            let page = self.page(args, &deck)?;
            let source = deck.source(page);
            let file = notes::file_for(&deck.paths[source]);
            let within = page - deck.starts()[source];
            notes::save(&file, within, text)
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
            return Ok(format!(
                "wrote the notes of slide {} into {}; ohp shows them as it presents",
                page + 1,
                file.display()
            ));
        }
        let all = all_notes(&deck);
        if number(args, "page")?.is_some() {
            let page = self.page(args, &deck)?;
            return Ok(all
                .get(&page)
                .cloned()
                .unwrap_or_else(|| format!("slide {} has no notes", page + 1)));
        }
        if all.is_empty() {
            return Ok("no slide has notes".into());
        }
        Ok(all
            .iter()
            .map(|(p, n)| format!("# {}\n{n}\n", p + 1))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn status(shown: Option<Status>) -> String {
    let Some(shown) = shown else {
        return "no ohp is running here; `open` shows a file in a new window".into();
    };
    let view = match (shown.presenting, shown.view.as_str()) {
        (true, "grid") => "the presenter, picking in the grid",
        (true, _) => "presenting to a second window",
        (false, "grid") => "the grid of slides",
        _ => "the slide",
    };
    format!(
        "ohp shows {}, slide {} of {}, as {view}",
        render::shown(&shown.files),
        shown.page + 1,
        shown.pages
    )
}

fn open(args: &Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("say which file to `open`: `path`")?;
    let path = resolve(path)?;
    let page = number(args, "page")?
        .map(from_one)
        .transpose()?
        .unwrap_or(0);
    if let Some(answer) = control::ask(&Request::Open(path.clone(), page)) {
        return reply(&answer);
    }
    let args = [
        "--page".into(),
        (page + 1).to_string().into(),
        path.clone().into_os_string(),
    ];
    match crate::link::open_terminal(&args, "ohp") {
        Ok(terminal) => Ok(format!(
            "no ohp was running: opened {} in a new {terminal} window",
            path.display()
        )),
        Err(command) => Err(format!(
            "no ohp is running, and no window can be opened from here: ask the user to run {command}"
        )),
    }
}

/// `ok …` as what was done, `err …` as why not.
fn reply(answer: &[String]) -> Result<String, String> {
    match answer.first().map(|l| l.split_once(' ').unwrap_or((l, ""))) {
        Some(("ok", what)) => Ok(what.to_string()),
        Some(("err", why)) => Err(why.to_string()),
        _ => Err("ohp did not answer".into()),
    }
}

/// `path` made whole: `~` as home, and relative to here.
fn resolve(path: &str) -> Result<PathBuf, String> {
    let path = match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| Path::new(&home).join(rest))
            .ok_or("no home to find ~ in")?,
        None => PathBuf::from(path),
    };
    let whole = std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !whole.exists() {
        return Err(format!("there is no {}", whole.display()));
    }
    Ok(whole)
}

/// The deck's notes, from it and the files beside its files.
fn all_notes(deck: &Deck) -> notes::Notes {
    let beside = notes::Beside::new(&deck.paths, &deck.starts());
    notes::merge(deck.speaker.clone(), beside.read())
}

/// The tools, as `tools/list` gives them.
fn tools() -> Value {
    let page =
        json!({"type": "integer", "minimum": 1, "description": "Slide or page number, from 1."});
    let path = json!({"type": "string", "description": "A PDF, markdown, R Markdown, HTML or image file, read with no code in it run: R Markdown's chunks are shown, not knitted. Without it, the deck the running ohp shows, as it shows it."});
    let read_only = json!({"readOnlyHint": true, "openWorldHint": false});
    json!([
        {
            "name": "status",
            "title": "What ohp shows",
            "description": "What the ohp running here shows: the file, the slide on screen, how many there are, and whether it is presenting to a second window.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": read_only,
        },
        {
            "name": "open",
            "title": "Show a file",
            "description": "Show a file to the user in the ohp running here, from the slide given; with none running, open ohp on it in a new terminal window.",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string", "description": "The file to show: PDF, markdown, R Markdown, HTML or an image."}, "page": page}, "required": ["path"]},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
        },
        {
            "name": "goto",
            "title": "Turn to a slide",
            "description": "Turn the running ohp to a slide, by number or by words of its title, as you talk about it. Presenting, the audience sees it too.",
            "inputSchema": {"type": "object", "properties": {"page": page, "title": {"type": "string", "description": "Words of the slide's title, matched in any case."}}},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
        },
        {
            "name": "slides",
            "title": "List the slides",
            "description": "The deck's page count, and each page's title, size and whether it has speaker notes, marking the one on screen.",
            "inputSchema": {"type": "object", "properties": {"path": path}},
            "annotations": read_only,
        },
        {
            "name": "text",
            "title": "Read a page's text",
            "description": "The text of one page, or of every page, in the order it is read.",
            "inputSchema": {"type": "object", "properties": {"path": path, "page": page}},
            "annotations": read_only,
        },
        {
            "name": "render",
            "title": "See a page",
            "description": "A page as a PNG image, to see its layout, figures and tables as the reader does. Without `page`, the one on screen.",
            "inputSchema": {"type": "object", "properties": {"path": path, "page": page, "width": {"type": "integer", "minimum": 100, "maximum": 4000, "description": "Pixels across; 1200 if not given."}}},
            "annotations": read_only,
        },
        {
            "name": "notes",
            "title": "Speaker notes",
            "description": "Read a slide's speaker notes, or every slide's; with `text`, write the slide's notes, in markdown, into the notes file beside the deck (talk.md for talk.pdf), which ohp's presenter view shows.",
            "inputSchema": {"type": "object", "properties": {"path": path, "page": page, "text": {"type": "string", "description": "The slide's notes, in markdown, in place of what it has; empty removes them."}}},
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
        },
    ])
}

#[cfg(test)]
#[path = "tests/mcp.rs"]
mod tests;
