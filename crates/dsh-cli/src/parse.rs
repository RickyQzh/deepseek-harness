//! argv parse for the Phase 5 `dsh` launcher.

use std::path::PathBuf;

use clap::{ArgAction, Parser};

/// Headless invocation after launcher flags.
#[derive(Debug)]
pub struct HeadlessLaunch {
    /// Joined positional task words.
    pub task: String,
    /// `--patch` files in argv order.
    pub patches: Vec<PathBuf>,
}

/// Parsed launcher action.
#[derive(Debug)]
pub enum ParsedCli {
    /// `dsh --profile headless …`.
    Headless(HeadlessLaunch),
}

/// Usage or not-implemented failure (always process exit 2).
#[derive(Debug)]
pub enum CliError {
    /// `web` / `plugin` / unimplemented profile.
    NotImplemented {
        /// Verb printed after `dsh: `.
        verb: String,
    },
    /// Missing flags or missing task.
    Usage {
        /// Message after `dsh: `.
        message: String,
    },
}

impl CliError {
    /// Always 2.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        2
    }

    /// `dsh: {message}\n`
    #[must_use]
    pub fn to_stderr_line(&self) -> String {
        match self {
            Self::NotImplemented { verb } => format!("dsh: {verb} is not implemented\n"),
            Self::Usage { message } => format!("dsh: {message}\n"),
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "dsh", about = "DeepSeek Harness", no_binary_name = false)]
struct Cli {
    /// Profile bundle. Phase 5 implements only `headless`.
    #[arg(long)]
    profile: Option<String>,
    /// Extra YAML patch documents, applied in order.
    #[arg(long, action = ArgAction::Append, value_name = "FILE")]
    patch: Vec<PathBuf>,
    /// Task words joined by spaces.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    task: Vec<String>,
}

/// Parse `args` including argv[0].
///
/// # Errors
///
/// [`CliError`] for not-implemented verbs, missing `--profile`, unimplemented profiles, or a missing task.
pub fn parse_cli<I, S>(args: I) -> Result<ParsedCli, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    if let Some(verb) = args.get(1).map(String::as_str) {
        if verb == "web" || verb == "plugin" {
            return Err(CliError::NotImplemented {
                verb: verb.to_string(),
            });
        }
    }
    let cli = Cli::try_parse_from(&args).map_err(|error| CliError::Usage {
        message: error.to_string().trim().to_string(),
    })?;
    let Some(profile) = cli.profile else {
        return Err(CliError::Usage {
            message: "required option --profile is missing".into(),
        });
    };
    if profile != "headless" {
        return Err(CliError::NotImplemented { verb: profile });
    }
    let task = cli.task.join(" ");
    if task.trim().is_empty() {
        return Err(CliError::Usage {
            message:
                "error: a task is required, for example: dsh --profile headless \"run the tests\""
                    .into(),
        });
    }
    Ok(ParsedCli::Headless(HeadlessLaunch {
        task,
        patches: cli.patch,
    }))
}

#[cfg(test)]
mod tests {
    use super::{ParsedCli, parse_cli};

    #[test]
    fn parse_profile_headless_joins_positional_task() {
        let parsed = parse_cli(["dsh", "--profile", "headless", "run", "the", "tests"]).unwrap();
        match parsed {
            ParsedCli::Headless(launch) => {
                assert_eq!(launch.task, "run the tests");
                assert!(launch.patches.is_empty());
            }
        }
    }

    #[test]
    fn parse_repeatable_patch_flags() {
        let parsed = parse_cli([
            "dsh",
            "--profile",
            "headless",
            "--patch",
            "a.yml",
            "--patch",
            "b.yml",
            "task",
        ])
        .unwrap();
        match parsed {
            ParsedCli::Headless(launch) => {
                assert_eq!(launch.task, "task");
                assert_eq!(
                    launch.patches,
                    vec![
                        std::path::PathBuf::from("a.yml"),
                        std::path::PathBuf::from("b.yml")
                    ]
                );
            }
        }
    }

    #[test]
    fn web_as_argv1_is_not_implemented_exit_2() {
        let err = parse_cli(["dsh", "web"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_stderr_line(), "dsh: web is not implemented\n");
    }

    #[test]
    fn plugin_as_argv1_is_not_implemented_exit_2() {
        let err = parse_cli(["dsh", "plugin", "add", "x"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_stderr_line(), "dsh: plugin is not implemented\n");
    }

    #[test]
    fn unknown_profile_is_not_implemented_exit_2() {
        let err = parse_cli(["dsh", "--profile", "web", "x"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_stderr_line(), "dsh: web is not implemented\n");
    }

    #[test]
    fn headless_requires_positional_task() {
        let err = parse_cli(["dsh", "--profile", "headless"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(
            err.to_stderr_line(),
            "dsh: error: a task is required, for example: dsh --profile headless \"run the tests\"\n"
        );
    }

    #[test]
    fn whitespace_only_task_is_rejected() {
        let err = parse_cli(["dsh", "--profile", "headless", "  "]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_stderr_line().contains("a task is required"));
    }

    #[test]
    fn profile_flag_is_required() {
        let err = parse_cli(["dsh", "just-a-task"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_stderr_line().contains("profile"),
            "{}",
            err.to_stderr_line()
        );
    }
}
