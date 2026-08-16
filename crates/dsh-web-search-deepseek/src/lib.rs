//! DeepSeek Anthropic-compatible `web_search_20250305` search provider.

pub mod plugin;
mod provider;

pub use plugin::register;
pub use provider::{
    DEEPSEEK_DEFAULT_API_VERSION, DEEPSEEK_DEFAULT_BASE_URL, DEEPSEEK_DEFAULT_MAX_TOKENS,
    DEEPSEEK_DEFAULT_MAX_USES, DEEPSEEK_DEFAULT_MODEL, DEEPSEEK_PROVIDER_ID,
    DeepSeekSearchProvider, DeepSeekSearchProviderOptions, citation_snippets, deepseek_search_body,
    map_anthropic_response, resolve_base_url, search_base_url_from_env,
};
