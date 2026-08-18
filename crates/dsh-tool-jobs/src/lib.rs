//! Model-facing `job_output`, `job_list`, and `job_kill` over `jobs`.

pub mod plugin;

pub use plugin::{CompletionDelivery, ToolJobsConfig, register};

use dsh_jobs::JobStatus;

/// Bracketed status line with optional producer detail.
#[must_use]
pub fn status_line(status: JobStatus, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("[status: {status}, {detail}]"),
        None => format!("[status: {status}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::status_line;
    use dsh_jobs::JobStatus;

    #[test]
    fn status_line_omits_detail_when_absent() {
        assert_eq!(status_line(JobStatus::Running, None), "[status: running]");
        assert_eq!(
            status_line(JobStatus::Completed, Some("exit code: 0")),
            "[status: completed, exit code: 0]"
        );
    }
}
