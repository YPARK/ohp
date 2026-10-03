use super::*;
use crate::fixture::{gone, marked_sleep};
use std::process::Stdio;
use std::sync::Arc;

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command
        .args(["-c", script])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[test]
fn a_program_that_ends_gives_its_status() {
    let go = AtomicBool::new(false);
    let ended = run(&mut shell("exit 3"), &go, None).unwrap();
    assert!(
        matches!(ended, Ended::Exited(s) if s.code() == Some(3)),
        "{ended:?}"
    );
}

#[test]
fn a_program_not_found_is_an_error() {
    let go = AtomicBool::new(false);
    let ran = run(&mut Command::new("no-such-program-ohp"), &go, None);
    assert_eq!(ran.unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_stop_ends_the_program_and_what_it_started() {
    let sleep = marked_sleep(71);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        stopping.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    let ended = run(&mut shell(&format!("{sleep} & wait")), &stop, None).unwrap();
    assert!(matches!(ended, Ended::Stopped), "{ended:?}");
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(gone(&sleep), "what it started outlived it");
}

#[test]
fn a_program_past_its_time_is_ended() {
    let sleep = marked_sleep(72);
    let go = AtomicBool::new(false);
    let timeout = Some(Duration::from_millis(300));
    let ended = run(&mut shell(&format!("{sleep} & wait")), &go, timeout).unwrap();
    assert!(matches!(ended, Ended::TimedOut), "{ended:?}");
    assert!(gone(&sleep), "what it started outlived it");
}

#[cfg(unix)]
#[test]
fn a_program_leads_a_session_of_its_own() {
    // In a session of its own it has no terminal, to ask for a password on
    // and wait there, as a chunk might.
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let running = std::thread::spawn(move || run(&mut shell("sleep 30"), &stopping, None));
    let start = Instant::now();
    // Each program running, any test's, with its session, read while it
    // runs: each is to lead its own.
    let mut sessions = Vec::new();
    while sessions.is_empty() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
        sessions = group::running()
            .into_iter()
            // SAFETY: getsid only reads a process's session.
            .map(|id| (id, unsafe { libc::getsid(id) }))
            // -1 for one that ended as it was read.
            .filter(|&(_, session)| session != -1)
            .collect();
    }
    stop.store(true, Ordering::Relaxed);
    running.join().unwrap().unwrap();
    assert!(!sessions.is_empty());
    for (id, session) in sessions {
        assert_eq!(session, id, "{id} does not lead its session");
    }
}

#[test]
fn a_program_is_not_started_once_stopped() {
    let stopped = AtomicBool::new(true);
    let ended = run(&mut Command::new("no-such-program-ohp"), &stopped, None).unwrap();
    assert!(matches!(ended, Ended::Stopped), "{ended:?}");
}

#[test]
fn what_a_program_left_running_is_ended_with_it() {
    let sleep = marked_sleep(73);
    let go = AtomicBool::new(false);
    // It exits at once, leaving the sleep in its group.
    let ended = run(&mut shell(&format!("{sleep} &")), &go, None).unwrap();
    assert!(
        matches!(ended, Ended::Exited(s) if s.success()),
        "{ended:?}"
    );
    assert!(gone(&sleep), "what it left running outlived it");
}
