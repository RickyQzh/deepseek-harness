//! YAML plugins `@deepseek-ai/dsh-user-approval` and `headless-auto-approve`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_HEADLESS_AUTO_APPROVE, PLUGIN_USER_APPROVAL, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::{ApprovalPolicy, ApprovalService};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

fn policy_from_config(config: &Value) -> Result<ApprovalPolicy, KernelError> {
    match config.get("policy") {
        None | Some(Value::Null) => Ok(ApprovalPolicy::Ask),
        Some(Value::String(value)) if value == "ask" => Ok(ApprovalPolicy::Ask),
        Some(Value::String(value)) if value == "never" => Ok(ApprovalPolicy::Never),
        Some(other) => Err(setup_err(format!(
            "approval policy must be \"ask\" or \"never\", got {other}"
        ))),
    }
}

/// Provide `approval` from YAML `{ policy?: "ask"|"never" }` (default `"ask"`).
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config: Value| {
        Box::pin(async move {
            let policy = policy_from_config(&config)?;
            ctx.provide("approval", ApprovalService::new(ctx.clone(), policy))
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_USER_APPROVAL, setup);
}

/// Register YAML `headless-auto-approve` as a terminal AllowedOnce answerer.
pub fn register_auto_approve(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config: Value| {
        Box::pin(async move {
            crate::auto_approve::install(&ctx)?;
            Ok(())
        })
    });
    registry.register(PLUGIN_HEADLESS_AUTO_APPROVE, setup);
}
