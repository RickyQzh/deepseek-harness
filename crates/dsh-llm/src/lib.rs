//! Provider-neutral LLM stream contract, block assembler, and mock adapter.

mod adapter;
mod assembler;
mod attribution;
mod error;
mod mock;
pub mod plugin;
pub mod replay;
mod types;

pub use adapter::{LlmAdapter, LlmRuntime, PreparedLlmCall};
pub use assembler::BlockAssembler;
pub use attribution::{attribution_headers, user_agent};
pub use error::{
    ABORTED_CODE, ApiKeyRejection, EMPTY_RESPONSE_CODE, INVALID_CREDENTIAL_CODE, LlmError,
    assert_usable_api_key, normalize_api_key,
};
pub use mock::{MockAdapter, MockScript, max_tokens_response, text_response, tool_call_response};
pub use types::{
    GenerateOptions, LlmModelContext, LlmModelReasoningInfo, LlmProviderInfo, LlmPurpose,
    LlmReasoningEffortInfo, LlmResolvedModelInfo, ToolSchema,
};
