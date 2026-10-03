use super::*;
use std::sync::LazyLock;

static NONE: LazyLock<HashSet<usize>> = LazyLock::new(HashSet::new);
static NOT_RENDERED: LazyLock<HashMap<Formula, Rendered>> = LazyLock::new(HashMap::new);

/// The setting of a file in `base`, with nothing asked of its formulas.
fn setting(base: &Path) -> Setting<'_> {
    Setting {
        base,
        citations: true,
        paper: None,
        raw_math: &NONE,
        source_only: &NONE,
        latex: &NOT_RENDERED,
    }
}

/// `text` as typst markup, without the prelude every document starts with.
fn typst(text: &str) -> String {
    let source = convert(text, &setting(Path::new("."))).source;
    let (_, body) = source.split_once(STYLE).expect("the style");
    body.to_string()
}

#[test]
fn front_matter_gives_the_title_and_whether_it_is_slides() {
    let (front, body) = front_matter(
        "---\ntitle: \"A talk\"\nauthor:\n  - name: Ann\n  - Bob\noutput:\n  beamer_presentation:\n    slide_level: 2\n---\nbody\n",
    );
    assert_eq!(front.title.as_deref(), Some("A talk"));
    assert_eq!(front.author.as_deref(), Some("Ann, Bob"));
    assert_eq!(front.slides, Some(Split::Headings));
    assert_eq!(front.slide_level, Some(2));
    assert_eq!(body, "body\n");

    let (front, _) = front_matter("---\nmarp: true\n---\n");
    assert_eq!(front.slides, Some(Split::Rules));
    let (front, _) = front_matter("---\ntitle: Notes on beamer\noutput: html_document\n---\n");
    assert_eq!(front.slides, None);
    let (front, body) = front_matter("# No front matter\n");
    assert!(front.title.is_none() && body == "# No front matter\n");
}

#[test]
fn the_slide_level_is_the_highest_heading_followed_by_content() {
    let events: Vec<Event> = Parser::new("# Part\n## Slide\ntext\n## Next\n### Sub\nx\n").collect();
    assert_eq!(slide_level(&events), Some(2));
}

#[test]
fn slides_break_at_headings_and_rules_with_sections_on_their_own() {
    let out =
        typst("---\noutput: ioslides_presentation\n---\n# Part\n## Slide\ntext\n\n---\n\nmore\n");
    assert!(
        out.contains("#pagebreak(weak: true)\n#ohp-section[Part]"),
        "{out}"
    );
    assert!(
        out.contains("#pagebreak(weak: true)\n#ohp-title[Slide]"),
        "{out}"
    );
    assert_eq!(out.matches("#pagebreak").count(), 3, "{out}");
}

#[test]
fn a_document_breaks_only_at_newpage() {
    let out = typst("# One\ntext\n\n---\n\n\\newpage\n\n# Two\n");
    assert!(out.contains("#heading(level: 1)[One]"), "{out}");
    assert!(out.contains("#line(length: 100%"), "{out}");
    assert_eq!(out.matches("#pagebreak").count(), 1, "{out}");
}

#[test]
fn text_that_typst_would_read_as_markup_is_escaped() {
    let out = typst("a_b #x [y] $5 and $6 @ref http://x.org/a <!-- c --> \\- not a list\n");
    assert!(
        out.contains(r"a\_b \#x \[y\] \$5 and \$6 \@ref http:\/\/x.org\/a"),
        "{out}"
    );
    let out = typst("\\- not a list\n\n\\= not a heading\n");
    assert!(
        out.contains("\\- not a list") && out.contains("\\= not a heading"),
        "{out}"
    );
}

#[test]
fn speaker_notes_and_pauses_are_left_out() {
    let out = typst("one\n\n. . .\n\n::: notes\nsecret\n:::\n\n::: {.columns}\ntwo\n:::\n");
    assert!(out.contains("one") && out.contains("two"), "{out}");
    assert!(!out.contains("secret") && !out.contains(". . ."), "{out}");
}

#[test]
fn an_r_chunk_shows_its_code() {
    let out = typst("```{r cars, echo=FALSE}\nplot(cars)\n```\n");
    assert!(
        out.contains("#raw(block: true, lang: \"r\", \"plot(cars)\")"),
        "{out}"
    );
}

