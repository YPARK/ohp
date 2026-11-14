//! Printing a deck through MCP, as `P` does on screen: its PDF, a PDF
//! annotated as its copy, handed to CUPS's `lp`, or `lpr`. What is picked
//! is not kept for next time, as `P` keeps it: the user's own picks stay.

use super::{Server, number};
use crate::print::{self, Job, Scale, Sides};
use crate::render;
use serde_json::Value;

impl Server {
    pub(super) fn print(&mut self, args: &Value) -> Result<String, String> {
        let deck = self.deck(args)?;
        let asked = args.get("pages").and_then(Value::as_str).unwrap_or("all");
        let this = asked.trim() == "." || asked.trim().eq_ignore_ascii_case("this");
        let current = match self.shows(&deck) {
            Some(page) => page,
            None if this => {
                return Err("no ohp shows it, so there is no page `this`: give its number".into());
            }
            None => 0,
        };
        let pages = print::pages(asked, current, deck.pages)?;
        let copies = number(args, "copies")?.unwrap_or(1);
        let copies = u32::try_from(copies)
            .ok()
            .filter(|c| (1..=print::MOST_COPIES).contains(c))
            .ok_or(format!("`copies` is 1 to {}", print::MOST_COPIES))?;
        let kept = print::kept(self.config.as_deref());
        let sides = word(args, "sides", "printer, one, two", Sides::from_word)?;
        let scale = word(args, "scale", "auto, fit, actual", Scale::from_word)?;
        // The one printed with last, while CUPS still lists it; else the
        // default, as `P` picks.
        let printer = match args.get("printer").and_then(Value::as_str) {
            Some(p) => Some(p.to_string()),
            None => kept
                .printer
                .filter(|p| print::printers().names.iter().any(|n| n == p)),
        };
        let job = Job {
            // Sent without the paper, the printer would take the PDF's own
            // page size.
            media: print::paper_of(printer.as_deref()).map(|p| p.name),
            printer,
            copies,
            pages: pages.as_ref().map(ToString::to_string),
            sides: sides.unwrap_or(kept.sides),
            scale: scale.unwrap_or(kept.scale),
            wide: print::wide(&deck.sizes, pages.as_ref()),
            ..Job::new(&render::name(&deck.paths[0]))
        };
        let programs: Vec<&std::ffi::OsStr> = self.printing.iter().map(|p| p.as_os_str()).collect();
        print::print_with(&programs, &deck.data, &job).map_err(|e| format!("{e:#}"))
    }

    pub(super) fn printers(&self) -> String {
        let printers = print::printers();
        if printers.names.is_empty() {
            return "no printers: CUPS lists none here".into();
        }
        let kept = print::kept(self.config.as_deref()).printer;
        let mut out = String::new();
        for (i, name) in printers.names.iter().enumerate() {
            let mut said = Vec::new();
            if i == 0 && printers.default {
                said.push("the default");
            }
            if kept.as_deref() == Some(name) {
                said.push("printed with last");
            }
            out.push_str(name);
            if !said.is_empty() {
                out.push_str(&format!(" ({})", said.join(", ")));
            }
            out.push('\n');
        }
        out
    }
}

/// What `args`' `key` names, by one of `words`, if it names anything.
fn word<T>(
    args: &Value,
    key: &str,
    words: &str,
    from_word: fn(&str) -> Option<T>,
) -> Result<Option<T>, String> {
    let Some(asked) = args.get(key).and_then(Value::as_str) else {
        return Ok(None);
    };
    from_word(asked)
        .map(Some)
        .ok_or_else(|| format!("`{key}` is one of {words}, not {asked:?}"))
}
