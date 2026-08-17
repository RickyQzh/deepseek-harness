//! Six model-facing `terminal_*` tools, render, and `tool:pty` guidance.

pub mod plugin;
mod render;
mod tools;

pub use plugin::register;
pub use render::{
    PresentCall, RenderedReadResult, RenderedSendRead, RenderedSendResult, RenderedSessionSnapshot,
    RenderedSessionStatus, RenderedSpawnResult, bound_terminal_text, present_close, present_list,
    present_open, present_read, present_send, present_signal, render_list, render_read,
    render_send, render_send_read, render_spawn,
};
