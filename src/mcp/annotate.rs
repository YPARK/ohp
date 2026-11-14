//! Annotating and signing a PDF through MCP, as `A` does on screen: a
//! signature, text, the date, ✓ ✕ ● put on its pages, in a copy beside it.
//!
//! Each call draws all the marks it is given afresh on the PDF, its own
//! bytes kept, into the copy: so what is moved, sized or taken back is
//! the next call's list, changed, as the copy on screen is the original
//! with all that is put.

use super::{Server, from_one, number, reply, resolve};
use crate::control::{self, Request};
use crate::render::{self, Deck};
use crate::sign::{self, Field, Mark, Original, Signature, What};
use serde_json::Value;
use std::path::{Path, PathBuf};

impl Server {
    /// The PDF `args` name, else the one on screen, and as it was before
    /// anything was put on it.
    fn pdf_to_sign(&mut self, args: &Value) -> Result<(std::sync::Arc<Deck>, Original), String> {
        // On screen, a copy made here stands for the PDF it was made from:
        // marks are drawn afresh on that, not added to the copy.
        let shown = match args.get("path") {
            None => self.running().and_then(|s| self.original_of(&s.files)),
            Some(_) => None,
        };
        let deck = match shown {
            Some(original) => self.deck(&serde_json::json!({"path": original}))?,
            None => self.deck(args)?,
        };
        if !render::pdf(&deck.paths) || deck.paths.len() != 1 {
            return Err("only a PDF is annotated: name one with `path`".into());
        }
        let original = Original::read(&deck.data).map_err(|e| format!("cannot sign it: {e:#}"))?;
        Ok((deck, original))
    }

    /// Its form fields to sign or type in, and what there is to sign with.
    pub(super) fn fields(&mut self, args: &Value) -> Result<String, String> {
        let (deck, original) = self.pdf_to_sign(args)?;
        let fields: Vec<(usize, Vec<Field>)> = match number(args, "page")? {
            Some(_) => {
                let page = self.page(args, &deck)?;
                vec![(page, sign::fields(&original, page))]
            }
            None => sign::all_fields(&original)
                .into_iter()
                .enumerate()
                .collect(),
        };
        let mut out = format!(
            "{}: {} pages; points from each page's top left, as it is shown\n",
            render::shown(&deck.paths),
            deck.pages
        );
        let mut any = false;
        for (page, fields) in fields {
            any |= !fields.is_empty();
            match deck.sizes.get(page) {
                Some((w, h)) => out.push_str(&format!("page {}: {w:.0}×{h:.0} pt\n", page + 1)),
                None => out.push_str(&format!("page {}\n", page + 1)),
            }
            for f in fields {
                out.push_str(&format!("  {}\n", field_line(&f)));
            }
        }
        if !any {
            out.push_str("no form fields to sign or type in: put marks at x, y\n");
        }
        let config = self.config.as_deref();
        out.push_str(&match sign::kept(config) {
            Some(path) => format!("signature: {}\n", path.display()),
            None => "signature: none kept; give `signature`, an image of one, to put it\n".into(),
        });
        let typed = sign::typed(config);
        if !typed.is_empty() {
            out.push_str(&format!("typed before: {}\n", typed.join(" · ")));
        }
        let sizes = sign::sizes(config);
        out.push_str(&format!(
            "today: {}\nsizes put at: signature {}, text {}, ✓ ✕ ● {}\n",
            sign::today(),
            sizes.of(sign::Kind::Signature),
            sizes.of(sign::Kind::Text),
            sizes.of(sign::Kind::Mark),
        ));
        Ok(out)
    }

