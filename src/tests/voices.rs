use super::*;
use crate::remote;
use ratatui::crossterm::event::KeyEvent;

/// A catalog of Piper's voices, as `voices.json`, of `voices`: each its
/// name, language family and English name, and its files' sizes.
fn catalog_of(voices: &[(&str, &str, &str, u64)]) -> String {
    let entries: Vec<String> = voices
        .iter()
        .map(|(key, family, language, size)| {
            let dir = format!("{family}/{key}");
            format!(
                r#""{key}": {{
                    "key": "{key}",
                    "language": {{"family": "{family}", "name_english": "{language}",
                                  "country_english": "Somewhere"}},
                    "quality": "medium",
                    "files": {{
                        "{dir}/{key}.onnx": {{"size_bytes": {size}}},
                        "{dir}/{key}.onnx.json": {{"size_bytes": 2}},
                        "{dir}/MODEL_CARD": {{"size_bytes": 1}}
                    }}
                }}"#
            )
        })
        .collect();
    format!("{{{}}}", entries.join(","))
}

fn names(voices: &[Voice]) -> Vec<&str> {
    voices.iter().map(|v| v.name.as_str()).collect()
}

#[test]
fn the_catalog_lists_the_voices_not_kept_the_users_language_first() {
    let json = catalog_of(&[
        ("de_DE-thorsten-medium", "de", "German", 3_000_000),
        ("en_US-ryan-high", "en", "English", 1),
        ("en_GB-alba-medium", "en", "English", 1),
        ("en_US-amy-medium", "en", "English", 1),
    ]);
    let kept = HashSet::from(["en_US-amy-medium".to_string()]);
    let voices = catalog(&json, &kept, "en_CA.UTF-8").unwrap();
    assert_eq!(
        names(&voices),
        [
            "en_GB-alba-medium",
            "en_US-ryan-high",
            "de_DE-thorsten-medium"
        ]
    );
    assert_eq!(voices[2].about, "German (Somewhere) · medium · 4 MB");
    let How::Fetch(remote) = &voices[2].how else {
        panic!("not to download")
    };
    // The model card is not downloaded.
    assert_eq!(remote.files.len(), 2);
    assert!(catalog("not json", &kept, "en").is_err());
}

#[test]
fn the_voices_at_hand_are_the_one_given_piper_s_and_others() {
    let dir = tempfile::tempdir().unwrap();
    let touch = |name: &str| std::fs::write(dir.path().join(name), "").unwrap();
    touch("en_US-amy-medium.onnx");
    touch("en_US-amy-medium.onnx.json");
    let amy = speak::piper_with(&dir.path().join("en_US-amy-medium.onnx")).unwrap();
    let others = [("espeak-ng", "espeak-ng --stdout")];

    let voices = at_hand(Some("my-voice"), Some(dir.path()), true, &others);
    assert_eq!(names(&voices), ["given", "en_US-amy-medium", "espeak-ng"]);
    assert_eq!(voices[1].how, How::Ready(amy.clone()));

    // Given one of those listed anyway, it is not listed twice.
    let voices = at_hand(Some(&amy), Some(dir.path()), true, &others);
    assert_eq!(names(&voices), ["en_US-amy-medium", "espeak-ng"]);
    // Without Piper, its voices are not listed.
    let voices = at_hand(None, Some(dir.path()), false, &others);
    assert_eq!(names(&voices), ["espeak-ng"]);
}

fn ready(name: &str) -> Voice {
    Voice {
        name: name.into(),
        about: "installed".into(),
        how: How::Ready(format!("{name} --stdout")),
    }
}

fn press(menu: &mut Menu, keys: &[KeyCode]) -> Outcome {
    let mut last = Outcome::Stay;
    for &code in keys {
        last = menu.key(KeyEvent::from(code));
    }
    last
}

