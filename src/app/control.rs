//! Answering another program, through the socket of [`crate::control`]:
//! what is shown, turning to a slide, and showing another file.

use super::{App, View};
use crate::control::{Listener, Request};

impl App {
    /// Listen for other programs, as `ohp mcp`, if a socket can be made.
    pub(super) fn listen(&mut self) {
        self.control = Listener::new().ok();
    }

    /// Answer what other programs have asked since last looked.
    pub(super) fn answer(&mut self) {
        let Some(control) = &self.control else {
            return;
        };
        let asked: Vec<_> = control.asked.try_iter().collect();
        for (request, reply) in asked {
            let answer = self.answer_one(request);
            let _ = reply.send(answer);
            self.dirty = true;
        }
    }

    pub(super) fn answer_one(&mut self, request: Request) -> Vec<String> {
        match request {
            Request::Status => {
                // Whole, as the asker is somewhere else.
                let whole = |p: &std::path::Path| std::path::absolute(p).unwrap_or(p.into());
                let mut out: Vec<String> = (self.deck.paths.iter())
                    .map(|p| format!("file {}", whole(p).display()))
                    .collect();
                out.push(format!("page {}", self.cur + 1));
                out.push(format!("pages {}", self.deck.pages));
                let view = match self.view {
                    View::Present => "present",
                    View::Grid => "grid",
                };
                out.push(format!("view {view}"));
                out.push(format!("presenting {}", u8::from(self.presenting())));
                let options = &self.deck.options;
                if let Some(paper) = &options.paper {
                    out.push(format!("paper {paper}"));
                }
                out.push(format!("knit {}", u8::from(options.knit)));
                out.push(format!("screen {}", u8::from(options.screen)));
                out
            }
            Request::Goto(n) if n < self.deck.pages => {
                self.cur = n;
                self.view = View::Present;
                vec![format!("ok showing slide {} of {}", n + 1, self.deck.pages)]
            }
            Request::Goto(n) => vec![format!(
                "err there is no slide {}: the deck has {}",
                n + 1,
                self.deck.pages
            )],
            Request::Open(path, n) => {
                if !path.exists() {
                    return vec![format!("err there is no {}", path.display())];
                }
                let same = self.deck.paths.len() == 1
                    && std::fs::canonicalize(&self.deck.paths[0]).ok()
                        == std::fs::canonicalize(&path).ok();
                if same {
                    return self.answer_one(Request::Goto(n.min(self.deck.pages - 1)));
                }
                self.landing = Some(n);
                self.open(vec![path.clone()]);
                vec![format!("ok opening {}", path.display())]
            }
        }
    }
}