    /// Put `marks` on the PDF, into its copy.
    pub(super) fn annotate(&mut self, args: &Value) -> Result<String, String> {
        let (deck, original) = self.pdf_to_sign(args)?;
        let specs = args
            .get("marks")
            .and_then(Value::as_array)
            .filter(|m| !m.is_empty())
            .ok_or("say what to put: `marks`, a list of them")?;
        let config = self.config.clone();
        let config = config.as_deref();
        let signature = signature(args, config, specs)?;
        let sizes = sign::sizes(config);
        // Each page's fields, read once, if a mark names one.
        let named = specs.iter().any(|m| m.get("field").is_some());
        let fields = named.then(|| sign::all_fields(&original));
        let to_put = Put {
            deck: &deck,
            original: &original,
            fields: fields.as_deref(),
            sizes: &sizes,
            signature: signature.as_ref(),
        };
        let mut marks = Vec::new();
        let mut lines = Vec::new();
        for (i, spec) in specs.iter().enumerate() {
            let (mark, field) = self
                .mark(spec, &to_put)
                .map_err(|why| format!("mark {}: {why}", i + 1))?;
            let [x0, y0, x1, y1] = sign::bounds(&mark, signature.as_ref());
            let fitted = field.map_or(String::new(), |f| format!(", fitted to {f}"));
            lines.push(format!(
                "{}. {} on page {}, over [{x0:.1}, {y0:.1}, {x1:.1}, {y1:.1}] pt at {:.0}%{fitted}",
                i + 1,
                shown(&mark.what),
                mark.page + 1,
                mark.scale * 100.
            ));
            marks.push(mark);
        }
        let path = &deck.paths[0];
        let copy = copy(path, args.get("name").and_then(Value::as_str))?;
        // Written over only if made here, from this PDF, and not changed
        // since, as by `A` signing it on screen; else only if asked.
        let ours = self
            .written
            .get(&copy)
            .is_some_and(|(from, stamp)| from == path && *stamp == render::stamp(&copy));
        let replace = args.get("replace").and_then(Value::as_bool) == Some(true);
        if copy.exists() && !ours && !replace {
            return Err(format!(
                "{} is there already, and not as annotate left it: give another `name`, or `replace` true to write over it",
                copy.display()
            ));
        }
        let pdf = sign::stamp(&original, &marks, signature.as_ref())
            .map_err(|e| format!("cannot sign it: {e:#}"))?;
        crate::write(&copy, &pdf).map_err(|e| format!("cannot write {}: {e}", copy.display()))?;
        self.written
            .insert(copy.clone(), (path.clone(), render::stamp(&copy)));
        let mut kept = None;
        if let Some(s) = &signature
            && args.get("signature").is_some()
            && args.get("keep_signature").and_then(Value::as_bool) == Some(true)
        {
            kept = Some(match sign::keep(config, s) {
                Ok(at) => format!("signature kept for next time in {}\n", at.display()),
                Err(e) => format!("signature not kept: {e:#}\n"),
            });
        }
        let mut out = format!("put {} into {}:\n", marks.len(), copy.display());
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(kept.as_deref().unwrap_or(""));
        // On screen, the copy, reread as it changes, is turned to the first
        // mark. The PDF itself is not swapped for it: `A` may be signing it.
        let shown = self.running().map(|s| s.files);
        let page = marks[0].page;
        if shown.as_deref() == Some(std::slice::from_ref(&copy)) {
            let answer = control::ask(&Request::Goto(page)).unwrap_or_default();
            match reply(&answer) {
                Ok(_) => out.push_str(&format!("ohp shows it, at page {}\n", page + 1)),
                Err(why) => out.push_str(&format!("ohp shows it, but did not turn: {why}\n")),
            }
        } else if shown.as_deref() == Some(std::slice::from_ref(path)) {
            out.push_str("ohp shows the PDF: `open` the copy to show it\n");
        }
        out.push_str(&format!(
            "`render` it, path {:?}, to see it; to move, size or take back what is put, annotate {:?} again with the marks changed",
            copy.display().to_string(),
            path.display().to_string()
        ));
        Ok(out)
    }

