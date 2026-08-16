//! In-process spawn provider: a fresh child with no parent history.

use std::future::Future;
use std::pin::Pin;

use dsh_agent::AgentHandle;
use dsh_kernel::Context;
use dsh_subagent::{
    ContinuableCreateSpec, SubagentCapabilities, SubagentError, SubagentProvider, SubagentResult,
    SubagentStartRequest,
};

use crate::driver::start_in_process_run;

/// Registry provider that starts a fresh in-process child.
pub(crate) struct SpawnInProcessProvider {
    name: String,
    ctx: Context,
}

impl SpawnInProcessProvider {
    pub(crate) fn new(name: String, ctx: Context) -> Self {
        Self { name, ctx }
    }
}

impl SubagentProvider for SpawnInProcessProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> SubagentCapabilities {
        SubagentCapabilities::all()
    }

    fn inherits_parent_context(&self) -> bool {
        false
    }

    fn start(
        &self,
        request: SubagentStartRequest,
        parent: AgentHandle,
    ) -> Pin<Box<dyn Future<Output = Result<SubagentResult, SubagentError>> + Send + '_>> {
        let ctx = self.ctx.clone();
        let name = self.name.clone();
        Box::pin(async move { start_in_process_run(&ctx, request, parent, &name, false).await })
    }

    fn prepare_continuable(&self, _parent: &AgentHandle) -> ContinuableCreateSpec {
        ContinuableCreateSpec { seed: None }
    }
}
