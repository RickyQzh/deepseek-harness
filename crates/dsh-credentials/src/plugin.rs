//! Kernel plugin `@deepseek-ai/dsh-credentials`.

use std::sync::Arc;

use dsh_boot::PluginSetup;
use dsh_kernel::KernelError;
use serde_json::Value;

use crate::credentials_from_home;

fn setup_err(message: impl Into<String>) -> KernelError {
    KernelError::SetupFailed(message.into())
}

/// Register the credentials plugin on `registry`.
pub fn register(registry: &mut dsh_boot::PluginRegistry) {
    let setup: PluginSetup = Arc::new(|ctx, _config: Value| {
        Box::pin(async move {
            let home = std::env::var("DSH_HOME").ok();
            let creds = credentials_from_home(home.as_deref())
                .map_err(|error| setup_err(error.to_string()))?;
            ctx.provide("credentials", creds)
                .map_err(|error| setup_err(error.to_string()))?;
            Ok(())
        })
    });
    registry.register(dsh_boot::PLUGIN_CREDENTIALS, setup);
}

#[cfg(test)]
mod tests {
    use super::register;
    use crate::{CredentialProvider, LayeredCredentials, credentials_from_home};
    use dsh_boot::{PluginRegistry, boot_yaml, process_interpolate_env};
    use dsh_kernel::Context;

    #[tokio::test]
    async fn provides_credentials_service() {
        let ctx = Context::new();
        let mut registry = PluginRegistry::new();
        register(&mut registry);
        boot_yaml(
            &ctx,
            "- name: '@deepseek-ai/dsh-credentials'\n",
            &[],
            &registry,
            &process_interpolate_env(),
        )
        .await
        .expect("boot credentials");
        let creds = ctx
            .inject::<LayeredCredentials>("credentials")
            .await
            .expect("credentials");
        let r = crate::credential_ref("TASK85_PLUGIN_ABSENT").unwrap();
        let info = creds.describe(&r).unwrap();
        assert!(!info.configured);
    }

    #[test]
    fn without_home_set_ref_uses_in_process_file_layer() {
        let creds = credentials_from_home(None).unwrap();
        creds.set_ref("TASK85_MEM_ONLY", "x").unwrap();
        let map = creds.describe_refs(&["TASK85_MEM_ONLY".into()]).unwrap();
        assert!(map["TASK85_MEM_ONLY"].configured());
        assert_eq!(map["TASK85_MEM_ONLY"].source(), Some("file"));
    }
}
