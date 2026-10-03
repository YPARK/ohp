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

/// Run in a test process of its own, by the test below.
#[test]
#[ignore = "installs signal handlers: run alone, by a_signal_sets_the_flags"]
fn installed() {
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
