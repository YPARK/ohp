use super::*;

#[test]
fn a_signal_says_which_it_was_and_stops_typesetting() {
    let stop = Arc::new(AtomicBool::new(false));
    let signaled = Arc::new(AtomicUsize::new(0));
    install(&stop, &signaled).unwrap();
    // One only: the third would end the tests.
    // SAFETY: raises a signal a handler is installed for.
    unsafe { libc::raise(SIGHUP) };
    assert!(stop.load(Ordering::Relaxed));
    assert_eq!(signaled.load(Ordering::Relaxed), SIGHUP as usize);
}

#[test]
fn a_signal_ends_ohp_with_the_status_a_shell_gives() {
    assert_eq!(crate::shell_status(SIGINT as usize), 130);
    assert_eq!(crate::shell_status(SIGTERM as usize), 143);
    assert_eq!(crate::shell_status(200), u8::MAX);
}
