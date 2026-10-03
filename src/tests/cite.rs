use super::*;

fn item(key: &str) -> Item {
    Item {
        key: key.to_string(),
        ..Item::default()
    }
}

#[test]
fn bracketed_citations_have_prefixes_locators_and_suppressed_authors() {
    let pieces = parse("Known [see @doe99, pp. 33-35; also -@smith:2020, chap. 1].");
    assert_eq!(
        pieces,
        [
            Piece::Text("Known "),
            Piece::Cite {
                items: vec![
                    Item {
                        prefix: "see".into(),
                        suffix: "pp. 33-35".into(),
                        ..item("doe99")
                    },
                    Item {
                        prefix: "also".into(),
                        suffix: "chap. 1".into(),
                        suppress_author: true,
                        ..item("smith:2020")
                    },
                ],
                bracketed: true,
            },
            Piece::Text("."),
        ]
    );
}

#[test]
fn citations_in_the_text_end_at_punctuation_and_take_a_locator() {
    let pieces = parse("@doe99 says, as @smith:2020 [p. 4] does. And @roe.");
    let keys: Vec<(&str, &str, bool)> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Cite { items, bracketed } => {
                Some((items[0].key.as_str(), items[0].suffix.as_str(), *bracketed))
            }
            Piece::Text(_) => None,
        })
        .collect();
    assert_eq!(
        keys,
        [
            ("doe99", "", false),
            ("smith:2020", "p. 4", false),
            ("roe", "", false)
        ]
    );
    assert_eq!(pieces.last(), Some(&Piece::Text(".")));
}

#[test]
fn what_only_looks_like_a_citation_is_text() {
    for text in [
        "me@example.com",
        "[a link] and [some @ sign]",
        "[@doe99; no key]",
        "@ alone",
    ] {
        assert!(
            parse(text)
                .iter()
                .all(|p| matches!(p, Piece::Text(_)) || text.starts_with('[')),
            "{text}"
        );
    }
    assert_eq!(parse("me@example.com"), [Piece::Text("me@example.com")]);
    // Not every part is a citation, so the brackets are text, and the key
    // in them a citation in the text.
    let pieces = parse("[@doe99; no key]");
    assert!(matches!(
        &pieces[1],
        Piece::Cite {
            bracketed: false,
            ..
        }
    ));
}

#[test]
fn braced_keys_take_anything() {
    assert_eq!(
        key("{Doe 1999, p.}x"),
        Some(("Doe 1999, p.".to_string(), 14))
    );
    assert_eq!(key("doe99."), Some(("doe99".to_string(), 5)));
    assert_eq!(key("a.b-c"), Some(("a.b-c".to_string(), 5)));
    assert_eq!(key("-x"), None);
}

#[test]
fn a_bibliography_is_read_from_bib_csl_json_and_front_matter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("refs.bib"),
        "@book{smith, author = {Smith, A.}, title = {T}, year = {2020}}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("refs.json"),
        r#"[{"id": "lee", "type": "article-journal", "title": "L", "issued": {"date-parts": [[2018]]}}]"#,
    )
    .unwrap();
    let meta = yaml(
        "bibliography: [refs.bib, refs.json, gone.bib]\n\
         references:\n- id: roe\n  title: R\n\
         csl: https://www.zotero.org/styles/apa\n\
         nocite: '@roe, @*'\n\
         reference-section-title: Works\n",
    )
    .unwrap();
    let bib = Bibliography::load(&meta, dir.path()).unwrap();
    let mut keys: Vec<&str> = bib.keys.iter().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["lee", "roe", "smith"]);
    assert_eq!(bib.sources, ["/bib/0.bib", "/bib/csl.bib"]);
    assert_eq!(bib.style, "apa");
    assert_eq!(bib.nocite, ["roe"]);
    assert!(bib.all);
    assert_eq!(bib.title.as_deref(), Some("Works"));
    assert_eq!(bib.warnings.len(), 1, "{:?}", bib.warnings);
    assert!(bib.warnings[0].contains("gone.bib"));

    assert!(Bibliography::load(&yaml("title: x\n").unwrap(), dir.path()).is_none());
}

#[test]
fn a_style_is_a_csl_file_or_one_typst_knows() {
    let dir = tempfile::tempdir().unwrap();
    let mut bib = Bibliography::default();
    assert_eq!(bib.style(None, dir.path()), "chicago-author-date");
    assert_eq!(bib.style(Some("ieee.csl"), dir.path()), "ieee");
    assert_eq!(
        bib.style(Some("no-such-style.csl"), dir.path()),
        "chicago-author-date"
    );
    assert_eq!(bib.warnings.len(), 1);
}
