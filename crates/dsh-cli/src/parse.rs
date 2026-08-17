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

/// `dsh web` / `--profile web` invocation after launcher flags.
#[derive(Debug)]
pub struct WebLaunch {
    /// Bind port. `0` asks the OS to assign one. Default `3080`.
    pub port: u16,
    /// `--dist` directory when the flag is present.
    pub dist: Option<PathBuf>,
    /// `--trusted-host` authorities in argv order.
    pub trusted_hosts: Vec<String>,
    /// `--patch` files in argv order.
    pub patches: Vec<PathBuf>,
}

/// Parsed launcher action.
#[derive(Debug)]
pub enum ParsedCli {
    /// `dsh --profile headless …`.
    Headless(HeadlessLaunch),
    /// `dsh web` / `dsh --profile web …`.
    Web(WebLaunch),
}

/// Usage or not-implemented failure (always process exit 2).
#[derive(Debug)]
pub enum CliError {
    /// `plugin` / unimplemented profile.
    NotImplemented {
        /// Verb printed after `dsh: `.
        verb: String,
    },
    /// Missing flags, missing task, or a disallowed `--host`.
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

const DEFAULT_WEB_PORT: u16 = 3080;
const LOOPBACK_HOST: &str = "127.0.0.1";
const HOST_ALL_INTERFACES_SAFETY: &str = "is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead";

#[derive(Parser, Debug)]
#[command(name = "dsh", about = "DeepSeek Harness", no_binary_name = false)]
struct Cli {
    /// Profile bundle. `headless` and `web` are implemented.
    #[arg(long)]
    profile: Option<String>,
    /// Extra YAML patch documents, applied in order.
    #[arg(long, action = ArgAction::Append, value_name = "FILE")]
    patch: Vec<PathBuf>,
    /// Web bind port. Default `3080`. `0` asks the OS to assign one.
    #[arg(long)]
    port: Option<u16>,
    /// Web bind host. Omitted or `127.0.0.1` only.
    #[arg(long)]
    host: Option<String>,
    /// Extra `/api` trust authorities (`host` or `host:port`). Repeatable.
    #[arg(long = "trusted-host", action = ArgAction::Append, value_name = "HOST")]
    trusted_host: Vec<String>,
    /// Web SPA dist directory. Else `DSH_WEB_DIST`, else `apps/web/dist`.
    #[arg(long)]
    dist: Option<PathBuf>,
    /// Task words joined by spaces (headless), or leftover words (web).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    task: Vec<String>,
}

/// Parse `args` including argv[0].
///
/// `dsh web` is an alias for `--profile web`. Neither requires a positional task.
///
/// # Errors
///
/// [`CliError`] for not-implemented verbs, missing `--profile`, unimplemented profiles, a missing headless task, or a disallowed `--host`.
pub fn parse_cli<I, S>(args: I) -> Result<ParsedCli, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args: Vec<String> = args.into_iter().map(Into::into).collect();
    if args.get(1).map(String::as_str) == Some("plugin") {
        return Err(CliError::NotImplemented {
            verb: "plugin".into(),
        });
    }
    if args.get(1).map(String::as_str) == Some("web") {
        args[1] = "--profile".into();
        args.insert(2, "web".into());
    }
    let cli = Cli::try_parse_from(&args).map_err(|error| CliError::Usage {
        message: error.to_string().trim().to_string(),
    })?;
    let Some(profile) = cli.profile.as_deref() else {
        return Err(CliError::Usage {
            message: "required option --profile is missing".into(),
        });
    };
    if profile == "web" {
        return parse_web_launch(cli);
    }
    if profile != "headless" {
        return Err(CliError::NotImplemented {
            verb: profile.to_string(),
        });
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

fn parse_web_launch(cli: Cli) -> Result<ParsedCli, CliError> {
    if let Some(host) = cli.host.as_deref() {
        if host != LOOPBACK_HOST {
            return Err(CliError::Usage {
                message: format!("--host {host} {HOST_ALL_INTERFACES_SAFETY}"),
            });
        }
    }
    Ok(ParsedCli::Web(WebLaunch {
        port: cli.port.unwrap_or(DEFAULT_WEB_PORT),
        dist: cli.dist,
        trusted_hosts: cli.trusted_host,
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
            ParsedCli::Web(_) => panic!("expected headless"),
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
            ParsedCli::Web(_) => panic!("expected headless"),
        }
    }

    #[test]
    fn web_as_argv1_parses_web_launch() {
        let parsed = parse_cli(["dsh", "web", "--port", "0"]).unwrap();
        match parsed {
            ParsedCli::Web(launch) => {
                assert_eq!(launch.port, 0);
                assert!(launch.dist.is_none());
                assert!(launch.trusted_hosts.is_empty());
                assert!(launch.patches.is_empty());
            }
            ParsedCli::Headless(_) => panic!("expected web"),
        }
    }

    #[test]
    fn profile_web_parses_web_launch() {
        let parsed = parse_cli(["dsh", "--profile", "web"]).unwrap();
        match parsed {
            ParsedCli::Web(launch) => {
                assert_eq!(launch.port, 3080);
                assert!(launch.dist.is_none());
                assert!(launch.trusted_hosts.is_empty());
                assert!(launch.patches.is_empty());
            }
            ParsedCli::Headless(_) => panic!("expected web"),
        }
    }

    #[test]
    fn web_host_all_interfaces_is_usage() {
        let err = parse_cli(["dsh", "web", "--host", "0.0.0.0"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_stderr_line().contains(
                "is intentionally not supported yet for safety: it would expose remote code execution to the network; use 127.0.0.1 instead"
            ),
            "{}",
            err.to_stderr_line()
        );
    }

    #[test]
    fn web_host_loopback_parses() {
        let parsed = parse_cli(["dsh", "web", "--host", "127.0.0.1"]).unwrap();
        match parsed {
            ParsedCli::Web(launch) => {
                assert_eq!(launch.port, 3080);
                assert!(launch.trusted_hosts.is_empty());
            }
            ParsedCli::Headless(_) => panic!("expected web"),
        }
    }

    #[test]
    fn web_repeatable_trusted_host_and_patch() {
        let parsed = parse_cli([
            "dsh",
            "web",
            "--trusted-host",
            "harness.example",
            "--trusted-host",
            "192.168.1.5:3080",
            "--patch",
            "a.yml",
            "--dist",
            "/tmp/dist",
        ])
        .unwrap();
        match parsed {
            ParsedCli::Web(launch) => {
                assert_eq!(
                    launch.trusted_hosts,
                    vec![
                        "harness.example".to_string(),
                        "192.168.1.5:3080".to_string()
                    ]
                );
                assert_eq!(launch.patches, vec![std::path::PathBuf::from("a.yml")]);
                assert_eq!(launch.dist, Some(std::path::PathBuf::from("/tmp/dist")));
            }
            ParsedCli::Headless(_) => panic!("expected web"),
        }
    }

    #[test]
    fn plugin_as_argv1_is_not_implemented_exit_2() {
        let err = parse_cli(["dsh", "plugin", "add", "x"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_stderr_line(), "dsh: plugin is not implemented\n");
    }

    #[test]
    fn unknown_profile_is_not_implemented_exit_2() {
        let err = parse_cli(["dsh", "--profile", "acp", "x"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert_eq!(err.to_stderr_line(), "dsh: acp is not implemented\n");
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
