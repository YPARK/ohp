use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// sherpa-onnx's release of voices, as GitHub's API gives it, of `files`,
/// each its name and size.
fn release_of(files: &[(&str, u64)]) -> String {
    let assets: Vec<String> = (files.iter())
        .map(|(name, size)| format!(r#"{{"name": "{name}", "size": {size}, "state": "uploaded"}}"#))
        .collect();
    format!(
        r#"{{"tag_name": "tts-models", "assets": [{}]}}"#,
        assets.join(",")
    )
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
    let json = release_of(&[
        ("vits-piper-de_DE-thorsten-medium.tar.bz2", 3_000_000),
        ("vits-piper-en_US-ryan-high.tar.bz2", 1),
        ("vits-piper-en_GB-alan-medium.tar.bz2", 1),
        ("kokoro-en-v0_19.tar.bz2", 1),
    ]);
    let voices = catalog(&json, "en_CA.UTF-8").unwrap();
    assert_eq!(
        names(&voices),
        [
            "en_GB-alan-medium",
            "en_US-ryan-high",
            "de_DE-thorsten-medium"
        ]
    );
    assert_eq!(voices[2].about, "de_DE · medium · 3 MB");
    let How::Fetch(remote) = &voices[2].how else {
        panic!("not to download")
    };
    assert_eq!(remote.file, "vits-piper-de_DE-thorsten-medium.tar.bz2");
    assert!(catalog("not json", "en").is_err());
    assert!(catalog(r#"{"message": "API rate limit exceeded"}"#, "en").is_err());
}

#[test]
fn only_piper_s_voices_that_sound_natural_are_listed() {
    let unnatural = [
        "vits-piper-en_US-amy-low.tar.bz2",
        "vits-piper-en_US-kathleen-x_low.tar.bz2",
        "vits-piper-en_US-ryan-high-int8.tar.bz2",
        "vits-piper-en_US-ryan-high-fp16.tar.bz2",
        "vits-piper-en_US-glados.tar.bz2",
        "vits-piper-en_US-glados-high.tar.bz2",
        "vits-piper-en_US-arctic-medium.tar.bz2",
        "vits-piper-en_US-l2arctic-medium.tar.bz2",
        "vits-piper-en_GB-southern_english_female_medium.tar.bz2",
    ];
    let mut files: Vec<(&str, u64)> = unnatural.iter().map(|f| (*f, 1)).collect();
    files.push(("vits-piper-en_US-lessac-high.tar.bz2", 1));
    let voices = catalog(&release_of(&files), "en").unwrap();
    assert_eq!(names(&voices), ["en_US-lessac-high"]);
}

#[test]
fn of_piper_s_english_voices_only_those_picked_are_listed() {
    let json = release_of(&[
        ("vits-piper-en_US-kristin-medium.tar.bz2", 1),
        ("vits-piper-en_GB-vctk-medium.tar.bz2", 1),
        ("vits-piper-en_GB-cori-high.tar.bz2", 1),
        ("vits-piper-en_GB-jenny_dioco-medium.tar.bz2", 1),
        ("vits-piper-de_DE-kerstin-medium.tar.bz2", 1),
    ]);
    let voices = catalog(&json, "en").unwrap();
    assert_eq!(
        names(&voices),
        [
            "en_GB-cori-high",
            "en_GB-jenny_dioco-medium",
            "de_DE-kerstin-medium"
        ]
    );
}

#[test]
fn the_voices_at_hand_are_piper_s_others_and_the_one_given() {
    let dir = tempfile::tempdir().unwrap();
    let touch = |name: &str| std::fs::write(dir.path().join(name), "").unwrap();
    touch("en_US-amy-medium.onnx");
    touch("en_US-amy-medium.onnx.json");
    let amy = speak::piper_with(&dir.path().join("en_US-amy-medium.onnx")).unwrap();
    let others = [("espeak-ng", "espeak-ng --stdout")];
    let piper = speak::Engines {
        sherpa: None,
        piper: true,
    };

    let voices = at_hand(Some("my-voice"), Some(dir.path()), &piper, &others);
    assert_eq!(names(&voices), ["given", "en_US-amy-medium", "espeak-ng"]);
    assert_eq!(voices[1].how, How::Ready(amy.clone()));
    // Given one of those listed anyway, it is not listed twice.
    let voices = at_hand(Some(&amy), Some(dir.path()), &piper, &others);
    assert_eq!(names(&voices), ["en_US-amy-medium", "espeak-ng"]);
    // With nothing to speak them, Piper's voices are not listed.
    let none = speak::Engines::default();
    assert_eq!(
        names(&at_hand(None, Some(dir.path()), &none, &others)),
        ["espeak-ng"]
    );
}

#[test]
fn a_voice_given_is_the_default() {
    assert_eq!(
        default(Some("my-voice".into())).as_deref(),
        Some("my-voice")
    );
}

#[test]
fn macos_s_voices_are_listed_the_user_s_language_first() {
    let listing = "Anna                de_DE    # Hallo! Ich heiße Anna.\n\
                   Ava (Premium)       en_US    # Hello! My name is Ava.\n\
                   Daniel              en_GB    # Hello! My name is Daniel.\n\
                   not a voice\n";
    let voices = macos(listing, "en_US.UTF-8", |name| format!("say -v '{name}'"));
    assert_eq!(names(&voices), ["Ava (Premium)", "Daniel", "Anna"]);
    assert_eq!(voices[1].about, "macOS · en_GB");
    assert_eq!(voices[0].how, How::Ready("say -v 'Ava (Premium)'".into()));
}

#[test]
fn macos_s_robotic_voices_are_left_out() {
    let listing = "Eddy (English (UK)) en_GB    # Hello! My name is Eddy.\n\
                   Grandma (German (Germany)) de_DE    # Hallo! Ich heiße Grandma.\n\
                   Bad News            en_US    # Hello! My name is Bad News.\n\
                   Zarvox              en_US    # Hello! My name is Zarvox.\n\
                   Fred                en_US    # Hello! My name is Fred.\n\
                   Samantha            en_US    # Hello! My name is Samantha.\n";
    let voices = macos(listing, "en_US.UTF-8", |name| name.into());
    assert_eq!(names(&voices), ["Samantha"]);
}

#[test]
fn of_macos_s_english_voices_only_those_picked_and_downloaded_are_listed() {
    let listing = "Karen               en_AU    # Hello! My name is Karen.\n\
                   Moira               en_IE    # Hello! My name is Moira.\n\
                   Zoe (Enhanced)      en_US    # Hello! My name is Zoe.\n\
                   Daniel              en_GB    # Hello! My name is Daniel.\n\
                   Thomas              fr_FR    # Bonjour, je m’appelle Thomas.\n";
    let voices = macos(listing, "en_US.UTF-8", |name| name.into());
    assert_eq!(names(&voices), ["Daniel", "Zoe (Enhanced)", "Thomas"]);
}

#[cfg(target_os = "macos")]
#[test]
fn a_macos_voice_is_said_in_by_name() {
    let say = speak::say_in("Eddy (English (UK))");
    assert!(
        say.contains("say -v 'Eddy (English (UK))' -f - -o"),
        "{say}"
    );
}

/// `files`, each a path and its text, as a `.tar.bz2` of directory `top`
/// in `dir`, named `name`: its path.
fn archive(dir: &Path, name: &str, top: &str, files: &[(&str, &str)]) -> PathBuf {
    let src = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let at = src.path().join(top).join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }
    std::fs::create_dir_all(dir).unwrap();
    let to = dir.join(name);
    let made = Command::new("tar")
        .arg("-cjf")
        .arg(&to)
        .arg("-C")
        .arg(src.path())
        .arg(top)
        .status()
        .unwrap();
    assert!(made.success());
    to
}

/// sherpa-onnx's release, as a `file://` URL to an archive in `dir`.
fn sherpa_release(dir: &Path) -> String {
    let release = archive(
        dir,
        "sherpa-onnx-shared.tar.bz2",
        "sherpa-onnx-v1-osx-arm64-shared",
        &[
            ("bin/sherpa-onnx-offline-tts", "#!/bin/sh\n"),
            ("bin/sherpa-onnx-microphone", ""),
            ("include/c-api.h", ""),
            ("lib/libonnxruntime.dylib", ""),
        ],
    );
    format!("file://{}", release.display())
}

fn tar() -> bool {
    speak::installed("tar") && speak::installed("bzip2") && curl()
}

#[test]
fn sherpa_onnx_is_installed_its_program_speaking_and_its_libraries() {
    if !tar() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let url = sherpa_release(&dir.path().join("host"));
    let voices = dir.path().join("piper");
    assert_eq!(done(&install_sherpa(&voices, &url)), Ok(()));
    let home = speak::sherpa_home(&voices);
    assert!(speak::sherpa_installed(&voices).is_file());
    assert!(home.join("lib/libonnxruntime.dylib").is_file());
    // Its other programs and headers are not kept, nor the archive.
    assert_eq!(std::fs::read_dir(home.join("bin")).unwrap().count(), 1);
    assert!(!home.join("include").exists());
    assert_eq!(
        std::fs::read_dir(&voices).unwrap().count(),
        1,
        "only sherpa-onnx is left"
    );
}

#[test]
fn a_failed_install_says_why_and_leaves_nothing() {
    if !tar() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let voices = dir.path().join("piper");
    let nowhere = format!("file://{}/nowhere.tar.bz2", dir.path().display());
    assert!(done(&install_sherpa(&voices, &nowhere)).is_err());
    assert_eq!(std::fs::read_dir(&voices).unwrap().count(), 0);

    // An archive without the program speaking.
    let wrong = archive(
        &dir.path().join("host"),
        "x.tar.bz2",
        "x",
        &[("bin/other", "")],
    );
    let url = format!("file://{}", wrong.display());
    let failed = done(&install_sherpa(&voices, &url)).unwrap_err();
    assert!(failed.contains("sherpa-onnx-offline-tts"), "{failed}");
    assert_eq!(std::fs::read_dir(&voices).unwrap().count(), 0);
}

#[test]
fn picking_sherpa_onnx_installs_it_and_opens_the_list_again() {
    if !tar() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let install = Voice {
        name: "sherpa-onnx".into(),
        about: "install".into(),
        how: How::Install,
    };
    let mut picker = open(vec![install], None);
    picker.sherpa = Some(sherpa_release(&dir.path().join("host")));
    picker.installable = Some(dir.path().join("piper"));
    assert!(press(&mut picker, &[KeyCode::Enter]).is_none());
    assert!(!picker.is_open());
    assert_eq!(picker.note().as_deref(), Some(" · installing sherpa-onnx…"));
    let start = Instant::now();
    let event = loop {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the install hung"
        );
        std::thread::sleep(Duration::from_millis(5));
        if let (_, Some(event)) = picker.poll() {
            break event;
        }
    };
    let Event::Notice(said) = event else {
        panic!("no notice")
    };
    assert!(said.contains("sherpa-onnx is installed"), "{said}");
    assert!(picker.is_open());
    assert!(speak::sherpa_installed(&dir.path().join("piper")).is_file());
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
    let mut picker = Voices::new("file:///nowhere", "file:///nowhere", None);
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
            file: "vits-piper-en_US-amy-medium.tar.bz2".into(),
            size: 1,
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

/// sherpa-onnx's release of Piper's voices `names`, in `dir`: each an
/// archive, and the release listing them. Its root, as a `file://` URL.
fn hosted(dir: &Path, names: &[&str]) -> String {
    let files: Vec<(String, u64)> = (names.iter())
        .map(|name| {
            let file = format!("vits-piper-{name}.tar.bz2");
            let top = format!("vits-piper-{name}");
            let (model, config) = (format!("{name}.onnx"), format!("{name}.onnx.json"));
            let made = archive(
                dir,
                &file,
                &top,
                &[
                    (&model, "model"),
                    (&config, "{}"),
                    ("tokens.txt", "_ 0"),
                    ("espeak-ng-data/en_dict", ""),
                ],
            );
            (file, std::fs::metadata(made).unwrap().len())
        })
        .collect();
    let files: Vec<(&str, u64)> = files.iter().map(|(f, s)| (f.as_str(), *s)).collect();
    std::fs::write(dir.join("release.json"), release_of(&files)).unwrap();
    format!("file://{}", dir.display())
}

#[test]
fn the_catalog_fetched_is_added_to_the_list_open() {
    if !tar() {
        return;
    }
    let host = tempfile::tempdir().unwrap();
    let root = hosted(host.path(), &["en_US-amy-medium"]);
    let mut picker = open(vec![ready("espeak")], None);
    picker.piper = Ok(PathBuf::from("/nowhere"));
    picker.catalog = Catalog::Fetching(fetch_catalog(&format!("{root}/release.json")));
    let start = Instant::now();
    while !matches!(picker.catalog, Catalog::Fetched(_)) {
        assert!(start.elapsed() < Duration::from_secs(10), "no catalog came");
        std::thread::sleep(Duration::from_millis(5));
        picker.poll();
    }
    let list = picker.list.as_ref().unwrap();
    assert_eq!(names(&list.voices), ["espeak", "en_US-amy-medium"]);
}

/// sherpa-onnx, as if installed.
fn sherpa() -> speak::Engines {
    speak::Engines {
        sherpa: Some(PathBuf::from("/opt/sherpa-onnx-offline-tts")),
        piper: false,
    }
}

/// Download the first of the voices `json` lists from `root`, to `to`.
fn download_from(json: &str, root: &str, to: &Path) -> (Download, Result<String, String>) {
    let voice = catalog(json, "en").unwrap().remove(0);
    let How::Fetch(remote) = &voice.how else {
        panic!("not to download")
    };
    let download = Download::start(&voice.name, remote, root, to, &sherpa());
    let done = done(&download.task);
    (download, done)
}

#[test]
fn a_voice_downloaded_is_unpacked_in_place_and_read_in() {
    if !tar() {
        return;
    }
    let (host, voices) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = hosted(host.path(), &["en_US-amy-medium"]);
    let json = std::fs::read_to_string(host.path().join("release.json")).unwrap();
    let to = voices.path().join("piper");
    let (download, done) = download_from(&json, &root, &to);

    let model = to.join("vits-piper-en_US-amy-medium/en_US-amy-medium.onnx");
    assert_eq!(done, Ok(sherpa().voice(&model).unwrap()));
    assert_eq!(speak::piper_models(&to), [model]);
    assert_eq!(download.percent(), 100);
    // No archive, part, nor log, is left.
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 1);
}

#[test]
fn a_download_that_fails_leaves_nothing() {
    if !tar() {
        return;
    }
    let (host, voices) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = hosted(host.path(), &["en_US-amy-medium"]);
    let to = voices.path().join("piper");
    // The catalog says the archive is larger than it is.
    let json = release_of(&[("vits-piper-en_US-amy-medium.tar.bz2", 1_000_000)]);
    let (_, done) = download_from(&json, &root, &to);
    assert!(done.is_err_and(|e| e.contains("came short")));
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);

    let (_, done) = download_from(&json, &format!("{root}/nowhere"), &to);
    assert!(done.is_err());
    assert_eq!(std::fs::read_dir(&to).unwrap().count(), 0);
}
