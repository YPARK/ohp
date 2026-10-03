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
    let go = AtomicBool::new(false);
    let done = render(&[good.clone(), tall.clone(), bad.clone()], preamble, &go);
    assert!(!done.contains_key(&bad));
    let (g, t) = (&done[&good], &done[&tall]);
    assert!(g.pdf.starts_with(b"%PDF"));
    assert!(g.height > 0.5 && g.height < 1.2, "{g:?}");
    assert!(t.height + t.depth > g.height + g.depth, "{t:?} {g:?}");

    let again = render(std::slice::from_ref(&good), preamble, &go);
    assert!(Arc::ptr_eq(&again[&good].pdf, &g.pdf));
}

#[test]
fn only_the_formulas_asked_for_most_recently_are_kept() {
    let formula = |n: u64| (String::new(), (format!("x_{n}"), false));
    let mut done: Done = (0..6).map(|n| (formula(n), (n / 2, None))).collect();
    evict(&mut done, 3);
    let mut left: Vec<_> = done.keys().map(|k| k.1.0.clone()).collect();
    left.sort();
    // Those asked for together, at 1, are kept or dropped together.
    assert_eq!(left, ["x_2", "x_3", "x_4", "x_5"]);
    evict(&mut done, 4);
    assert_eq!(done.len(), 4);
}

#[test]
fn a_stopped_render_renders_nothing_new() {
    let stop = AtomicBool::new(true);
    // One LaTeX would render, and no other test asks for.
    let formula = ("x + 1 % stopped".to_string(), false);
    assert!(render(&[formula], "", &stop).is_empty());
}
