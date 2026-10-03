use super::*;
use crate::render::Deck;

use crate::fixture::{r_here, write_text as write};

fn options(knit: bool) -> Options {
    Options {
        knit,
        ..Options::default()
    }
}

#[test]
fn slides_are_one_page_each() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "talk.md",
        "---\ntitle: A talk\noutput: beamer_presentation\n---\n# Part\n## One\nx $\\alpha$\n## Two\ny\n",
    );
    let deck = Deck::open(&path).unwrap();
    // The title, the part, and two slides.
    assert_eq!(deck.pages, 4);
    let (w, h) = deck.page_size;
    assert!(w > h, "{w}×{h}");
    assert_eq!(deck.name, "talk.md");
}

#[test]
fn a_document_fills_pages_of_its_paper() {
    let dir = tempfile::tempdir().unwrap();
    let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit.\n\n".repeat(120);
    let path = write(dir.path(), "notes.md", &text);
    let letter = Deck::open(&path).unwrap();
    let (w, h) = letter.page_size;
    assert!((w - 612.).abs() < 1. && (h - 792.).abs() < 1., "{w}×{h}");
    let small = Options {
        paper: Some("a6".into()),
        ..options(false)
    };
    let a6 = Deck::open_with(&path, small).unwrap();
    assert!(letter.pages > 1 && a6.pages > letter.pages);
}

#[test]
fn a_formula_typst_cannot_set_is_shown_as_latex() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "math.md", "Fine $x^2$, broken $\\mathbb{}$.\n");
    let done = typeset(&path, &options(false)).unwrap();
    assert!(done.note.is_none());
    assert!(!done.pdf.is_empty());
}

#[test]
fn only_known_paper_sizes_are_taken() {
    assert_eq!(paper("a4").as_deref(), Ok("a4"));
    assert!(paper("presentation-16-9").is_ok());
    assert!(paper("napkin").is_err());
}

#[test]
fn markdown_is_finished_once_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.md", "half a sen");
    assert!(crate::render::finished(&path));
    assert!(!crate::render::finished(&dir.path().join("gone.md")));
}

#[test]
fn chunks_not_knitted_show_as_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.Rmd", "# A\n\n```{r}\n1 + 1\n```\n");
    let done = typeset(&path, &options(false)).unwrap();
    assert!(done.note.is_none());
    assert!(!done.pdf.is_empty());
}

#[test]
fn a_knitted_file_shows_its_output() {
    if !r_here() {
        eprintln!("skipped: no R with knitr");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.Rmd", "```{r p}\n6 * 7\nplot(1:3)\n```\n");
    let done = typeset(&path, &options(true)).unwrap();
    assert!(done.note.is_none(), "{:?}", done.note);
}

/// The text typst sets for a markdown file, page by page.
fn set_text(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let none = HashSet::new();
    let setting = Setting {
        base: path.parent().unwrap(),
        citations: true,
        paper: None,
        raw_math: &none,
        source_only: &none,
        latex: &HashMap::new(),
        broken: &none,
    };
    let doc = markdown::convert(&text, &setting);
    let world = Doc::new(doc.source, &doc.images, &doc.files, &mut HashMap::new());
    let document = typst::compile::<PagedDocument>(&world).output.unwrap();
    fn walk(frame: &typst::layout::Frame, out: &mut String) {
        for (_, item) in frame.items() {
            match item {
                typst::layout::FrameItem::Group(group) => walk(&group.frame, out),
                typst::layout::FrameItem::Text(text) => {
                    out.push_str(&text.text);
                    out.push(' ');
                }
                _ => {}
            }
        }
    }
    let pages = document.pages().iter().map(|page| {
        let mut out = String::new();
        walk(&page.frame, &mut out);
        out
    });
    pages.collect()
}

#[test]
fn citations_are_set_in_the_style_and_the_works_cited_listed() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "refs.bib",
        "@article{doe99, author = {Doe, Jane and Roe, Richard}, title = {On Things}, journaltitle = {J}, year = {1999}}\n",
    );
    write(
        dir.path(),
        "refs.yaml",
        "- id: lee\n  type: book\n  title: Wide\n  author:\n  - family: Lee\n    given: Kim\n  issued: 2018\n",
    );
    let path = write(
        dir.path(),
        "paper.md",
        "---\nbibliography: [refs.bib, refs.yaml]\n---\nKnown [@doe99, p. 3]. @lee shows it [-@lee].\n",
    );
    let pages = set_text(&path).join(" ");
    let squeezed: String = pages.split_whitespace().collect();
    assert!(squeezed.contains("Known(DoeandRoe1999,p.3)."), "{pages}");
    assert!(squeezed.contains("Lee(2018)showsit(2018)."), "{pages}");
    assert!(squeezed.contains("Doe,Jane,andRichardRoe.1999."), "{pages}");
    assert!(squeezed.contains("Lee,Kim.2018.Wide."), "{pages}");

    let done = typeset(&path, &options(false)).unwrap();
    assert!(done.note.is_none(), "{:?}", done.note);
}

#[test]
fn text_right_after_a_formula_shown_as_source_is_not_read_as_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "eq.md", "$$\\undefinedmacro{x}$$(1)\n");
    let done = typeset(&path, &options(false)).unwrap();
    assert!(
        !done
            .note
            .as_deref()
            .is_some_and(|n| n.contains("shown as source")),
        "{:?}",
        done.note
    );
}

#[test]
fn an_image_typst_cannot_read_is_left_out_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fig.png"), b"not a png").unwrap();
    let path = write(dir.path(), "fig.md", "Text.\n\n![a figure](fig.png)\n");
    let note = typeset(&path, &options(false)).unwrap().note.unwrap();
    assert!(note.starts_with("cannot show fig.png"), "{note}");
    assert!(!note.contains("shown as source"), "{note}");
}

#[test]
fn groups_typeset_as_bases() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "iso.md",
        "Carbon $a {}^{14}C$, $\\sum_{i} {x_1}^2 + {a,b}_n$.\n",
    );
    let done = typeset(&path, &options(false)).unwrap();
    assert!(done.note.is_none(), "{:?}", done.note);
}

#[test]
fn a_bibliography_typst_cannot_set_from_leaves_citations_as_written() {
    let dir = tempfile::tempdir().unwrap();
    // Each file reads, but typst takes no key twice.
    let entry = "@book{doe, author = {Doe, J.}, title = {A Book}, year = {1999}}\n";
    write(dir.path(), "a.bib", entry);
    write(dir.path(), "b.bib", entry);
    let path = write(
        dir.path(),
        "cites.md",
        "---\nbibliography: [a.bib, b.bib]\n---\nAs @doe says, $x^2$.\n",
    );
    let note = typeset(&path, &options(false)).unwrap().note.unwrap();
    assert!(note.starts_with("citations left as written"), "{note}");
    assert!(!note.contains("shown as source"), "{note}");
}

#[test]
fn a_stopped_typeset_ends_without_a_pdf() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.md", "Text.\n");
    let stopped = options(false);
    stopped.stop.store(true, Ordering::Relaxed);
    let why = typeset(&path, &stopped).err().unwrap();
    assert!(why.is::<Stopped>(), "{why}");
}
