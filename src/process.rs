//! ohp's own process and those it starts: programs run so they can be ended
//! with all they started, and the signals that end ohp.

pub mod child;
#[cfg(unix)]
pub mod signals;
