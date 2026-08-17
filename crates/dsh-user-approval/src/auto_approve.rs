//! Terminal `approval/request` listener that grants [`ApprovalOutcome::AllowedOnce`].

use dsh_kernel::{Context, KernelError};
use dsh_tools::ApprovalOutcome;

use crate::{ApprovalQuestion, EVENT_APPROVAL_REQUEST};

/// Register a waterfall listener that returns [`ApprovalOutcome::AllowedOnce`] without `next()`.
///
/// # Errors
///
/// [`KernelError::InactiveEffect`] when this fiber cannot register effects.
pub fn install(ctx: &Context) -> Result<(), KernelError> {
    ctx.on_waterfall::<ApprovalQuestion, _, _>(
        EVENT_APPROVAL_REQUEST,
        |question, _next| async move { question.with_outcome(ApprovalOutcome::AllowedOnce) },
    )?;
    Ok(())
}