#[test]
fn images_here_are_shown_and_others_described() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plot.png"), b"png").unwrap();
    let doc = convert(
        "![a plot](plot.png)\n\n![gone](gone.png) ![web](https://x.org/a.png)\n",
        &setting(dir.path()),
    );
    assert_eq!(
        doc.images,
        [("/img/0.png".to_string(), dir.path().join("plot.png"))]
    );
    assert!(
        doc.source.contains("image(\"/img/0.png\")"),
        "{}",
        doc.source
    );
    assert!(doc.source.contains("\\[image: gone\\]"), "{}", doc.source);
    assert!(doc.source.contains("\\[image: web\\]"), "{}", doc.source);
}

#[test]
fn formulas_are_tracked_and_shown_raw_when_asked() {
    let text = "$x$ and $$\\frac{1}{2}$$ and $\\unknown$\n";
    let doc = convert(text, &setting(Path::new(".")));
    assert_eq!(doc.formulas.len(), 3);
    assert_eq!(&doc.source[doc.formulas[0].clone()], "$x$");
    assert_eq!(&doc.source[doc.formulas[1].clone()], "$ frac(1, 2) $");
    assert_eq!(
        &doc.source[doc.formulas[2].clone()],
        "#raw(\"\\\\unknown\");"
    );

    let raw = HashSet::from([0]);
    let doc = convert(
        text,
        &Setting {
            raw_math: &raw,
            ..setting(Path::new("."))
        },
    );
    assert_eq!(&doc.source[doc.formulas[0].clone()], "#raw(\"x\");");
}

#[test]
fn hard_breaks_are_not_read_as_escapes() {
    let out = typst("[a\\\nb]\n\nc  \nd\n");
    assert!(out.contains("\\[a#linebreak();b\\]"), "{out}");
    assert!(out.contains("c#linebreak();d"), "{out}");
}

#[test]
fn footnotes_are_written_where_they_are_referenced() {
    let out = typst("Text.[^n]\n\n[^n]: The *note*.\n");
    assert!(out.contains("Text.#footnote[The #emph[note];.];"), "{out}");
}

#[test]
fn the_paper_is_chosen_for_slides_and_pages_unless_given() {
    let paper = |text: &str, paper: Option<&str>| {
        let doc = convert(
            text,
            &Setting {
                paper,
                ..setting(Path::new("."))
            },
        );
        doc.source.lines().next().unwrap().to_string()
    };
    assert!(paper("text", None).contains("\"us-letter\""));
    assert!(paper("---\nmarp: true\n---\n", None).contains("\"presentation-16-9\""));
    assert!(paper("text", Some("a4")).contains("\"a4\""));
}

#[test]
fn macros_are_for_latex_and_not_shown() {
    let text = "---\nheader-includes:\n  - \\usepackage{bbm}\n---\n\\newcommand{\\R}{\n  \\mathbb{R}}\n\\DeclareMathOperator{\\tr}{tr}\n\nIn $\\R^n$.\n";
    let doc = convert(text, &setting(Path::new(".")));
    assert_eq!(
        doc.preamble,
        "\\usepackage{bbm}\n\\newcommand{\\R}{\n  \\mathbb{R}}\n\\DeclareMathOperator{\\tr}{tr}"
    );
    assert!(!doc.source.contains("newcommand") && !doc.source.contains("Declare"));
    assert_eq!(doc.unset, [("\\R^n".to_string(), false)]);
}

#[test]
fn a_formula_latex_rendered_sits_on_the_baseline() {
    let rendered = Rendered {
        pdf: Arc::from(&b"%PDF"[..]),
        height: 0.7,
        depth: 0.25,
    };
    let latex = HashMap::from([
        (("\\R".to_string(), false), rendered.clone()),
        (("\\R".to_string(), true), rendered),
    ]);
    let text = "In $\\R$ and $$\\R$$.\n";
    let doc = convert(
        text,
        &Setting {
            latex: &latex,
            ..setting(Path::new("."))
        },
    );
    assert!(doc.unset.is_empty());
    assert_eq!(doc.files.len(), 2);
    assert!(
        doc.source
            .contains("#box(baseline: 0.2500em, image(\"/latex/0.pdf\", height: 0.9500em));"),
        "{}",
        doc.source
    );
    assert!(
        doc.source
            .contains("#align(center, image(\"/latex/1.pdf\", height: 0.9500em))"),
        "{}",
        doc.source
    );

    // Failing to typeset even as rendered, a formula is shown as its source.
    let first = HashSet::from([0]);
    let doc = convert(
        text,
        &Setting {
            source_only: &first,
            latex: &latex,
            ..setting(Path::new("."))
        },
    );
    assert!(doc.unset.is_empty());
    assert_eq!(doc.files.len(), 1);
    assert_eq!(&doc.source[doc.formulas[0].clone()], "#raw(\"\\\\R\");");
}

