use super::*;
use crate::fixture;
use std::path::Path;
use std::time::{Duration, Instant};

fn deck(texts: &[&str]) -> Arc<Vec<u8>> {
    Arc::new(fixture::saying(texts))
}

/// A speaker whose voice says the text as it is, and whose player writes
/// what it is given to `out`, a sentence a line, after any before it.
fn recorder(out: &Path) -> Speaker {
    Speaker::new(
        Some("cat".into()),
        Some(format!(
            "cat >> {}",
            remote::quote(&out.display().to_string())
        )),
    )
}

/// Wait for the reading under way to end.
fn ended(speaker: &mut Speaker) -> Result<usize, String> {
    let start = Instant::now();
    loop {
        if let Some(ended) = speaker.ended() {
            return ended;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "reading hung");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_command_given_is_used_and_a_blank_one_is_not() {
    let found = || Some("found".to_string());
    assert_eq!(
        given_or(Some("piper".into()), found).as_deref(),
        Some("piper")
    );
    assert_eq!(given_or(Some("  ".into()), found).as_deref(), Some("found"));
    assert_eq!(given_or(None, found).as_deref(), Some("found"));
}

#[test]
fn the_slide_is_voiced_and_played_a_sentence_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let mut speaker = recorder(&out);
    speaker
        .read(deck(&["one", "Hello there. It is me"]), 1)
        .unwrap();
    assert_eq!(speaker.page(), Some(1));
    assert_eq!(ended(&mut speaker), Ok(1));
    assert_eq!(speaker.page(), None);
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "Hello there.\nIt is me\n"
    );
}

#[test]
fn the_sentence_playing_is_known() {
    let dir = tempfile::tempdir().unwrap();
    let flag = dir.path().join("go");
    // The player waits for the test to let it end.
    let player = format!(
        "cat > /dev/null; while [ ! -e '{}' ]; do sleep 0.01; done; rm '{0}'",
        flag.display()
    );
    let mut speaker = Speaker::new(Some("cat".into()), Some(player));
    speaker.read(deck(&["First one. Second one."]), 0).unwrap();
    for k in 0..2 {
        let start = Instant::now();
        while speaker.sentence() != Some(k) {
            assert!(start.elapsed() < Duration::from_secs(10), "no sentence {k}");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(&flag, "").unwrap();
    }
    assert_eq!(ended(&mut speaker), Ok(0));
    assert_eq!(speaker.sentence(), None);
}

#[test]
fn a_slide_with_no_text_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("read.txt");
    let mut speaker = recorder(&out);
    speaker.read(Arc::new(fixture::pdf(1)), 0).unwrap();
    assert_eq!(ended(&mut speaker), Ok(0));
    assert!(!out.exists());
}

#[test]
fn a_failing_voice_or_player_says_why() {
    let fail = || Some("echo 'no voice here' >&2; exit 3".to_string());
    for mut speaker in [
        Speaker::new(fail(), Some("cat > /dev/null".into())),
        Speaker::new(Some("cat".into()), fail()),
    ] {
        speaker.read(deck(&["Hello"]), 0).unwrap();
        let why = ended(&mut speaker).unwrap_err();
        assert!(why.contains("no voice here"), "{why}");
    }
}

#[test]
fn nothing_to_read_with_is_an_error() {
    for mut speaker in [
        Speaker::new(None, Some("cat".into())),
        Speaker::new(Some("cat".into()), None),
    ] {
        assert!(speaker.read(deck(&["Hello"]), 0).is_err());
        assert_eq!(speaker.page(), None);
    }
}

#[test]
fn stopping_ends_the_whole_pipeline() {
    let sleep = fixture::marked_sleep(30);
    let mut speaker = Speaker::new(Some("cat".into()), Some(format!("cat | {sleep} | cat")));
    speaker.read(deck(&["Hello"]), 0).unwrap();
    assert!(fixture::started(&sleep), "the pipeline never started");
    let start = Instant::now();
    speaker.stop();
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(speaker.page(), None);
    assert!(fixture::gone(&sleep), "the pipeline outlived the stop");
}

#[cfg(feature = "markdown")]
#[test]
fn formulas_tables_and_toml_front_matter_are_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let md = fixture::write_text(
        dir.path(),
        "notes.md",
        "+++\ntitle = \"Notes\"\ndraft = true\n+++\n\
         Before the formula $x^2 + y$ and after it.\n\n\
         $$\\sum_i \\beta_i z_i$$\n\n\
         | Gene | Score |\n|---|---|\n| ABC | 1.5 |\n\n\
         The end.\n",
    );
    let deck = crate::render::Deck::open(&md).unwrap();
    let said = words(&deck.data, 0).unwrap();
    let text = |s: &str| Spoken::Text(s.into());
    assert_eq!(
        said,
        [
            text("Notes"),
            text("Before the formula"),
            Spoken::Pause,
            text("and after it."),
            Spoken::Pause,
            text("The end."),
        ]
    );
}

#[test]
fn a_pause_lasts_as_long_as_asked_unless_stopped() {
    let start = Instant::now();
    wait(Duration::from_millis(150), &AtomicBool::new(false));
    assert!(start.elapsed() >= Duration::from_millis(150));
    let start = Instant::now();
    wait(Duration::from_secs(30), &AtomicBool::new(true));
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn piper_reads_with_the_first_voice_it_has() {
    let dir = tempfile::tempdir().unwrap();
    let (bin, voices) = (dir.path().join("bin"), dir.path().join("it's voices"));
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&voices).unwrap();
    let path = bin.clone().into_os_string();
    let touch = |name: &str| std::fs::write(voices.join(name), "").unwrap();

    touch("en_US-ryan-high.onnx");
    touch("en_US-ryan-high.onnx.json");
    assert_eq!(piper(&path, &voices), None, "no piper on the path");

    std::fs::write(bin.join("piper"), "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(piper(&path, &voices), None, "a piper not allowed to run");
        let runs = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(bin.join("piper"), runs).unwrap();
    }
    touch("en_GB-alan-low.onnx");
    touch("de_DE-thorsten.onnx.json");
    let model = voices.join("en_US-ryan-high.onnx").display().to_string();
    assert_eq!(
        piper(&path, &voices),
        Some(format!("piper -m {} -f -", remote::quote(&model)))
    );
    assert_eq!(piper(&path, &dir.path().join("none")), None);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_says_a_sentence_as_wav() {
    let dir = tempfile::tempdir().unwrap();
    let (text, wav) = (dir.path().join("s.txt"), dir.path().join("s.wav"));
    std::fs::write(&text, "Hello there.").unwrap();
    run(SAY, &text, Some(&wav), &AtomicBool::new(false)).unwrap();
    let wav = std::fs::read(&wav).unwrap();
    assert!(wav.len() > 1000, "{} bytes", wav.len());
    assert_eq!((&wav[..4], &wav[8..12]), (&b"RIFF"[..], &b"WAVE"[..]));
}
