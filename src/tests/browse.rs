use super::*;

/// A directory with talks in it, and other files besides.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for d in ["talks", "talks/old", "notes", ".git"] {
        std::fs::create_dir(root.join(d)).unwrap();
    }
    for f in [
        "alpha.pdf",
        "beta.PDF",
        "readme.txt",
        "talks/intro.pdf",
        "talks/old/first.pdf",
        ".hidden.pdf",
    ] {
        std::fs::write(root.join(f), b"").unwrap();
    }
    std::fs::write(root.join("slides.md"), b"").unwrap();
    dir
}

fn names(b: &Browser) -> Vec<&str> {
    b.entries.iter().map(|e| e.name.as_str()).collect()
}

fn picked(b: &Browser) -> &str {
    &b.picked().unwrap().name
}

fn press(b: &mut Browser, keys: &[KeyCode]) -> Outcome {
    let mut last = Outcome::Stay;
    for &code in keys {
        last = b.key(KeyEvent::from(code));
    }
    last
}

fn type_in(b: &mut Browser, text: &str) {
    for c in text.chars() {
        b.key(KeyEvent::from(KeyCode::Char(c)));
    }
}

fn opened(outcome: Outcome) -> PathBuf {
    match outcome {
        Outcome::Open(path) => path,
        _ => panic!("nothing opened"),
    }
}

#[test]
fn lists_folders_then_slides_without_hidden_or_others() {
    let dir = tree();
    let b = Browser::new(dir.path()).unwrap();
    let mut want = vec!["..", "notes", "talks", "alpha.pdf", "beta.PDF"];
    if cfg!(feature = "markdown") {
        want.push("slides.md");
    }
    assert_eq!(names(&b), want);
}

#[test]
fn typing_narrows_to_matches_best_first() {
    let dir = tree();
    let mut b = Browser::new(dir.path()).unwrap();
    type_in(&mut b, "ta");
    // "talks" starts with it, "beta.PDF" has it in it.
    assert_eq!(names(&b), ["talks", "beta.PDF"]);
    assert_eq!(picked(&b), "talks");
    type_in(&mut b, "x");
    assert!(b.entries.is_empty());
    press(&mut b, &[KeyCode::Backspace]);
    assert_eq!(names(&b), ["talks", "beta.PDF"]);
}

#[test]
fn letters_in_order_match_last() {
    assert_eq!(rank("alpha.pdf", "al"), Some(0));
    assert_eq!(rank("alpha.pdf", "pha"), Some(1));
    assert_eq!(rank("alpha.pdf", "apf"), Some(2));
    assert_eq!(rank("alpha.pdf", "fa"), None);
}

#[test]
fn a_leading_dot_shows_hidden_files() {
    let dir = tree();
    let mut b = Browser::new(dir.path()).unwrap();
    type_in(&mut b, ".h");
    assert_eq!(names(&b), [".hidden.pdf"]);
}

#[test]
fn up_and_down_move_within_the_list() {
    let dir = tree();
    let mut b = Browser::new(dir.path()).unwrap();
    press(&mut b, &[KeyCode::Down, KeyCode::Down]);
    assert_eq!(picked(&b), "talks");
    press(&mut b, &[KeyCode::Up, KeyCode::Up, KeyCode::Up]);
    assert_eq!(picked(&b), "..");
    b.rows.set(20);
    press(&mut b, &[KeyCode::PageDown]);
    assert_eq!(picked(&b), *names(&b).last().unwrap());
}

#[test]
fn enter_goes_into_a_folder_and_opens_a_file() {
    let dir = tree();
    let root = dir.path().canonicalize().unwrap();
    let mut b = Browser::new(&root).unwrap();
    type_in(&mut b, "talks");
    press(&mut b, &[KeyCode::Enter]);
    assert_eq!(b.dir, root.join("talks"));
    assert!(b.query.is_empty());
    type_in(&mut b, "int");
    let path = opened(press(&mut b, &[KeyCode::Enter]));
    assert_eq!(path, root.join("talks/intro.pdf"));
}

#[test]
fn going_up_picks_the_folder_left() {
    let dir = tree();
    let root = dir.path().canonicalize().unwrap();
    let mut b = Browser::new(&root.join("talks/old")).unwrap();
    press(&mut b, &[KeyCode::Backspace]);
    assert_eq!(b.dir, root.join("talks"));
    assert_eq!(picked(&b), "old");
    press(&mut b, &[KeyCode::Left]);
    assert_eq!(b.dir, root);
    assert_eq!(picked(&b), "talks");
    press(&mut b, &[KeyCode::Right]);
    assert_eq!(b.dir, root.join("talks"));
}

#[test]
fn a_typed_path_goes_to_its_directory() {
    let dir = tree();
    let root = dir.path().canonicalize().unwrap();
    let mut b = Browser::new(&root.join("notes")).unwrap();
    type_in(&mut b, "../talks/");
    assert_eq!(b.dir, root.join("talks"));
    assert!(b.query.is_empty());
    type_in(&mut b, "old/fi");
    assert_eq!(b.dir, root.join("talks/old"));
    assert_eq!(names(&b), ["first.pdf"]);
    let absolute = format!("{}/", root.display());
    type_in(&mut b, &absolute);
    assert_eq!(b.dir, root.join("talks/old"));
    press(&mut b, &[KeyCode::Esc]);
    type_in(&mut b, &absolute);
    assert_eq!(b.dir, root);
}

#[test]
fn a_path_to_nowhere_stays_typed() {
    let dir = tree();
    let root = dir.path().canonicalize().unwrap();
    let mut b = Browser::new(&root).unwrap();
    type_in(&mut b, "nowhere/");
    assert_eq!(b.dir, root);
    assert_eq!(b.query, "nowhere/");
    assert!(b.entries.is_empty());
    assert!(b.trouble().is_some());
    assert!(matches!(press(&mut b, &[KeyCode::Enter]), Outcome::Stay));
}

#[test]
fn a_file_typed_in_full_opens_unlisted() {
    let dir = tree();
    let root = dir.path().canonicalize().unwrap();
    let mut b = Browser::new(&root).unwrap();
    type_in(&mut b, "readme.txt");
    assert!(b.entries.is_empty());
    let path = opened(press(&mut b, &[KeyCode::Enter]));
    assert_eq!(path, root.join("readme.txt"));
}

#[test]
fn esc_clears_then_quits() {
    let dir = tree();
    let mut b = Browser::new(dir.path()).unwrap();
    type_in(&mut b, "al");
    assert!(matches!(press(&mut b, &[KeyCode::Esc]), Outcome::Stay));
    assert!(b.query.is_empty());
    assert!(matches!(press(&mut b, &[KeyCode::Esc]), Outcome::Quit));
}
