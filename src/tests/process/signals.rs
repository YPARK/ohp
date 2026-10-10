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

/// Set for the test process `installed` runs alone in.
const ALONE: &str = "OHP_TEST_SIGNALS_ALONE";

/// Run `test`, ignored, in a test process of its own.
fn alone(test: &str) -> std::process::Output {
    let tests = std::env::current_exe().unwrap();
    let test = format!("process::signals::tests::{test}");
    std::process::Command::new(tests)
        .env(ALONE, "1")
        .args(["--ignored", "--exact", &test])
        .output()
        .unwrap()
}

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
    unsafe { libc::raise(SIGTERM) };
    assert!(stop.load(Ordering::Relaxed), "typesetting is not stopped");
    assert_eq!(signaled.load(Ordering::Relaxed), SIGTERM as usize);
}

#[test]
fn a_signal_sets_the_flags() {
    let ran = alone("installed");
    let out = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{out}{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(out.contains("1 passed"), "{out}");
}

/// Run in a test process of its own, by the test below: hung up, and stuck,
/// as ohp is in crossterm then.
#[test]
#[ignore = "installs signal handlers: run alone, by a_hang_up_ends_ohp_stuck"]
fn hung_up() {
    if std::env::var_os(ALONE).is_none() {
        return;
    }
    install(&Arc::default(), &Arc::default()).unwrap();
    // SAFETY: raises a signal a handler is installed for, once.
    unsafe { libc::raise(SIGHUP) };
    std::thread::sleep(std::time::Duration::from_secs(30));
}

#[test]
fn a_hang_up_ends_ohp_stuck() {
    let start = std::time::Instant::now();
    let ran = alone("hung_up");
    assert_eq!(ran.status.code(), Some(129), "{ran:?}");
    assert!(start.elapsed().as_secs() < 10, "ended late");
}
