//! Model-facing `web_search` tool. `web_fetch` is not registered in this phase.

pub mod plugin;
mod search;

#[cfg(test)]
mod phase6_exit;

pub use plugin::{ToolWebConfig, register};
pub use search::{
    WEB_SEARCH_MAX_RESULTS, WebSearchMeta, format_search_output, parse_search_args,
    search_meta_from_result,
};
