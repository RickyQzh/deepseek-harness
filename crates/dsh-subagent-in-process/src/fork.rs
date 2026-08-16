//! In-process fork provider: a child seeded with the parent's completed-turn prefix.

use std::future::Future;
use std::pin::Pin;

use dsh_agent::AgentHandle;
use dsh_kernel::Context;
use dsh_subagent::{
    ContinuableCreateSpec, SubagentCapabilities, SubagentError, SubagentProvider, SubagentResult,
    SubagentStartRequest,
};

use crate::driver::{completed_turn_prefix, start_in_process_run};

/// Registry provider that forks completed parent history into the child.
pub(crate) struct ForkInProcessProvider {
    name: String,
    ctx: Context,
}

impl ForkInProcessProvider {
    pub(crate) fn new(name: String, ctx: Context) -> Self {
        Self { name, ctx }
    }
}

impl SubagentProvider for ForkInProcessProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SubagentCapabilities {
        SubagentCapabilities::all()
    }

    fn inherits_parent_context(&self) -> bool {
        true
    }

    fn start(
        &self,
        request: SubagentStartRequest,
        parent: AgentHandle,
    ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>> {
        let ctx = self.ctx.clone();
        let name = self.name.clone();
        Box::pin(async move { start_in_process_run(&ctx, request, parent, &name, true).await })
    }

    fn prepare_continuable(&self, parent: &AgentHandle) -> ContinuableCreateSpec {
        let seed = completed_turn_prefix(&parent.lock().session);
        ContinuableCreateSpec {
            seed: if seed.is_empty() { None } else { Some(seed) },
        }
    }
}
