use super::*;

// Nothing here installs the handlers: they would stay for the other tests,
// and the developer's Ctrl-C.

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
