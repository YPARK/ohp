use super::*;

#[test]
fn the_log_gives_height_and_depth() {
    let log = "(./f.aux)\nOHP:6.94444pt:2.5pt\n[1]\n";
    assert_eq!(measure(log), Some((6.94444, 2.5)));
    assert_eq!(measure("OHP:0.0pt:0.0pt\n"), None);
    assert_eq!(measure("no measure\n"), None);
}

#[test]
fn latex_renders_what_it_knows_and_keeps_it() {
    if !available() {
        eprintln!("skipped: no pdflatex");
        return;
    }
    let preamble = "\\newcommand{\\R}{\\mathbb{R}}";
    let good = ("\\R^n_{\\tiny ok}".to_string(), false);
    let tall = ("\\sum_{i=1}^n x_i".to_string(), true);
    let bad = ("\\undefinedmacro".to_string(), false);
    let done = render(&[good.clone(), tall.clone(), bad.clone()], preamble);
    assert!(!done.contains_key(&bad));
    let (g, t) = (&done[&good], &done[&tall]);
    assert!(g.pdf.starts_with(b"%PDF"));
    assert!(g.height > 0.5 && g.height < 1.2, "{g:?}");
    assert!(t.height + t.depth > g.height + g.depth, "{t:?} {g:?}");

    let again = render(std::slice::from_ref(&good), preamble);
    assert!(Arc::ptr_eq(&again[&good].pdf, &g.pdf));
}
