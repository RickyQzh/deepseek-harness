//! Interactive bash PTY backend for owner-scoped terminal sessions.

mod backend;
mod config;
mod plugin;
mod sanitize;
mod session;

#[cfg(test)]
mod phase8_pty_exit;

pub use plugin::register;