#[test]
fn the_menu_starts_on_the_voice_in_use_and_typing_narrows_it() {
    let voices = vec![ready("espeak-ng"), ready("espeak"), ready("say")];
    let mut menu = Menu::new(voices, Some("espeak --stdout"), Err("no Piper".into()));
    assert_eq!(menu.picked().map(|v| v.name.as_str()), Some("espeak"));

    // The voice the typing starts first, then those with its letters.
    press(&mut menu, &[KeyCode::Char('s'), KeyCode::Char('a')]);
    assert_eq!(menu.shown, [2, 0, 1]);
    let Outcome::Pick(voice) = press(&mut menu, &[KeyCode::Enter]) else {
        panic!("nothing picked")
    };
    assert_eq!(voice.name, "say");

    // Esc clears what is typed, then closes the list.
    press(&mut menu, &[KeyCode::Char('z'), KeyCode::Char('z')]);
    assert!(menu.picked().is_none());
    assert!(matches!(press(&mut menu, &[KeyCode::Esc]), Outcome::Stay));
    assert_eq!(menu.shown.len(), 3);
    assert!(matches!(press(&mut menu, &[KeyCode::Esc]), Outcome::Quit));
}

fn curl() -> bool {
    speak::installed("curl")
}

/// A catalog's root, as a `file://` URL to `dir`, holding `voices`'s
/// files, each the size the catalog says.
fn hosted(dir: &Path, voices: &[(&str, &str, &str, u64)]) -> String {
    for (key, family, _, size) in voices {
        let at = dir.join(family).join(key);
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join(format!("{key}.onnx")), vec![0u8; *size as usize]).unwrap();
        std::fs::write(at.join(format!("{key}.onnx.json")), "{}").unwrap();
    }
    std::fs::write(dir.join("voices.json"), catalog_of(voices)).unwrap();
    format!("file://{}", dir.display())
}

#[test]
fn the_catalog_is_fetched_and_listed_in_the_menu() {
    if !curl() {
        return;
    }
    let host = tempfile::tempdir().unwrap();
    let root = hosted(host.path(), &[("en_US-amy-medium", "en", "English", 10)]);
    let mut menu = Menu::new(vec![ready("espeak")], None, Ok(Catalog::fetch(&root)));
    let start = std::time::Instant::now();
    while !menu.poll() {
        assert!(start.elapsed().as_secs() < 10, "the catalog never came");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(names(&menu.voices), ["espeak", "en_US-amy-medium"]);
    assert!(menu.trouble.is_none());
}

#[test]
fn a_voice_downloaded_is_put_in_place_and_read_in() {
    if !curl() {
        return;
    }
    let (host, voices) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let listed = [("en_US-amy-medium", "en", "English", 5000)];
    let root = hosted(host.path(), &listed);
    let json = std::fs::read_to_string(host.path().join("voices.json")).unwrap();
    let remote = match catalog(&json, &HashSet::new(), "en").unwrap().remove(0).how {
        How::Fetch(remote) => remote,
        How::Ready(_) => panic!("not to download"),
    };
    let to = voices.path().join("piper");

    let download = Download::start(&remote, &root, &to);
    let ended = loop {
        if let Some(ended) = download.ended() {
            break ended;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let model = to.join("en_US-amy-medium.onnx");
    assert_eq!(
        ended,
        Ok(format!(
            "piper -m {} -f -",
            remote::quote(model.to_str().unwrap())
        ))
    );
    assert_eq!(speak::piper_models(&to), [model]);
    assert_eq!(download.progress(), 1.);
    // No part, nor log, is left.
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 2);
}

#[test]
fn a_download_that_fails_leaves_nothing() {
    if !curl() {
        return;
    }
    let (host, voices) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = hosted(host.path(), &[("en_US-amy-medium", "en", "English", 10)]);
    // The catalog says the model is larger than it is.
    let json = catalog_of(&[("en_US-amy-medium", "en", "English", 11)]);
    let remote = match catalog(&json, &HashSet::new(), "en").unwrap().remove(0).how {
        How::Fetch(remote) => remote,
        How::Ready(_) => panic!("not to download"),
    };
    let to = voices.path().join("piper");
    let download = Download::start(&remote, &root, &to);
    let ended = loop {
        if let Some(ended) = download.ended() {
            break ended;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(ended.is_err_and(|e| e.contains("came short")));
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);

    let missing = Download::start(&remote, &format!("{root}/nowhere"), &to);
    while missing.ended().is_none() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
}
