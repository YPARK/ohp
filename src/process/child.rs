//! Programs ohp runs and must be able to end, as knitr's R and LaTeX.
//!
//! Each runs in a session of its own, without a terminal: what it starts,
//! as a chunk's processes or LaTeX's font makers, is in its process group,
//! to be ended with it, and none can read the terminal, as to ask for a
//! password, and wait there for ever, nor write over the slides. It is
//! waited on until it exits, ohp stops it, or its time runs out. One paused,
//! as by a SIGSTOP, would never go on, and is ended too.
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
    /// Ended for being paused.
    Paused,
}

/// Most programs that may run at once: past it, one waits for another to
/// end before it is started.
pub const MOST: usize = 64;

/// First wait between looks at a program, so a quick one is seen done soon.
const FIRST_POLL: Duration = Duration::from_millis(2);
/// Longest wait between looks, so a long one costs few wake-ups.
const LAST_POLL: Duration = Duration::from_millis(50);

/// Run `command` until it ends, as it does itself or as `stop` or `timeout`
/// ends it, and with it all it started. It is taken, to be run once: each
/// run sets it up to lead a session.
pub fn run(
    mut command: Command,
    stop: &AtomicBool,
    timeout: Option<Duration>,
) -> io::Result<Ended> {
    // Known before it is started, waiting while too many run.
    let Some(known) = group::reserve(stop) else {
        return Ok(Ended::Stopped);
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: between fork and exec, only calls safe there: setsid and
        // sigprocmask.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                group::unblock_signals();
                Ok(())
            })
        };
    }
    let mut child = group::start(&mut command, &known)?;
    let start = Instant::now();
    let mut poll = FIRST_POLL;
    let ended = loop {
        // Asked to stop, it is stopped, whatever else it is.
        if stop.load(Ordering::Relaxed) {
            break Some(Ended::Stopped);
        }
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

/// The last line written to `log`, trimmed, of those `keep` keeps: as most
/// programs write why they failed last.
pub fn last_line(log: &std::path::Path, keep: impl Fn(&str) -> bool) -> Option<String> {
    let text = std::fs::read_to_string(log).ok()?;
    text.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && keep(l))
        .map(str::to_string)
}

/// Run `command`, what it writes on its error output to `log`, until it is
/// done or `stop` is set: whether it ran to its end, or why it failed, as
/// it wrote last.
#[cfg(feature = "speech")]
pub fn run_logged(
    mut command: Command,
    log: &std::path::Path,
    stop: &AtomicBool,
) -> Result<bool, String> {
    let stderr = std::fs::File::create(log).map_err(|e| e.to_string())?;
    command.stderr(stderr);
    match run(command, stop, None) {
        Ok(Ended::Exited(status)) if status.success() => Ok(true),
        Ok(Ended::Exited(_)) => Err(last_line(log, |_| true).unwrap_or_else(|| "it failed".into())),
        Ok(Ended::Stopped) => Ok(false),
        // Given no time limit, it cannot run past one.
        Ok(Ended::Paused | Ended::TimedOut) => Err("it was paused".into()),
        Err(e) => Err(e.to_string()),
    }
}

/// Kill every group running, as ohp ends in a hurry. Safe in a signal
/// handler: it only reads atomics, sleeps and sends signals.
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
        Some(code) if is_exit(code) => State::Exited,
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
    use super::MOST;
    use std::io;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
    use std::time::Duration;

    /// Groups running, by id; 0 where a slot is free, and `RESERVED` where
    /// a program is about to be started.
    static GROUPS: [AtomicI32; MOST] = [const { AtomicI32::new(0) }; MOST];

    const RESERVED: i32 = -1;

    /// How long one waits for a slot before looking again.
    const SLOT_WAIT: Duration = Duration::from_millis(10);

    /// Programs being started, not yet known: `kill_all` waits for them.
    static STARTING: AtomicUsize = AtomicUsize::new(0);

    /// Longest `kill_all` waits for programs being started, in steps of
    /// 10ms.
    const STARTING_WAIT: u32 = 20;

    /// The signals that end ohp in a hurry, held off a thread starting a
    /// program, so `kill_all` runs on another and waits for it to be known.
    const HURRY: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    /// A slot, reserved for a program and then knowing its group, until it
    /// is freed.
    pub struct Known(&'static AtomicI32);

    impl Drop for Known {
        fn drop(&mut self) {
            self.0.store(0, Ordering::SeqCst);
        }
    }

    /// A slot for a program, waited for while all are taken; `None` once
    /// `stop` is set.
    pub fn reserve(stop: &AtomicBool) -> Option<Known> {
        loop {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            let free = GROUPS.iter().find(|s| {
                s.compare_exchange(0, RESERVED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            });
            if let Some(slot) = free {
                return Some(Known(slot));
            }
            std::thread::sleep(SLOT_WAIT);
        }
    }

    /// Start `command`, its group known in `known`'s slot.
    pub fn start(command: &mut Command, known: &Known) -> io::Result<Child> {
        STARTING.fetch_add(1, Ordering::SeqCst);
        let held = hold_signals();
        let started = command
            .spawn()
            .and_then(|mut child| match i32::try_from(child.id()) {
                Ok(id) => {
                    known.0.store(id, Ordering::SeqCst);
                    Ok(child)
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(io::Error::other(e))
                }
            });
        // Known, or not to be, before a signal held off comes: a handler
        // run here then waits for no program.
        STARTING.fetch_sub(1, Ordering::SeqCst);
        release_signals(held);
        started
    }

    fn hurry_set() -> libc::sigset_t {
        // SAFETY: a zeroed sigset_t is emptied and filled by the calls.
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: `set` is a valid sigset_t.
        unsafe {
            libc::sigemptyset(&mut set);
            for signal in HURRY {
                libc::sigaddset(&mut set, signal);
            }
        }
        set
    }

    /// Hold the hurry signals off this thread; what was held before.
    fn hold_signals() -> libc::sigset_t {
        let set = hurry_set();
        // SAFETY: a zeroed sigset_t is valid for the old mask to be written.
        let mut old: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: both sets are valid.
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old) };
        old
    }

    fn release_signals(old: libc::sigset_t) {
        // SAFETY: restores the mask `hold_signals` saved.
        unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut()) };
    }

    /// In a program started, between fork and exec: the signals held off
    /// the thread that started it are not to be held off the program.
    pub fn unblock_signals() {
        let set = hurry_set();
        // SAFETY: sigprocmask is safe between fork and exec.
        unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
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
            let id = slot.load(Ordering::SeqCst);
            // Not free, nor reserved for a program not started.
            if id > 0 {
                kill(id);
            }
        }
    }
}

#[cfg(not(unix))]
mod group {
    use std::io;
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicBool, Ordering};

    pub struct Known;

    pub fn reserve(stop: &AtomicBool) -> Option<Known> {
        (!stop.load(Ordering::Relaxed)).then_some(Known)
    }

    pub fn start(command: &mut Command, _: &Known) -> io::Result<Child> {
        command.spawn()
    }
}

#[cfg(test)]
#[path = "../tests/process/child.rs"]
mod tests;
