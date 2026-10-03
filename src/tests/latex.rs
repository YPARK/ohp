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
    let mut done: Done = (0..6)
        .map(|n| (formula(n), (n / 2, Kept::Failed)))
        .collect();
    evict(&mut done, 3);
    let mut left: Vec<_> = done.keys().map(|k| k.1.0.clone()).collect();
    left.sort();
    // Those asked for together, at 1, are kept or dropped together.
    assert_eq!(left, ["x_2", "x_3", "x_4", "x_5"]);
    evict(&mut done, 4);
    assert_eq!(done.len(), 4);
}

#[test]
fn formulas_asked_for_at_once_are_kept_however_many() {
    let formula = |n: u64| (String::new(), (format!("x_{n}"), false));
    let mut done: Done = (0..10)
        .map(|n| (formula(n), (u64::from(n > 0), Kept::Failed)))
        .collect();
    evict(&mut done, 3);
    assert_eq!(done.len(), 9, "all of the latest document");
}

#[test]
fn what_one_render_asks_for_is_stamped_alike() {
    if !available() {
        eprintln!("skipped: no pdflatex");
        return;
    }
    // Ones no other test asks for, rendered a batch each.
    let formulas: Vec<Formula> = (0..3)
        .map(|n| (format!("y_{{{}}}", 9100 + n), false))
        .collect();
    let preamble = "% stamped alike";
    let done = render_on(&formulas, preamble, &AtomicBool::new(false), 1);
    assert_eq!(done.len(), 3);
    // Read, then checked with the lock let go: a failure here must not
    // poison it for the other tests.
    let stamps: Vec<Option<u64>> = {
        let (cache, _) = &*RENDERED.lock().unwrap();
        formulas
            .iter()
            .map(|f| cache.get(&(preamble.to_string(), f.clone())).map(|e| e.0))
            .collect()
    };
    assert!(
        stamps.iter().all(|s| s.is_some() && *s == stamps[0]),
        "{stamps:?}"
    );
}

#[test]
fn a_stopped_render_renders_nothing_new() {
    let stop = AtomicBool::new(true);
    // One LaTeX would render, and no other test asks for.
    let formula = ("x + 9200".to_string(), false);
    assert!(render(std::slice::from_ref(&formula), "", &stop).is_empty());
    if available() {
        let go = AtomicBool::new(false);
        assert!(render(&[formula], "", &go).len() == 1, "renders once going");
    }
}

#[test]
fn a_run_that_never_ends_times_out_or_stops() {
    if !available() {
        eprintln!("skipped: no pdflatex");
        return;
    }
    let looping = "\\def\\a{\\a}\\a";
    let go = AtomicBool::new(false);
    let start = std::time::Instant::now();
    let ran = run(looping, false, "", &go, Duration::from_secs(1));
    assert!(matches!(ran, Some(Kept::TimedOut)));
    assert!(start.elapsed() < Duration::from_secs(5));

    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopping.store(true, Ordering::Relaxed);
    });
    let ran = run(looping, false, "", &stop, Duration::from_secs(60));
    assert!(ran.is_none());
}

#[test]
fn a_formula_timed_out_is_tried_once_more() {
    if !available() {
        eprintln!("skipped: no pdflatex");
        return;
    }
    // As if it had timed out once: it renders now, and is kept as set.
    let formula = ("z_{9300}".to_string(), false);
    let key = (String::new(), formula.clone());
    RENDERED
        .lock()
        .unwrap()
        .0
        .insert(key.clone(), (0, Kept::TimedOut));
    let done = render(std::slice::from_ref(&formula), "", &AtomicBool::new(false));
    assert!(done.contains_key(&formula));
    let kept = RENDERED.lock().unwrap().0.get(&key).map(|e| e.1.clone());
    assert!(matches!(kept, Some(Kept::Set(_))), "{kept:?}");
}
