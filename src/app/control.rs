//! Answering another program, through the socket of [`crate::control`]:
//! what is shown, turning to a slide, and showing another file.

use super::{App, View};
use crate::control::{Listener, Request, Status};

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
            // Only what changes the screen draws it again.
            self.dirty |= request != Request::Status;
            let answer = self.answer_one(request);
            let _ = reply.send(answer);
        }
    }

    pub(super) fn answer_one(&mut self, request: Request) -> Vec<String> {
        match request {
            Request::Status => {
                // Whole, as the asker is somewhere else.
                let whole = |p: &std::path::PathBuf| std::path::absolute(p).unwrap_or(p.clone());
                let options = &self.deck.options;
                Status {
                    files: self.deck.paths.iter().map(whole).collect(),
                    page: self.cur,
                    pages: self.deck.pages,
                    view: match self.view {
                        View::Present => "present",
                        View::Grid => "grid",
                    }
                    .into(),
                    presenting: self.presenting(),
                    paper: options.paper.clone(),
                    knit: options.knit,
                    screen: options.screen,
                }
                .lines()
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
