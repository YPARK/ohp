//! The signals that end ohp: Ctrl-C, hang-up and termination.
//!
//! The first asks ohp to stop as quitting does: it is told which, for the
//! app to quit, and typesetting under way is stopped, as a knitr run in a
//! process group of its own that the signal does not reach. A second asks
//! again. A third, for a stop that does not come, ends ohp at once: the
//! programs it runs are killed and the terminal restored, as nothing else
//! will do it then.

use crate::process::child;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::{flag, low_level};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// The terminal's settings before ohp changed them, to restore in a hurry.
static TERMINAL: OnceLock<libc::termios> = OnceLock::new();

/// Leave the alternate screen, and show the cursor.
const LEAVE_SCREEN: &[u8] = b"\x1b[?1049l\x1b[?25h";

/// Have the signals set `signaled` to which came and `stop`, and the third
/// end ohp. Before the terminal is changed, so its settings can be restored.
pub fn install(stop: &Arc<AtomicBool>, signaled: &Arc<AtomicUsize>) -> io::Result<()> {
    // SAFETY: a zeroed termios is valid, and tcgetattr only fills it in.
    let mut terminal: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: `terminal` is a valid termios to write to.
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut terminal) } == 0 {
        let _ = TERMINAL.set(terminal);
    }
    let seen = Arc::new(AtomicUsize::new(0));
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        flag::register(signal, stop.clone())?;
        flag::register_usize(signal, signaled.clone(), signal as usize)?;
        let seen = seen.clone();
        let status = i32::from(crate::shell_status(signal as usize));
        // SAFETY: the action only touches atomics and makes system calls
        // that are safe in a signal handler: killpg, tcsetattr, write and
        // _exit.
        unsafe {
            low_level::register(signal, move || {
                if seen.fetch_add(1, Ordering::SeqCst) >= 2 {
                    child::kill_all();
                    restore_terminal();
                    low_level::exit(status);
                }
            })
        }?;
    }
    Ok(())
}

/// The terminal as it was, without what ratatui would do on its way out.
fn restore_terminal() {
    if let Some(terminal) = TERMINAL.get() {
        // SAFETY: restores settings tcgetattr gave.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, terminal) };
    }
    // SAFETY: writes a static buffer of its own length.
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            LEAVE_SCREEN.as_ptr().cast(),
            LEAVE_SCREEN.len(),
        )
    };
}

#[cfg(test)]
#[path = "../tests/process/signals.rs"]
mod tests;
