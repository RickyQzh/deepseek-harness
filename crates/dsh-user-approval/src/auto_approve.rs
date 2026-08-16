//! Terminal `approval/request` listener that grants [`ApprovalOutcome::AllowedOnce`].

use dsh_kernel::{Context, KernelError};
use dsh_tools::ApprovalOutcome;

use crate::EVENT_APPROVAL_REQUEST;

/// Register a waterfall listener that returns [`ApprovalOutcome::AllowedOnce`] without `next()`.
///
/// # Errors
///
/// [`KernelError::InactiveEffect`] when this fiber cannot register effects.
pub fn install(ctx: &Context) -> Result<(), KernelError> {
    ctx.on_waterfall::<ApprovalOutcome, _, _>(EVENT_APPROVAL_REQUEST, |_outcome, _next| async {
        ApprovalOutcome::AllowedOnce
    })?;
    Ok(())
}
