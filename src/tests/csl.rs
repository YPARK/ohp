use super::*;
use serde_json::json;

#[test]
fn csl_items_are_biblatex_entries_hayagriva_reads() {
    let items = [
        json!({
            "id": "lee2018",
            "type": "article-journal",
            "title": "Deep & Wide: 100% {models}",
            "author": [
                {"family": "Rossum", "given": "Guido", "non-dropping-particle": "van"},
                {"literal": "The ABC Consortium"}
            ],
            "container-title": "Nature Methods",
            "volume": 15,
            "issue": "2",
            "page": "100-110",
            "issued": {"date-parts": [[2018, 2, 1]]},
            "DOI": "10.1038/nm_1234",
            "URL": "https://x.org/~lee?a=1&b=2"
        }),
        json!({"id": "roe", "type": "thesis", "title": "T", "issued": 1995, "publisher": "MIT"}),
        json!({"title": "no id"}),
    ];
    let bib = biblatex(&items);
    assert!(bib.starts_with("@article{lee2018,\n"), "{bib}");
    assert!(
        bib.contains("author = {van Rossum, Guido and {The ABC Consortium}}"),
        "{bib}"
    );
    assert!(
        bib.contains("title = {Deep \\& Wide: 100\\% \\{models\\}}"),
        "{bib}"
    );
    assert!(bib.contains("journaltitle = {Nature Methods}"), "{bib}");
    assert!(bib.contains("date = {2018-02-01}"), "{bib}");
    assert!(bib.contains("publisher = {MIT}"), "{bib}");
    assert!(!bib.contains("no id"), "{bib}");

    let library = hayagriva::io::from_biblatex_str(&bib).expect("BibLaTeX hayagriva reads");
    let lee = library.get("lee2018").unwrap();
    assert_eq!(
        lee.title().unwrap().to_string(),
        "Deep & Wide: 100% {models}"
    );
    assert_eq!(
        lee.url().unwrap().value.as_str(),
        "https://x.org/~lee?a=1&b=2"
    );
    assert_eq!(library.keys().count(), 2);
}

#[test]
fn dates_come_in_any_of_csl_forms() {
    assert_eq!(date(&json!(1995)).as_deref(), Some("1995"));
    assert_eq!(date(&json!("2020-03-15")).as_deref(), Some("2020-03-15"));
    assert_eq!(date(&json!("March 2020")).as_deref(), Some("2020"));
    assert_eq!(
        date(&json!({"date-parts": [[2020, 1], [2021]]})).as_deref(),
        Some("2020-01/2021")
    );
    assert_eq!(
        date(&json!([{"year": 2019, "month": 7}])).as_deref(),
        Some("2019-07")
    );
    assert_eq!(date(&json!({"raw": "2001"})).as_deref(), Some("2001"));
}