#[test]
fn a_code_language_is_the_first_word_of_the_info() {
    assert_eq!(language("{r setup, include=FALSE}").as_deref(), Some("r"));
    assert_eq!(language("python").as_deref(), Some("python"));
    assert_eq!(language("{.R}").as_deref(), Some("r"));
    assert_eq!(language(""), None);
}

/// A file in a directory with `refs.bib`, as typst markup and warnings.
fn cited(text: &str) -> (String, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{doe99, author = {Doe, J.}, title = {T}, year = {1999}}\n\
         @book{roe, author = {Roe, R.}, title = {U}, year = {2001}}\n",
    )
    .unwrap();
    let doc = convert(text, &setting(dir.path()));
    let (_, body) = doc.source.split_once(STYLE).expect("the style");
    (body.to_string(), doc.warnings)
}

#[test]
fn citations_are_set_where_front_matter_names_a_bibliography() {
    let (out, warnings) = cited(
        "---\nbibliography: refs.bib\n---\nKnown [see @doe99, p. 3; @roe]. @doe99\nsays [-@roe], not [@gone].\n",
    );
    assert!(
        out.contains(
            "Known see #cite(label(\"doe99\"), supplement: [p. 3]);#cite(label(\"roe\"));."
        ),
        "{out}"
    );
    assert!(
        out.contains("#cite(label(\"doe99\"), form: \"prose\"); says (#cite(label(\"roe\"), form: \"year\");)"),
        "{out}"
    );
    assert!(out.contains("not (#strong[gone?];)."), "{out}");
    assert!(
        out.trim_end().ends_with(
            "#[\n#bibliography((\"/bib/0.bib\",), style: \"chicago-author-date\", title: none, full: false)\n]"
        ),
        "{out}"
    );
    assert_eq!(warnings, ["no gone in the bibliography"]);
}

#[test]
fn without_a_bibliography_an_at_sign_is_text() {
    let out = typst("Mail @doe99 [@doe99].\n");
    assert!(out.contains("Mail \\@doe99 \\[\\@doe99\\]."), "{out}");
    let (out, _) = cited("---\nbibliography: refs.bib\n---\nNothing cited.\n");
    assert!(!out.contains("bibliography("), "{out}");
}

#[test]
fn the_works_cited_are_listed_in_a_refs_div_under_their_title() {
    let (out, _) = cited(
        "---\nbibliography: refs.bib\nreference-section-title: Works\noutput: beamer_presentation\n---\n## One\n\n[@doe99]\n\n::: {#refs}\n:::\n\n## Appendix\n\nx\n",
    );
    let list = out.find("#bibliography(").expect("the list");
    assert!(list < out.find("Appendix").unwrap(), "{out}");
    assert!(
        out.contains("#pagebreak(weak: true)\n#ohp-title[Works]\n#[#set text(size: 0.8em)\n"),
        "{out}"
    );
    assert_eq!(out.matches("#bibliography(").count(), 1, "{out}");
}

#[test]
fn a_suppressed_bibliography_still_sets_citations_and_nocite_lists_works() {
    let (out, _) = cited(
        "---\nbibliography: refs.bib\nsuppress-bibliography: true\nnocite: '@roe'\nlang: de-AT\n---\n[@doe99]\n",
    );
    assert!(out.contains("#[#show bibliography: none\n"), "{out}");
    assert!(out.contains("#cite(label(\"roe\"), form: none)\n"), "{out}");
    let dir = tempfile::tempdir().unwrap();
    let doc = convert("---\nlang: de-AT\n---\nx\n", &setting(dir.path()));
    assert!(
        doc.source
            .contains("#set text(lang: \"de\", region: \"AT\")"),
        "{}",
        doc.source
    );
    assert_eq!(lang("x-1"), None);
}
