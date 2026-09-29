use super::*;
use crate::fixture;
use crate::render::Deck;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

/// An ssh that runs the command here, and cannot reach the host `down`.
fn fake_ssh(dir: &Path) -> PathBuf {
    let ssh = dir.join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\n\
         while :; do case \"$1\" in -o) shift 2 ;; -T) shift ;; *) break ;; esac; done\n\
         if [ \"$1\" = down ]; then echo 'ssh: connect to host down: Connection refused' >&2; exit 255; fi\n\
         shift\n\
         exec sh -c \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    ssh
}

#[test]
fn host_and_path_are_split_as_scp_does() {
    fn split(s: &str) -> Option<(&str, &str)> {
        super::split(Path::new(s))
    }
    assert_eq!(
        split("ypark@astrocyte:~/work/slides.pdf"),
        Some(("ypark@astrocyte", "~/work/slides.pdf"))
    );
    assert_eq!(
        split("astrocyte:/tmp/a.pdf"),
        Some(("astrocyte", "/tmp/a.pdf"))
    );
    assert_eq!(split("slides.pdf"), None);
    assert_eq!(split("./talk:v2.pdf"), None);
    assert_eq!(split("talks/talk:v2.pdf"), None);
    assert_eq!(split(":slides.pdf"), None);
    assert_eq!(split("astrocyte:"), None);
}

#[test]
fn a_local_file_with_a_colon_is_not_remote() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("talk:v2.pdf");
    std::fs::write(&local, b"").unwrap();
    assert_eq!(split(&local), None);
    let relative = Path::new("talk:v2.pdf");
    assert_eq!(split(relative), Some(("talk", "v2.pdf")));
}

#[test]
fn paths_are_quoted_for_sh_leaving_home_to_expand() {
    assert_eq!(quote_path("/a b/it's.pdf"), r"'/a b/it'\''s.pdf'");
    assert_eq!(quote_path("~/talk.pdf"), "~/'talk.pdf'");
}

#[test]
fn a_remote_pdf_is_copied_and_follows_changes() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let far = tempfile::tempdir().unwrap();
    let odd = far.path().join("it's a talk");
    std::fs::create_dir(&odd).unwrap();
    let original = fixture::write(&odd, 3);

    let remote = Remote::open(ssh.as_os_str(), "astrocyte", original.to_str().unwrap()).unwrap();
    assert_eq!(remote.path().file_name().unwrap(), "deck.pdf");
    assert_eq!(Deck::open(remote.path()).unwrap().pages, 3);

    fixture::write(&odd, 5);
    wait_for(&remote, 5);

    let copy = remote.path().to_path_buf();
    drop(remote);
    assert!(!copy.exists(), "the copy outlived ohp");
}

/// Wait up to ten seconds for the copy to have `pages` pages.
fn wait_for(remote: &Remote, pages: usize) {
    let start = Instant::now();
    while Deck::open(remote.path()).map_or(0, |d| d.pages) != pages {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the copy never changed"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn remote_names_cannot_clash_with_files_kept_beside_the_copy() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let far = tempfile::tempdir().unwrap();
    let deck = fixture::write(far.path(), 3);
    for name in ["ssh", "ssh.log", "part", "x.part", "copy"] {
        let original = far.path().join(name);
        std::fs::copy(&deck, &original).unwrap();
        let remote =
            Remote::open(ssh.as_os_str(), "astrocyte", original.to_str().unwrap()).unwrap();
        assert_eq!(remote.path().file_name().unwrap(), name);
        assert_eq!(Deck::open(remote.path()).unwrap().pages, 3, "{name}");
    }
}

#[test]
fn the_control_socket_path_is_short() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let far = tempfile::tempdir().unwrap();
    let original = fixture::write(far.path(), 1);
    let remote = Remote::open(ssh.as_os_str(), "astrocyte", original.to_str().unwrap()).unwrap();
    // ssh adds a suffix of up to 17 bytes, and macOS allows 104.
    let socket = remote
        .path()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ssh");
    assert!(socket.as_os_str().len() + 17 < 104, "{}", socket.display());
}

#[test]
fn a_lost_connection_is_reported_and_remade() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let far = tempfile::tempdir().unwrap();
    let original = fixture::write(far.path(), 3);
    let remote = Remote::open(ssh.as_os_str(), "astrocyte", original.to_str().unwrap()).unwrap();
    assert_eq!(remote.link().trouble(), None);

    let mut child = remote.master.lock().unwrap().child.take().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let start = Instant::now();
    while remote.link().trouble().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the loss went unnoticed"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(remote.link().trouble().unwrap().contains("astrocyte"));

    fixture::write(far.path(), 5);
    wait_for(&remote, 5);
    assert_eq!(remote.link().trouble(), None);
}

#[test]
fn a_missing_remote_file_is_reported() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let err = Remote::open(ssh.as_os_str(), "astrocyte", "/no/such/slides.pdf")
        .err()
        .expect("opened a missing file");
    assert_eq!(err.to_string(), "astrocyte:/no/such/slides.pdf not found");
}

#[test]
fn an_unreachable_host_reports_what_ssh_said() {
    let tools = tempfile::tempdir().unwrap();
    let ssh = fake_ssh(tools.path());
    let err = Remote::open(ssh.as_os_str(), "down", "/tmp/slides.pdf")
        .err()
        .expect("reached an unreachable host");
    assert!(err.to_string().contains("Connection refused"), "{err}");
}
