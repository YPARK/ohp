//! Programs ohp runs and must be able to end, as knitr's R and LaTeX.
//!
//! Each runs in a session of its own, without a terminal: what it starts,
//! as a chunk's processes or LaTeX's font makers, is in its process group,
//! to be ended with it, and none can read the terminal, as to ask for a
//! password, and wait there for ever, nor write over the slides. It is
//! waited on until it exits, ohp stops it, or its time runs out.
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
}

/// Most programs that may run at once: more are refused.
#[cfg(unix)]
pub const MOST: usize = group::SLOTS;

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
    if stop.load(Ordering::Relaxed) {
        return Ok(Ended::Stopped);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is safe to call between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            })
        };
    }
    let (mut child, known) = group::start(command)?;
    let Some(known) = known else {
        end(&mut child);
        let _ = child.wait();
        return Err(io::Error::other("too many programs running at once"));
    };
    let start = Instant::now();
    let mut poll = FIRST_POLL;
    let ended = loop {
        match exited(&mut child) {
            Ok(true) => break None,
            Ok(false) => {}
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
    let status = child.wait();
    // Ended by ohp, it is so however its reaping went.
    match ended {
        Some(ended) => Ok(ended),
        None => status.map(Ended::Exited),
    }
}

/// Kill every group running, as ohp ends in a hurry. Safe in a signal
/// handler: it only reads atomics, sleeps and sends signals.
#[cfg(unix)]
pub fn kill_all() {
    group::kill_all();
}

/// Whether the program has exited, leaving it unreaped.
#[cfg(unix)]
fn exited(child: &mut Child) -> io::Result<bool> {
    Ok(wait(child, libc::WEXITED | libc::WNOHANG | libc::WNOWAIT)?.is_some_and(is_exit))
}

#[cfg(not(unix))]
fn exited(child: &mut Child) -> io::Result<bool> {
    Ok(child.try_wait()?.is_some())
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
            Some(code) if is_exit(code) => return Ok(()),
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
fn is_exit(code: libc::c_int) -> bool {
    matches!(code, libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED)
}

/// The process groups running, for `kill_all`.
#[cfg(unix)]
mod group {
    use std::io;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

    pub const SLOTS: usize = 64;

    /// Groups running, by id; 0 where a slot is free.
    static GROUPS: [AtomicI32; SLOTS] = [const { AtomicI32::new(0) }; SLOTS];

    /// Programs being started, not yet known: `kill_all` waits for them.
    static STARTING: AtomicUsize = AtomicUsize::new(0);

    /// Longest `kill_all` waits for programs being started, in steps of
    /// 10ms: it may run on the very thread starting one.
    const STARTING_WAIT: u32 = 20;

    /// A group known while it runs.
    pub struct Known(&'static AtomicI32);

    impl Drop for Known {
        fn drop(&mut self) {
            self.0.store(0, Ordering::SeqCst);
        }
    }

    /// Start `command` and know its group; `None` if no slot is free.
    pub fn start(command: &mut Command) -> io::Result<(Child, Option<Known>)> {
        STARTING.fetch_add(1, Ordering::SeqCst);
        let started = command.spawn().map(|child| {
            let id = i32::try_from(child.id()).unwrap_or(0);
            let slot = GROUPS.iter().find(|s| {
                id != 0
                    && s.compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
            });
            (child, slot.map(Known))
        });
        STARTING.fetch_sub(1, Ordering::SeqCst);
        started
    }

    /// The groups running, by id.
    #[cfg(test)]
    pub fn running() -> Vec<i32> {
        GROUPS
            .iter()
            .map(|s| s.load(Ordering::SeqCst))
            .filter(|&id| id != 0)
            .collect()
    }

    pub fn kill(id: libc::pid_t) {
        // SAFETY: killpg only sends a signal, to a group of ohp's own.
        unsafe { libc::killpg(id, libc::SIGKILL) };
    }

    pub fn kill_all() {
        let step = libc::timespec {
            tv_sec: 0,
            tv_nsec: 10_000_000,
        };
        for _ in 0..STARTING_WAIT {
            if STARTING.load(Ordering::SeqCst) == 0 {
                break;
            }
            // SAFETY: nanosleep is safe in a signal handler.
            unsafe { libc::nanosleep(&step, std::ptr::null_mut()) };
        }
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
    use std::io;
    use std::process::{Child, Command};

    pub struct Known;

    pub fn start(command: &mut Command) -> io::Result<(Child, Option<Known>)> {
        Ok((command.spawn()?, Some(Known)))
    }
}

#[cfg(test)]
#[path = "../tests/process/child.rs"]
mod tests;
