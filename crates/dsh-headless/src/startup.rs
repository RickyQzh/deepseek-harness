//! Kernel plugin `headless-startup`.

use std::sync::Arc;

use dsh_boot::{PLUGIN_HEADLESS_STARTUP, PluginRegistry, PluginSetup};
use dsh_kernel::KernelError;

use crate::error::HeadlessError;
use crate::io::{CmdlineArgs, HeadlessStartup};

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Parse the inner argv into `headlessStartup`.
pub fn register(registry: &mut PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, config| {
        Box::pin(async move {
            let args = ctx.inject::<CmdlineArgs>("cmdlineArgs").await?;
            let task = args.get().join(" ");
            if task.trim().is_empty() {
                return Err(setup_err(HeadlessError::MissingTask.to_string()));
            }
            let resume = config
                .get("resumeSessionId")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            ctx.provide(
                "headlessStartup",
                HeadlessStartup::new(task).with_resume_session_id(resume),
            )
            .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(PLUGIN_HEADLESS_STARTUP, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::{CmdlineArgs, HeadlessStartup};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn resume_session_id_reads_optional_yaml() {
        let ctx = Context::new();
        ctx.provide(
            "cmdlineArgs",
            CmdlineArgs::new(vec![
                "Acknowledge the current workspace instruction.".into(),
            ]),
        )
        .unwrap();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: headless-startup\n  config:\n    resumeSessionId: workspace-context-resume\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .unwrap();
        let startup = ctx.get::<HeadlessStartup>("headlessStartup").unwrap();
        assert_eq!(
            startup.task(),
            "Acknowledge the current workspace instruction."
        );
        assert_eq!(
            startup.resume_session_id(),
            Some("workspace-context-resume")
        );
    }
}