    /// The mark `spec` says, as it is put; the field it was fitted to.
    fn mark(&mut self, spec: &Value, put: &Put) -> Result<(Mark, Option<String>), String> {
        let Put {
            deck,
            original,
            fields,
            sizes,
            signature,
        } = *put;
        let what = match spec.get("what").and_then(Value::as_str) {
            Some("signature") => What::Signature,
            Some("date") => What::Text(sign::today()),
            Some("text") => {
                let text = spec.get("text").and_then(Value::as_str).unwrap_or("");
                let text = text.trim();
                if text.is_empty() || text.contains('\n') {
                    return Err("give text one line of `text`".into());
                }
                if !sign::settable(text) {
                    return Err(format!("Helvetica has not every letter of {text:?}"));
                }
                What::Text(text.to_string())
            }
            Some("check") => What::Check,
            Some("cross") => What::Cross,
            Some("dot") => What::Dot,
            Some(other) => return Err(format!("cannot put a {other:?}")),
            None => {
                return Err(
                    "say what it is: `what`, signature, text, date, check, cross or dot".into(),
                );
            }
        };
        let scale = match spec.get("size") {
            None | Some(Value::Null) => sizes.of(what.kind()),
            Some(v) => v
                .as_f64()
                .filter(|s| s.is_finite() && *s > 0.)
                .map(|s| (s as f32).clamp(sign::SMALLEST, sign::LARGEST))
                .ok_or_else(|| format!("`size` is a scale, 1 its own size, not {v}"))?,
        };
        // In a field named: its page, and its box.
        if let Some(name) = spec.get("field").and_then(Value::as_str) {
            let only = number(spec, "page")?.map(from_one).transpose()?;
            let (page, field) = (fields.unwrap_or_default().iter().enumerate())
                .filter(|(p, _)| only.is_none_or(|o| o == *p))
                .find_map(|(p, f)| {
                    f.iter()
                        .find(|f| f.name.as_deref() == Some(name))
                        .map(|f| (p, f))
                })
                .filter(|(p, _)| *p < deck.pages)
                .ok_or_else(|| {
                    format!("no field to sign or type in is named {name:?}: `fields` lists them")
                })?;
            let [x0, y0, x1, y1] = field.shown;
            let mark = Mark {
                scale,
                ..Mark::new(page, ((x0 + x1) / 2., (y0 + y1) / 2.), what)
            };
            return Ok((
                sign::fit_to(mark, field.shown, signature),
                Some(format!("field {name:?}")),
            ));
        }
        let page = self.page(spec, deck)?;
        let coordinate = |key: &str| {
            spec.get(key)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .map(|n| n as f32)
                .ok_or_else(|| format!("say where: `{key}`, in points, or a `field`"))
        };
        let at = (coordinate("x")?, coordinate("y")?);
        let (w, h) = deck.sizes[page];
        if !(0. ..=w).contains(&at.0) || !(0. ..=h).contains(&at.1) {
            return Err(format!(
                "({}, {}) is off page {}, {w:.0}×{h:.0} pt, counted from its top left",
                at.0,
                at.1,
                page + 1
            ));
        }
        let mark = Mark {
            scale,
            ..Mark::new(page, at, what)
        };
        if spec.get("fit").and_then(Value::as_bool) == Some(false) {
            return Ok((mark, None));
        }
        let (mark, field) = sign::place(original, mark, signature);
        Ok((mark, field.map(|_| "the form field there".into())))
    }
}

/// What marks are put with: the PDF, its fields if they were read, the
/// sizes each kind is put at, and the signature.
#[derive(Clone, Copy)]
struct Put<'a> {
    deck: &'a Deck,
    original: &'a Original,
    fields: Option<&'a [Vec<Field>]>,
    sizes: &'a sign::Sizes,
    signature: Option<&'a Signature>,
}

/// The signature to sign with: the image `args` name, else the one kept;
/// none if no mark is one.
fn signature(
    args: &Value,
    config: Option<&Path>,
    specs: &[Value],
) -> Result<Option<Signature>, String> {
    let signs = specs
        .iter()
        .any(|m| m.get("what").and_then(Value::as_str) == Some("signature"));
    if !signs {
        return Ok(None);
    }
    let path = match args.get("signature").and_then(Value::as_str) {
        Some(path) => resolve(path)?,
        None => sign::kept(config).ok_or(
            "no signature is kept: give `signature`, a PNG of one, or a photo or scan of it on paper",
        )?,
    };
    let signature = Signature::read(&path).map_err(|e| format!("{}: {e:#}", path.display()))?;
    Ok(Some(signature))
}

/// The copy of `path` marks are put in, beside it: `name`, else its name
/// with `-signed` added; never `path` itself, which is kept as it is.
fn copy(path: &Path, name: Option<&str>) -> Result<PathBuf, String> {
    let copy = sign::named(&sign::copy_of(path, sign::SIGNED), name.unwrap_or(""));
    let same = std::fs::canonicalize(&copy).ok() == std::fs::canonicalize(path).ok();
    if same {
        return Err("the copy cannot be the PDF itself: give another `name`".into());
    }
    Ok(copy)
}

/// What a mark is, as it is said.
fn shown(what: &What) -> String {
    match what {
        What::Signature => "the signature".into(),
        What::Text(t) => format!("{t:?}"),
        What::Check => "✓".into(),
        What::Cross => "✕".into(),
        What::Dot => "●".into(),
    }
}

/// A field, as `fields` lists it.
fn field_line(f: &Field) -> String {
    let [x0, y0, x1, y1] = f.shown;
    let kind = if f.signature { "signature" } else { "text" };
    let name = f
        .name
        .as_deref()
        .map_or_else(|| "(no name)".into(), |n| format!("{n:?}"));
    format!("{kind} field {name}: [{x0:.1}, {y0:.1}, {x1:.1}, {y1:.1}]")
}
