use super::*;
use crate::fixture;
use std::time::{Duration, Instant};

/// `texts`, as sentences on page `page`.
fn on(page: usize, texts: &[&'static str]) -> Vec<(usize, &'static str)> {
    texts.iter().map(|t| (page, *t)).collect()
}

#[test]
fn a_page_is_numbered_from_one() {
    let prompt = prompt(&on(0, &["Cells differ.", "We measured them."]), false);
    assert!(prompt.contains("[1] Cells differ.\n[2] We measured them.\n"));
    assert!(prompt.contains("KEY:"));
}

#[test]
fn the_documents_sentences_say_their_pages() {
    let sentences = vec![(0, "Cells differ."), (6, "We measured them.")];
    let prompt = prompt(&sentences, true);
    assert!(prompt.contains("[1] (page 1) Cells differ.\n[2] (page 7) We measured them.\n"));
}

#[test]
fn a_long_page_is_cut_short() {
    let long: &'static str = "word ".repeat(100).leak();
    let sentences = vec![(0, long); 100];
    let prompt = prompt(&sentences, false);
    assert!(prompt.len() < TEXT + 400, "{}", prompt.len());
    assert!(prompt.contains("[1] "));
    assert!(!prompt.contains("[100] "));
}

fn key(page: usize, sentence: usize, text: &str) -> Key {
    Key {
        page,
        sentence,
        text: text.into(),
    }
}

#[test]
fn the_answer_gives_the_summary_and_its_key_sentences() {
    let sentences = on(
        4,
        &[
            "It says one thing.",
            "It says two things.",
            "It says three things.",
            "It says four things.",
        ],
    );
    let summary = summary("SUMMARY: It counts.\nKEY: 3, 1", &sentences, &|k| k, 3);
    assert_eq!(summary.text, "It counts.");
    assert_eq!(
        summary.key,
        vec![
            key(4, 0, "It says one thing."),
            key(4, 2, "It says three things.")
        ]
    );
    assert_eq!(summary.picked(4), Picked::of([0, 2]));
    assert!(summary.picked(3).is_empty());
}

#[test]
fn key_sentences_are_read_however_the_model_writes_them() {
    let sentences = on(
        0,
        &[
            "It says one thing.",
            "It says two things.",
            "It says three things.",
            "It says four things.",
            "It says five things.",
        ],
    );
    let answer = "Summary: It counts.\nKey: [2], [9], 2, 4, 5, 1";
    let summary = summary(answer, &sentences, &|k| k, 3);
    assert_eq!(summary.text, "It counts.");
    // None past the page, none twice, and no more than asked for.
    let key: Vec<usize> = summary.key.iter().map(|k| k.sentence).collect();
    assert_eq!(key, vec![1, 3, 4]);
}

#[test]
fn the_documents_key_sentences_are_where_they_were_picked_from() {
    let sentences = vec![
        (0, "It says one thing."),
        (2, "It says two things."),
        (2, "It says three things."),
    ];
    let picked = [(0, 5), (2, 1), (2, 7)];
    let summary = summary(
        "SUMMARY: It counts.\nKEY: 3",
        &sentences,
        &|i| picked[i].1,
        5,
    );
    assert_eq!(summary.key, vec![key(2, 7, "It says three things.")]);
    assert_eq!(summary.picked(2), Picked::of([7]));
}

#[test]
fn an_answer_out_of_form_is_all_summary() {
    let summary = summary(
        "The page counts to three.",
        &on(0, &["It says one thing."]),
        &|k| k,
        3,
    );
    assert_eq!(summary.text, "The page counts to three.");
    assert!(summary.key.is_empty());
}

#[test]
fn the_summary_is_shown_as_it_comes() {
    assert_eq!(shown("SUMMARY: It co"), "It co");
    assert_eq!(shown("SUMMARY: It counts.\nKE"), "It counts.\nKE");
    assert_eq!(shown("SUMMARY: It counts.\nKEY: 1"), "It counts.");
}

#[test]
fn a_summary_kept_for_one_page_is_put_on_another() {
    let summary = Summary {
        text: "It counts.".into(),
        key: vec![key(1, 2, "It says three things.")],
    };
    assert_eq!(summary.on(5).key, vec![key(5, 2, "It says three things.")]);
}

#[test]
fn models_are_suggested_by_name_or_given_as_files_or_repos() {
    let qwen = Weights::Hub {
        repo: "Qwen/Qwen2.5-3B-Instruct-GGUF".into(),
        file: Some("qwen2.5-3b-instruct-q4_k_m.gguf".into()),
    };
    assert_eq!(weights("qwen2.5-3b"), Ok(qwen.clone()));
    assert_eq!(qwen.name(), "qwen2.5-3b");
    assert_eq!(weights("Qwen2.5-1.5B"), Ok(Weights::from(&SUGGESTED[1])));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mine.gguf");
    std::fs::write(&file, b"").unwrap();
    assert_eq!(
        weights(file.to_str().unwrap()),
        Ok(Weights::File(file.clone()))
    );
    assert_eq!(Weights::File(file).name(), "mine.gguf");
    let repo = |repo: &str, file: Option<&str>| Weights::Hub {
        repo: repo.into(),
        file: file.map(String::from),
    };
    assert_eq!(
        weights("bartowski/Phi-GGUF"),
        Ok(repo("bartowski/Phi-GGUF", None))
    );
    assert_eq!(
        repo("bartowski/Phi-GGUF", None).name(),
        "bartowski/Phi-GGUF"
    );
    assert_eq!(
        weights("https://huggingface.co/bartowski/Phi-GGUF/blob/main/Phi-Q4_K_M.gguf"),
        Ok(repo("bartowski/Phi-GGUF", Some("Phi-Q4_K_M.gguf")))
    );
    assert_eq!(
        weights("hf.co/a/b/sub/c.gguf"),
        Ok(repo("a/b", Some("sub/c.gguf")))
    );
    assert!(weights("gpt-9").is_err());
    assert!(weights("a/b/readme.md").is_err());
    assert!(weights("../model").is_err());
}

#[test]
fn the_gguf_picked_is_whole_and_quantised_as_liked() {
    let files = [
        "README.md",
        "m-f16.gguf",
        "m-Q8_0.gguf",
        "m-Q4_K_M-00001-of-00002.gguf",
        "m-Q4_K_S.gguf",
        "mmproj-m-Q4_K_M.gguf",
    ];
    assert_eq!(gguf(files), Some("m-Q4_K_S.gguf"));
    assert_eq!(gguf(["m-f16.gguf", "m-bf16.gguf"]), Some("m-f16.gguf"));
    assert_eq!(gguf(["README.md"]), None);
}

#[test]
fn how_far_a_summary_has_come_is_shown() {
    let mut state = state(2);
    let page = Task::Page(1);
    assert_eq!(state.progress(page).what, "waiting…");
    state.model = Weighed::Loading("qwen2.5-3b".into());
    assert_eq!(
        state.progress(page),
        Progress::unknown("loading qwen2.5-3b…".into())
    );
    state.model = Weighed::Loaded;
    state.doing = Some(Doing::Reading(page, 256, 1024));
    assert_eq!(state.progress(page).done, Some(0.25));
    assert_eq!(state.progress(Task::Page(0)).what, "waiting…");
    state.doing = Some(Doing::Writing(
        page,
        "It counts.".into(),
        60,
        Instant::now(),
    ));
    let view = state.view(page, None, "");
    assert_eq!(view.text, "It counts.");
    assert_eq!(view.busy.unwrap().done, Some(60. / PAGE_ANSWER as f32));
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("blob.incomplete");
    std::fs::write(&part, vec![0; 250]).unwrap();
    state.fetching = Some(Fetching {
        what: "qwen2.5-3b".into(),
        part,
        size: 1000,
    });
    let fetching = state.progress(page);
    assert_eq!(fetching.done, Some(0.25));
    assert!(
        fetching.what.starts_with("downloading qwen2.5-3b"),
        "{}",
        fetching.what
    );
    state.fetching = None;
    state.model = Weighed::Broken("no model".into());
    assert_eq!(state.view(page, None, ""), View::says("no model"));
}

#[test]
fn another_model_sums_up_all_again() {
    let qwen3b = Weights::from(&SUGGESTED[0]);
    let mut summarizer = Summarizer::new(Some(qwen3b.clone()));
    {
        let mut state = lock(&summarizer.shared);
        state.pages.insert(0, Ok(Arc::default()));
        state
            .known
            .insert(vec!["It says one thing.".into()], Arc::default());
    }
    summarizer.set_weights(qwen3b);
    assert_eq!(lock(&summarizer.shared).pages.len(), 1, "the same model");
    let round = lock(&summarizer.shared).round;
    summarizer.set_weights(Weights::from(&SUGGESTED[1]));
    let state = lock(&summarizer.shared);
    assert!(state.pages.is_empty() && state.known.is_empty());
    assert_eq!(state.round, round + 1);
    assert_eq!(state.weights, Weights::from(&SUGGESTED[1]));
}

fn state(pages: usize) -> State {
    State {
        deck: Some((Arc::new(Vec::new()), pages)),
        ..State::default()
    }
}

#[test]
fn the_page_on_screen_is_summed_up_before_the_whole() {
    let mut state = state(3);
    assert_eq!(state.next(), None);
    state.wanted = Some(2);
    state.whole_wanted = true;
    assert_eq!(state.next(), Some(Task::Page(2)));
    state.pages.insert(2, Err("unreadable".into()));
    assert_eq!(state.next(), Some(Task::Whole));
    state.whole = Some(Ok(Arc::default()));
    assert_eq!(state.next(), None);
    state.wanted = Some(5);
    assert_eq!(state.next(), None, "past the last page");
}

#[test]
fn without_a_model_only_what_is_shown_is_looked_for() {
    let mut state = state(3);
    state.wanted = Some(0);
    state.model = Weighed::Broken("no model".into());
    assert_eq!(state.next(), Some(Task::Page(0)), "it may be kept");
    state.pages.insert(0, Err("no model".into()));
    assert_eq!(state.next(), None, "none summed up ahead");
    state.whole_wanted = true;
    assert_eq!(state.next(), Some(Task::Whole));
}

#[test]
fn a_summary_kept_is_shown_though_the_model_cannot_be_had() {
    let pdf = fixture::with(&[
        fixture::text(40., 150., 12., "Cells differ from one another."),
        fixture::text(40., 150., 12., "Genes tell the cells apart."),
    ]);
    let said = text::of(Arc::new(pdf.clone()), 1).unwrap().said();
    let mut summarizer = unloadable();
    let kept = Summary {
        text: "Genes tell them apart.".into(),
        key: Vec::new(),
    };
    lock(&summarizer.shared).known.insert(said, Arc::new(kept));
    summarizer.set_deck(Arc::new(pdf), 2);
    summarizer.want(Some(0));
    until(|| !summarizer.page(0).text.is_empty());
    assert!(summarizer.page(0).text.contains("/no/such/model.gguf"));
    summarizer.want(Some(1));
    until(|| summarizer.page(1).busy.is_none() && !summarizer.page(1).text.is_empty());
    assert_eq!(summarizer.page(1).text, "Genes tell them apart.");
}

/// Wait until `done`, for ten seconds at most.
fn until(done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn unloadable() -> Summarizer {
    Summarizer::new(Some(Weights::File("/no/such/model.gguf".into())))
}

#[test]
fn a_page_with_no_text_is_summed_up_without_the_model() {
    let mut summarizer = unloadable();
    summarizer.set_deck(Arc::new(fixture::pdf(2)), 2);
    summarizer.want(Some(1));
    until(|| summarizer.page(1).busy.is_none());
    assert_eq!(summarizer.page(1).text, "Nothing on this page to sum up.");
    assert!(summarizer.picked(1).is_empty());
}

#[test]
fn a_model_that_cannot_be_loaded_says_why() {
    let mut summarizer = unloadable();
    let pdf = fixture::hello();
    summarizer.set_deck(Arc::new(pdf), 1);
    summarizer.want(Some(0));
    until(|| !summarizer.page(0).text.is_empty());
    let view = summarizer.page(0);
    assert!(view.text.contains("/no/such/model.gguf"), "{view:?}");
}

#[test]
fn a_file_with_no_text_is_summed_up_without_the_model() {
    let mut summarizer = unloadable();
    summarizer.set_deck(Arc::new(fixture::pdf(3)), 3);
    summarizer.want_whole(true);
    until(|| summarizer.whole().busy.is_none());
    assert_eq!(summarizer.whole().text, "Nothing in this file to sum up.");
}

/// A model downloaded and a PDF, as `OHP_TEST_MODEL` and `OHP_TEST_PDF`
/// name them: too large to be fetched for a test.
#[test]
#[ignore = "needs a model downloaded, and a PDF, named by OHP_TEST_MODEL and OHP_TEST_PDF"]
fn sums_up_a_real_page_and_the_whole() {
    let model = std::env::var("OHP_TEST_MODEL").expect("OHP_TEST_MODEL names a GGUF file");
    let pdf = std::env::var("OHP_TEST_PDF").expect("OHP_TEST_PDF names a PDF");
    let data = Arc::new(std::fs::read(pdf).expect("PDF read"));
    let pages = crate::render::sizes(&Pdf::new(data.clone()).unwrap()).len();
    let mut summarizer = Summarizer::new(Some(weights(&model).unwrap()));
    summarizer.set_deck(data, pages);
    let start = Instant::now();
    summarizer.want(Some(0));
    while summarizer.page(0).busy.is_some() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let view = summarizer.page(0);
    eprintln!(
        "page 1 in {:?}:\n{}\nkey: {:#?}",
        start.elapsed(),
        view.text,
        view.key
    );
    assert!(!view.text.is_empty());
    let start = Instant::now();
    while summarizer.page(1).busy.is_some() {
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("page 2, summed up ahead, in {:?}", start.elapsed());
    summarizer.want(Some(1));
    assert!(summarizer.page(1).busy.is_none(), "ready when turned to");
    let start = Instant::now();
    summarizer.want(None);
    summarizer.want_whole(true);
    let mut phase = String::new();
    while let Some(busy) = summarizer.whole().busy {
        let now = busy
            .what
            .split(" · ")
            .next()
            .unwrap_or_default()
            .to_string();
        if now != phase {
            eprintln!("{:>6.2}s {now}", start.elapsed().as_secs_f32());
            phase = now;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let view = summarizer.whole();
    eprintln!(
        "whole in {:?}:\n{}\nkey: {:#?}",
        start.elapsed(),
        view.text,
        view.key
    );
    assert!(!view.text.is_empty());
}

#[test]
fn a_piece_of_a_sentence_is_never_a_key_one() {
    let sentences = on(0, &["vector.", "The rep gene was mapped codon by codon."]);
    let summary = summary("SUMMARY: Mapped.\nKEY: 1, 2", &sentences, &|k| k, 3);
    let key: Vec<usize> = summary.key.iter().map(|k| k.sentence).collect();
    assert_eq!(key, vec![1]);
}

#[test]
fn the_pages_after_the_one_on_screen_are_summed_up_after_it() {
    let mut state = state(4);
    state.wanted = Some(1);
    assert_eq!(state.next(), Some(Task::Page(1)));
    state.pages.insert(1, Ok(Arc::default()));
    assert_eq!(state.next(), Some(Task::Page(2)));
    state.whole_wanted = true;
    assert_eq!(state.next(), Some(Task::Whole), "the file before them");
    state.whole = Some(Ok(Arc::default()));
    state.pages.insert(2, Ok(Arc::default()));
    assert_eq!(state.next(), Some(Task::Page(3)));
    state.pages.insert(3, Ok(Arc::default()));
    assert_eq!(state.next(), None, "none past the last");
    state.wanted = None;
    state.pages.clear();
    state.whole = Some(Ok(Arc::default()));
    assert_eq!(state.next(), None, "none unless one is on screen");
}

#[test]
fn what_is_no_longer_wanted_is_told_to_give_up() {
    let mut state = state(9);
    let abandon = Arc::new(AtomicBool::new(false));
    let given_up = || abandon.load(Ordering::Relaxed);
    state.wanted = Some(3);
    state.pages.insert(3, Ok(Arc::default()));
    state.running = Some(Running {
        task: Task::Page(4),
        round: state.round,
        abandon: abandon.clone(),
    });
    state.recheck();
    assert!(!given_up(), "summed up ahead");
    state.wanted = Some(4);
    state.recheck();
    assert!(!given_up(), "turned to, it goes on");
    state.wanted = Some(7);
    state.recheck();
    assert!(given_up(), "turned past");
    abandon.store(false, Ordering::Relaxed);
    state.wanted = Some(4);
    state.anew();
    state.recheck();
    assert!(given_up(), "another model, or deck");
}

#[test]
fn summaries_are_kept_by_model_and_text() {
    let said = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (ab, a, b) = (said(&["A.", "B."]), said(&["A."]), said(&["B."]));
    let name = |weights: &Weights, what, pages: &[&Vec<String>]| {
        kept_name(weights, what, pages.iter().map(|p| p.as_slice()))
    };
    let qwen = Weights::from(&SUGGESTED[0]);
    let once = name(&qwen, "page", &[&ab]);
    assert_eq!(once, name(&qwen, "page", &[&ab]));
    assert!(once.ends_with(".json") && once.len() == 21);
    assert_ne!(once, name(&Weights::from(&SUGGESTED[1]), "page", &[&ab]));
    assert_ne!(once, name(&qwen, "whole", &[&ab]));
    assert_ne!(once, name(&qwen, "page", &[&a, &b]), "pages apart");
}

#[cfg(feature = "summary")]
#[test]
fn a_summary_kept_is_read_back_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let summary = Summary {
        text: "It counts.\nTo three.".into(),
        key: vec![key(2, 5, "It says \"three\" things.")],
    };
    assert_eq!(recall(dir.path(), "x.json"), None);
    keep(&dir.path().join("made"), "x.json", &summary);
    assert_eq!(recall(&dir.path().join("made"), "x.json"), Some(summary));
    std::fs::write(dir.path().join("bad.json"), "{\"text\": 3}").unwrap();
    assert_eq!(recall(dir.path(), "bad.json"), None);
}
