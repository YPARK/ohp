use super::*;
use crate::remote;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// A catalog of Piper's voices, as `voices.json`, of `voices`: each its
/// name, language family and English name, and its model's size.
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

/// What `task` came to, waiting for it a while.
fn done<T: Send + 'static>(task: &Task<T>) -> Result<T, String> {
    let start = Instant::now();
    loop {
        if let Some(done) = task.done() {
            return done;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "the task hung");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_catalog_lists_piper_s_voices_the_user_s_language_first() {
    let json = catalog_of(&[
        ("de_DE-thorsten-medium", "de", "German", 3_000_000),
        ("en_US-ryan-high", "en", "English", 1),
        ("en_GB-alba-medium", "en", "English", 1),
    ]);
    let voices = catalog(&json, "en_CA.UTF-8").unwrap();
    assert_eq!(
        names(&voices),
        [
            "en_GB-alba-medium",
            "en_US-ryan-high",
            "de_DE-thorsten-medium"
        ]
    );
    assert_eq!(voices[2].about, "German (Somewhere) · medium · 4 MB");
    // The model card is not downloaded.
    let How::Fetch(remote) = &voices[2].how else {
        panic!("not to download")
    };
    assert!(remote.model.0.ends_with(".onnx") && remote.config.0.ends_with(".onnx.json"));
    assert!(catalog("not json", "en").is_err());
}

#[test]
fn the_voices_at_hand_are_piper_s_others_and_the_one_given() {
    let dir = tempfile::tempdir().unwrap();
    let touch = |name: &str| std::fs::write(dir.path().join(name), "").unwrap();
    touch("en_US-amy-medium.onnx");
    touch("en_US-amy-medium.onnx.json");
    let amy = speak::piper_with(&dir.path().join("en_US-amy-medium.onnx")).unwrap();
    let others = [("espeak-ng", "espeak-ng --stdout")];

    let voices = at_hand(Some("my-voice"), Some(dir.path()), &others);
    assert_eq!(names(&voices), ["given", "en_US-amy-medium", "espeak-ng"]);
    assert_eq!(voices[1].how, How::Ready(amy.clone()));
    // Given one of those listed anyway, it is not listed twice.
    let voices = at_hand(Some(&amy), Some(dir.path()), &others);
    assert_eq!(names(&voices), ["en_US-amy-medium", "espeak-ng"]);
    // Without Piper, its voices are not listed.
    assert_eq!(names(&at_hand(None, None, &others)), ["espeak-ng"]);
}

#[test]
fn a_voice_given_is_the_default() {
    assert_eq!(
        default(Some("my-voice".into())).as_deref(),
        Some("my-voice")
    );
}

fn ready(name: &str) -> Voice {
    Voice {
        name: name.into(),
        about: "installed".into(),
        how: How::Ready(format!("{name} --stdout")),
    }
}

/// A picker of `voices`, open on them, `current` in use.
fn open(voices: Vec<Voice>, current: Option<&str>) -> Voices {
    let mut picker = Voices::new("file:///nowhere");
    picker.list = Some(List::new(voices, current));
    picker
}

fn press(picker: &mut Voices, keys: &[KeyCode]) -> Option<Event> {
    keys.iter()
        .fold(None, |_, &code| picker.key(KeyEvent::from(code)))
}

fn picked(picker: &Voices) -> Option<&str> {
    Some(picker.list.as_ref()?.picked()?.name.as_str())
}

#[test]
fn the_list_starts_on_the_voice_in_use_and_typing_narrows_it() {
    let voices = vec![ready("espeak-ng"), ready("espeak"), ready("say")];
    let mut picker = open(voices, Some("espeak --stdout"));
    assert_eq!(picked(&picker), Some("espeak"));

    // The voice the typing starts first, then those with its letters.
    press(&mut picker, &[KeyCode::Char('s'), KeyCode::Char('a')]);
    assert_eq!(picker.list.as_ref().unwrap().shown, [2, 0, 1]);
    let Some(Event::Use(name, command)) = press(&mut picker, &[KeyCode::Enter]) else {
        panic!("nothing picked")
    };
    assert_eq!((name.as_str(), command.as_str()), ("say", "say --stdout"));
    assert!(!picker.is_open());

    // Esc clears what is typed, then closes the list.
    let mut picker = open(vec![ready("say")], None);
    press(&mut picker, &[KeyCode::Char('z'), KeyCode::Char('z')]);
    assert_eq!(picked(&picker), None);
    press(&mut picker, &[KeyCode::Esc]);
    assert_eq!(picked(&picker), Some("say"));
    press(&mut picker, &[KeyCode::Esc]);
    assert!(!picker.is_open());
}

#[test]
fn ctrl_n_and_ctrl_p_move_as_down_and_up() {
    let voices = vec![ready("espeak-ng"), ready("espeak"), ready("say")];
    let mut picker = open(voices, None);
    let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
    picker.key(ctrl('n'));
    picker.key(ctrl('n'));
    assert_eq!(picked(&picker), Some("say"));
    picker.key(ctrl('p'));
    assert_eq!(picked(&picker), Some("espeak"));
}

#[test]
fn a_voice_to_download_says_why_it_cannot_be() {
    let far = Voice {
        name: "en_US-amy-medium".into(),
        about: "English".into(),
        how: How::Fetch(Remote {
            model: ("en/en_US-amy-medium.onnx".into(), 1),
            config: ("en/en_US-amy-medium.onnx.json".into(), 1),
        }),
    };
    let mut picker = open(vec![far], None);
    picker.piper = Err("install curl to download Piper's voices");
    let Some(Event::Notice(why)) = press(&mut picker, &[KeyCode::Enter]) else {
        panic!("nothing said")
    };
    assert!(why.contains("curl"));
    assert!(picker.download.is_none());
}

#[test]
fn the_catalog_added_keeps_the_voice_picked_and_lists_none_twice() {
    let mut list = List::new(vec![ready("espeak"), ready("say")], None);
    list.menu.pick(Some(1));
    let mut amy = ready("en_US-amy-medium");
    amy.about = "English".into();
    list.add(&[ready("say"), amy]);
    assert_eq!(names(&list.voices), ["espeak", "say", "en_US-amy-medium"]);
    assert_eq!(list.picked().map(|v| v.name.as_str()), Some("say"));
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
fn the_catalog_fetched_is_added_to_the_list_open() {
    if !curl() {
        return;
    }
    let host = tempfile::tempdir().unwrap();
    let root = hosted(host.path(), &[("en_US-amy-medium", "en", "English", 10)]);
    let mut picker = open(vec![ready("espeak")], None);
    picker.root = root;
    picker.piper = Ok(PathBuf::from("/nowhere"));
    picker.catalog = Catalog::Fetching(fetch_catalog(&picker.root));
    let start = Instant::now();
    while !matches!(picker.catalog, Catalog::Fetched(_)) {
        assert!(start.elapsed() < Duration::from_secs(10), "no catalog came");
        std::thread::sleep(Duration::from_millis(5));
        picker.poll();
    }
    let list = picker.list.as_ref().unwrap();
    assert_eq!(names(&list.voices), ["espeak", "en_US-amy-medium"]);
}

/// Download Piper's voice `key` from `root`, which `json` lists, to `to`.
fn download_from(json: &str, root: &str, to: &Path) -> (Download, Result<String, String>) {
    let voice = catalog(json, "en").unwrap().remove(0);
    let How::Fetch(remote) = &voice.how else {
        panic!("not to download")
    };
    let download = Download::start(&voice.name, remote, root, to);
    let done = done(&download.task);
    (download, done)
}

#[test]
fn a_voice_downloaded_is_put_in_place_and_read_in() {
    if !curl() {
        return;
    }
    let (host, voices) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let listed = [("en_US-amy-medium", "en", "English", 5000)];
    let root = hosted(host.path(), &listed);
    let to = voices.path().join("piper");
    let (download, done) = download_from(&catalog_of(&listed), &root, &to);

    let model = to.join("en_US-amy-medium.onnx");
    assert_eq!(
        done,
        Ok(format!(
            "piper -m {} -f -",
            remote::quote(model.to_str().unwrap())
        ))
    );
    assert_eq!(speak::piper_models(&to), [model]);
    assert_eq!(download.percent(), 100);
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
    let to = voices.path().join("piper");
    // The catalog says the model is larger than it is.
    let json = catalog_of(&[("en_US-amy-medium", "en", "English", 11)]);
    let (_, done) = download_from(&json, &root, &to);
    assert!(done.is_err_and(|e| e.contains("came short")));
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);

    let (_, done) = download_from(&json, &format!("{root}/nowhere"), &to);
    assert!(done.is_err());
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
}
