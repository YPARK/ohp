//! Programs ohp runs and must be able to end, as knitr's R and LaTeX.
//!
//! Each runs in a process group of its own, so what it starts, as a chunk's
//! processes or LaTeX's font makers, can be ended with it. It is waited on
//! until it exits, ohp stops it, or its time runs out. One paused by a
//! signal, as a program that touched the terminal from the background, would
//! never go on, and is ended too.
//!
//! Ending kills the group twice: while the program, unreaped, holds the
//! group's id, so no other group can have been given it, and once more
//! after it has gone, for a process forked as the group was first killed.
//! Groups running are known, so ohp, ending in a hurry, can kill them all.

use std::io;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How a program run came to its end.
#[derive(Debug)]
pub enum Ended {
    Exited(ExitStatus),
    /// Ended as ohp asked.
    Stopped,
    /// Ended for running past its time.
    TimedOut,
    /// Ended for being paused by a signal.
    Paused,
}

/// First wait between looks at a program, so a quick one is seen done soon.
const FIRST_POLL: Duration = Duration::from_millis(2);
/// Longest wait between looks, so a long one costs few wake-ups.
const LAST_POLL: Duration = Duration::from_millis(50);

/// Run `command` until it ends, as it does itself or as `stop` or `timeout`
/// ends it, and with it all it started.
pub fn run(
    command: &mut Command,
    stop: &AtomicBool,
    timeout: Option<Duration>,
) -> io::Result<Ended> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    let mut child = command.spawn()?;
    let known = group::Known::new(&child);
    let start = Instant::now();
    let mut poll = FIRST_POLL;
    let ended = loop {
        match state(&mut child) {
            Ok(State::Running) => {}
            Ok(State::Exited) => break None,
            Ok(State::Paused) => break Some(Ended::Paused),
            Err(e) => {
                end(&mut child);
                drop(known);
                let _ = child.wait();
                return Err(e);
            }
        }
        if stop.load(Ordering::Relaxed) {
            break Some(Ended::Stopped);
        }
        if timeout.is_some_and(|t| start.elapsed() > t) {
            break Some(Ended::TimedOut);
        }
        std::thread::sleep(poll);
        poll = (poll * 2).min(LAST_POLL);
    };
    end(&mut child);
    // No longer killed by `kill_all` once reaped: its group's id is free.
    drop(known);
    let status = child.wait()?;
    Ok(ended.unwrap_or(Ended::Exited(status)))
}

/// Kill every group running, as ohp ends in a hurry. Safe in a signal
/// handler: it only reads atomics and sends signals.
#[cfg(unix)]
pub fn kill_all() {
    group::kill_all();
}

enum State {
    Running,
    Exited,
    Paused,
}

/// Whether the program has exited or been paused, leaving it unreaped.
#[cfg(unix)]
fn state(child: &mut Child) -> io::Result<State> {
    let flags = libc::WEXITED | libc::WSTOPPED | libc::WNOHANG | libc::WNOWAIT;
    Ok(match wait(child, flags)? {
        None => State::Running,
        Some(code) if exited(code) => State::Exited,
        Some(libc::CLD_STOPPED | libc::CLD_TRAPPED) => State::Paused,
        Some(_) => State::Running,
    })
}

#[cfg(not(unix))]
fn state(child: &mut Child) -> io::Result<State> {
    Ok(match child.try_wait()? {
        Some(_) => State::Exited,
        None => State::Running,
    })
}

/// End the program and its group, leaving it to be reaped.
#[cfg(unix)]
fn end(child: &mut Child) {
    let Ok(id) = libc::pid_t::try_from(child.id()) else {
        let _ = child.kill();
        return;
    };
    group::kill(id);
    // Once it has exited, unreaped, its group is killed again. Failing to
    // see it exit, the group's id may since be another's: not again.
    if wait_until_exited(child).is_ok() {
        group::kill(id);
    }
}

#[cfg(not(unix))]
fn end(child: &mut Child) {
    let _ = child.kill();
}

/// Wait, leaving it unreaped, until the program, killed, has exited.
#[cfg(unix)]
fn wait_until_exited(child: &mut Child) -> io::Result<()> {
    loop {
        // macOS reports a pause too, though only an exit is asked for.
        match wait(child, libc::WEXITED | libc::WNOWAIT)? {
            Some(code) if exited(code) => return Ok(()),
            _ => std::thread::sleep(FIRST_POLL),
        }
    }
}

/// What `waitid` reports of the program with `flags`, if anything: its
/// `si_code`.
#[cfg(unix)]
fn wait(child: &Child, flags: libc::c_int) -> io::Result<Option<libc::c_int>> {
    let pid = libc::id_t::from(child.id());
    loop {
        // SAFETY: a zeroed siginfo_t is valid, and waitid only fills it in.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid siginfo_t to write to.
        if unsafe { libc::waitid(libc::P_PID, pid, &mut info, flags) } != 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        // SAFETY: waitid filled in a child's pid, or left it zero.
        let reported = unsafe { info.si_pid() } != 0;
        return Ok(reported.then_some(info.si_code));
    }
}

#[cfg(unix)]
fn exited(code: libc::c_int) -> bool {
    matches!(code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED)
}

/// The process groups running, for `kill_all`.
#[cfg(unix)]
mod group {
    use std::process::Child;
    use std::sync::atomic::{AtomicI32, Ordering};

    /// Slots for groups running: more than ohp runs at once.
    static GROUPS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

    /// A group known while it runs.
    pub struct Known(Option<&'static AtomicI32>);

    impl Known {
        pub fn new(child: &Child) -> Self {
            let id = i32::try_from(child.id()).unwrap_or(0);
            let slot = GROUPS.iter().find(|s| {
                id != 0
                    && s.compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
            });
            Known(slot)
        }
    }

    impl Drop for Known {
        fn drop(&mut self) {
            if let Some(slot) = self.0 {
                slot.store(0, Ordering::SeqCst);
            }
        }
    }

    pub fn kill(id: libc::pid_t) {
        // SAFETY: killpg only sends a signal, to a group of ohp's own.
        unsafe { libc::killpg(id, libc::SIGKILL) };
    }

    pub fn kill_all() {
        for slot in &GROUPS {
            match slot.load(Ordering::SeqCst) {
                0 => {}
                id => kill(id),
            }
        }
    }
}

#[cfg(not(unix))]
mod group {
    pub struct Known;

    impl Known {
        pub fn new(_: &std::process::Child) -> Self {
            Known
        }
    }
}

#[cfg(test)]
#[path = "../tests/process/child.rs"]
mod tests;
