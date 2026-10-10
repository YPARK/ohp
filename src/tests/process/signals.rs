use super::*;

// Only in a test process of its own are the handlers installed: they would
// stay for the other tests, and the developer's Ctrl-C.

#[test]
fn the_third_signal_ends_ohp_at_once() {
    let seen = AtomicUsize::new(0);
    assert!(!out_of_patience(&seen), "the first asks");
    assert!(!out_of_patience(&seen), "the second asks again");
    assert!(out_of_patience(&seen), "the third ends it");
    assert!(out_of_patience(&seen));
}

#[test]
fn a_signal_ends_ohp_with_the_status_a_shell_gives() {
    assert_eq!(crate::shell_status(SIGINT as usize), 130);
    assert_eq!(crate::shell_status(SIGTERM as usize), 143);
    assert_eq!(crate::shell_status(SIGHUP as usize), 129);
    assert_eq!(crate::shell_status(200), u8::MAX);
}

#[test]
fn a_terminal_is_open_until_it_hangs_up() {
    // SAFETY: opens a pseudo-terminal and its other end, as C strings name,
    // and closes them.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(master >= 0);
        assert_eq!(libc::grantpt(master), 0);
        assert_eq!(libc::unlockpt(master), 0);
        let name = libc::ptsname(master);
        assert!(!name.is_null());
        let slave = libc::open(name, libc::O_RDWR | libc::O_NOCTTY);
        assert!(slave >= 0);
        assert!(open(slave), "a terminal not hung up");
        // Its window closed.
        libc::close(master);
        assert!(!open(slave), "a terminal hung up");
        libc::close(slave);
        assert!(!open(slave), "no terminal");
    }
}

/// Set for the test process `installed` runs alone in.
const ALONE: &str = "OHP_TEST_SIGNALS_ALONE";

/// Run in a test process of its own, by the test below.
#[test]
#[ignore = "installs signal handlers: run alone, by a_signal_sets_the_flags"]
fn installed() {
    // Run with the others, as by --include-ignored, it would install them
    // there.
    if std::env::var_os(ALONE).is_none() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let signaled = Arc::new(AtomicUsize::new(0));
    install(&stop, &signaled).unwrap();
    // SAFETY: raises a signal a handler is installed for, once.
    unsafe { libc::raise(SIGHUP) };
    assert!(stop.load(Ordering::Relaxed), "typesetting is not stopped");
    assert_eq!(signaled.load(Ordering::Relaxed), SIGHUP as usize);
}

#[test]
fn a_signal_sets_the_flags() {
    let tests = std::env::current_exe().unwrap();
    let ran = std::process::Command::new(tests)
        .env(ALONE, "1")
        .args(["--ignored", "--exact", "process::signals::tests::installed"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(out.contains("1 passed"), "{out}");
}
