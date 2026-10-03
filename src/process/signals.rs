//! The signals that end ohp: Ctrl-C, hang-up and termination.
//!
//! The first asks ohp to stop as quitting does: it is told which, for the
//! app to quit, and typesetting under way is stopped, as a knitr run in a
//! session of its own that the signal does not reach. A second asks again.
//! A third, for a stop that does not come, ends ohp at once: the programs
//! it runs are killed and the terminal restored, as nothing else will do it
//! then.

use crate::process::child;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::{flag, low_level};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// The terminal, as a file kept open, and its settings before ohp changed
/// them, to restore in a hurry. Read from `/dev/tty`, where crossterm sets
/// raw mode when the input is not the terminal.
static TERMINAL: OnceLock<(libc::c_int, libc::termios)> = OnceLock::new();

/// Leave the alternate screen, and show the cursor.
const LEAVE_SCREEN: &[u8] = b"\x1b[?1049l\x1b[?25h";

/// Signals seen before the one that ends ohp at once.
const PATIENCE: usize = 2;

/// Have the signals set `signaled` to which came and `stop`, and the third
/// end ohp. Before the terminal is changed, so its settings can be restored.
pub fn install(stop: &Arc<AtomicBool>, signaled: &Arc<AtomicUsize>) -> io::Result<()> {
    save_terminal();
    let seen = Arc::new(AtomicUsize::new(0));
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        flag::register(signal, stop.clone())?;
        flag::register_usize(signal, signaled.clone(), signal as usize)?;
        let seen = seen.clone();
        let status = i32::from(crate::shell_status(signal as usize));
        // SAFETY: the action only touches atomics and makes system calls
        // that are safe in a signal handler: nanosleep, killpg, tcsetattr,
        // write and _exit.
        unsafe {
            low_level::register(signal, move || {
                if out_of_patience(&seen) {
                    child::kill_all();
                    restore_terminal();
                    low_level::exit(status);
                }
            })
        }?;
    }
    Ok(())
}

/// Count a signal: whether it is the one to end ohp at once.
fn out_of_patience(seen: &AtomicUsize) -> bool {
    seen.fetch_add(1, Ordering::SeqCst) >= PATIENCE
}

fn save_terminal() {
    // SAFETY: opens a path given as a C string.
    let fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return;
    }
    // SAFETY: a zeroed termios is valid, and tcgetattr only fills it in.
    let mut settings: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: `settings` is a valid termios to write to.
    if unsafe { libc::tcgetattr(fd, &mut settings) } == 0 {
        let _ = TERMINAL.set((fd, settings));
    } else {
        // SAFETY: closes the file just opened.
        unsafe { libc::close(fd) };
    }
}

/// The terminal as it was, without what ratatui would do on its way out:
/// its settings, where they were saved, and the screen it showed, through
/// the terminal or, without one, the output where it is a terminal.
fn restore_terminal() {
    let fd = match TERMINAL.get() {
        Some((fd, settings)) => {
            // SAFETY: restores settings tcgetattr gave, to the terminal
            // they came from.
            unsafe { libc::tcsetattr(*fd, libc::TCSANOW, settings) };
            *fd
        }
        // SAFETY: isatty only asks of a file.
        None if unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1 => libc::STDOUT_FILENO,
        // Output to a file or a pipe has no screen to leave.
        None => return,
    };
    // SAFETY: writes a static buffer of its own length.
    unsafe { libc::write(fd, LEAVE_SCREEN.as_ptr().cast(), LEAVE_SCREEN.len()) };
}

#[cfg(test)]
#[path = "../tests/process/signals.rs"]
mod tests;
