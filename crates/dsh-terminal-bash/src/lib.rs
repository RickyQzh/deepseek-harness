//! Interactive bash PTY backend for owner-scoped terminal sessions.

mod backend;
mod config;
mod plugin;
mod sanitize;
mod session;

pub use plugin::register;
