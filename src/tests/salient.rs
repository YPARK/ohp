use super::*;

fn pages(pages: &[&[&str]]) -> Vec<Vec<String>> {
    (pages.iter())
        .map(|said| said.iter().map(|s| s.to_string()).collect())
        .collect()
}

#[test]
fn the_sentences_most_like_the_document_are_picked_in_order() {
    let pages = pages(&[
        &[
            "Single cell sequencing measures gene expression in each cell.",
            "The weather was pleasant on the day of the conference.",
        ],
        &[
            "Gene expression in each cell tells cell types apart.",
            "Lunch was served in the main hall at noon.",
            "Cell types differ in the expression of marker genes.",
        ],
    ]);
    let picked = pick(&pages, 3 * (60 + NUMBERING));
    assert_eq!(picked, vec![(0, 0), (1, 0), (1, 2)]);
}

#[test]
fn headings_and_references_are_not_picked() {
    let pages = pages(&[&[
        "Introduction",
        "Cells were sorted by the expression of their marker genes today.",
        "Smith J, Lee K. Sorting cells. Nature. 2019;5:1-9. Cited in 2021 too.",
        "Lee K. Sorted cells by marker genes, doi:10.1000/xyz.",
    ]]);
    assert_eq!(pick(&pages, 10_000), vec![(0, 1)]);
}

#[test]
fn findings_come_before_details_pointing_to_figures() {
    let pages = pages(&[
        &["A title slide of a talk about sorting cells by their genes."],
        &[
            "Cells were sorted by marker genes, as Figure 2 shows for each sample.",
            "Here we show that marker genes sort cells into their types.",
        ],
    ]);
    let one = 70 + NUMBERING;
    assert_eq!(pick(&pages, one), vec![(1, 1)]);
}

#[test]
fn pieces_of_sentences_are_not_picked() {
    let pages = pages(&[&[
        "by inosine, guanosine and adenosine or by cyclic AMP alone.",
        "Inosine activates the kinase at nanomolar concentrations in cells.",
    ]]);
    assert_eq!(pick(&pages, 10_000), vec![(0, 1)]);
}

#[test]
fn running_heads_are_not_picked() {
    let foot = |n: usize| format!("Ober, Githure, Volpato Santos et al. eLife 2023;12:RP91040 {n}");
    let findings = [
        "Inosine activates the kinase at nanomolar concentrations in cells.",
        "Guanosine binds the second site of the kinase with high affinity.",
        "Adenosine binds the first site of the kinase and activates it less.",
    ];
    let pages: Vec<Vec<String>> = (findings.iter().zip(1..))
        .map(|(finding, n)| vec![finding.to_string(), foot(n)])
        .collect();
    let picked = pick(&pages, 10_000);
    assert!(picked.iter().all(|&(_, k)| k == 0), "{picked:?}");
}

#[test]
fn a_sentence_said_again_is_picked_once() {
    let same = "Marker genes tell the sorted cell types apart in every sample.";
    let pages = pages(&[&[same], &[same]]);
    assert_eq!(pick(&pages, 10_000), vec![(0, 0)]);
}

#[test]
fn as_many_are_picked_as_the_budget_holds() {
    let said: Vec<String> = (0..50)
        .map(|i| format!("Cells of sample {i} were sorted by marker genes."))
        .collect();
    let picked = pick(std::slice::from_ref(&said), 5 * (said[0].len() + NUMBERING));
    assert!(picked.len() <= 5 && !picked.is_empty(), "{picked:?}");
}

#[test]
fn sentences_are_weighed_by_what_they_mean_where_that_is_known() {
    let pages = pages(&[
        &["Cells were sorted by the genes they express in the sample."],
        &[
            "Cells were grouped by their gene expression in the sample.",
            "The dinner of the conference was held beside the river.",
            "Sorting cells by expression tells their types apart well.",
        ],
    ]);
    let candidates = candidates(&pages);
    assert_eq!(candidates.len(), 4);
    let unit = |v: [f32; 2]| {
        let n = (v[0] * v[0] + v[1] * v[1]).sqrt();
        vec![v[0] / n, v[1] / n]
    };
    // The first two mean the same; the dinner is far from the rest.
    let meanings = Vectors::Meanings(vec![
        unit([1., 0.1]),
        unit([1., 0.12]),
        unit([0., 1.]),
        unit([0.7, 0.7]),
    ]);
    let two = 2 * (60 + NUMBERING) + 10;
    let picked = pick_from(&pages, &candidates, &meanings, two);
    assert_eq!(picked, vec![(0, 0), (1, 2)]);
}
