//! Re-export of the kernel event bus.
//!
//! The bus lives in `dsh-kernel` because listeners are fiber effects and
//! isolate/scope filters read `Context`. This crate exists so the spec name
//! `dsh-events` is a real workspace member.

pub use dsh_kernel::{Context, Next, OnOptions, Payload};

#[cfg(test)]
mod tests {
    use dsh_kernel::Context;

    #[tokio::test]
    async fn reexport_waterfall_is_the_kernel_bus() {
        let root = Context::new();
        root.on_waterfall::<i32, _, _>("t", |value, next| async move { next(value).await })
            .unwrap();
        assert_eq!(root.waterfall("t", 4).await, 4);
    }
}
