//! What Supraman's gpui binaries run on that is not UI: wake-ups, the UI-thread stall watch, the frame probe and work handed off the UI thread.

pub mod frames;
pub mod nudge;
mod off_thread;
pub mod stall;

pub use off_thread::{open_off_thread, save_off_thread};
