//! In-process slash-command registry: parse, list, register, execute.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;

/// Async handler invoked with one parsed command line.
pub type CommandHandler =
    Arc<dyn Fn(ParsedCommand) -> BoxFuture<'static, CommandResult> + Send + Sync>;

/// Syntactically valid slash command before registry lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedCommand {
    name: String,
    raw_input: String,
}

impl ParsedCommand {
    /// Lowercase command name without the leading slash.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Exact text following the command name, including separator whitespace.
    #[must_use]
    pub fn raw_input(&self) -> &str {
        &self.raw_input
    }
}

/// Handler-free command metadata for discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDescriptor {
    name: String,
    description: String,
    input_hint: Option<String>,
}

impl CommandDescriptor {
    /// Lowercase command name without the leading slash.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human-readable summary used in discovery UI.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Optional free-form input hint advertised to capable clients.
    #[must_use]
    pub fn input_hint(&self) -> Option<&str> {
        self.input_hint.as_deref()
    }
}

/// Plugin-owned command registration.
pub struct CommandDefinition {
    name: String,
    description: String,
    input_hint: Option<String>,
    handler: CommandHandler,
}

impl CommandDefinition {
    /// Build a definition. `name` is the slash-free lowercase token.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: CommandHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_hint: None,
            handler,
        }
    }

    /// Advertise an input hint on this definition.
    #[must_use]
    pub fn with_input_hint(mut self, hint: impl Into<String>) -> Self {
        self.input_hint = Some(hint.into());
        self
    }
}

/// Settled handler outcome plus the pairing id for this execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandExecution {
    command_id: String,
    result: CommandResult,
}

impl CommandExecution {
    /// Pairing id minted for this execution (`cmd-{pid}-{nanos}`).
    #[must_use]
    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    /// Handler outcome.
    #[must_use]
    pub fn result(&self) -> &CommandResult {
        &self.result
    }
}

/// Normalized handler result rendered by the dispatching UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandResult {
    /// Handler completed without reporting failure.
    Success {
        /// Optional text for the dispatching UI.
        text: Option<String>,
    },
    /// Handler reported failure.
    Error {
        /// Failure text shown by the dispatching UI.
        text: String,
    },
}

struct Registered {
    descriptor: CommandDescriptor,
    handler: CommandHandler,
}

struct DisposeInner {
    commands: Arc<Mutex<HashMap<String, Registered>>>,
    name: String,
}

/// Unregisters one command. [`Drop`] unregisters once.
#[must_use = "dropping Dispose unregisters the command"]
pub struct Dispose {
    inner: Option<DisposeInner>,
}

impl fmt::Debug for Dispose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispose")
            .field(
                "name",
                &self.inner.as_ref().map(|inner| inner.name.as_str()),
            )
            .finish()
    }
}

impl Dispose {
    /// Unregister this command. Equivalent to dropping `self`.
    pub fn dispose(mut self) {
        self.unregister();
    }

    fn unregister(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut guard = inner.commands.lock().expect("command registry lock");
            guard.remove(&inner.name);
        }
    }
}

impl Drop for Dispose {
    fn drop(&mut self) {
        self.unregister();
    }
}

/// Duplicate command name.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RegisterError {
    /// A command with this name is already registered.
    #[error("command is already registered")]
    Duplicate,
}

impl RegisterError {
    /// Stable error token for this failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Duplicate => "duplicate-command",
        }
    }
}

/// In-process slash-command registry. Product commands are not registered here.
#[derive(Clone)]
pub struct CommandRegistry {
    commands: Arc<Mutex<HashMap<String, Registered>>>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Insert `def`. Duplicate names fail.
    ///
    /// # Errors
    ///
    /// [`RegisterError::Duplicate`] when `def`'s name is already registered.
    pub fn register(&self, def: CommandDefinition) -> Result<Dispose, RegisterError> {
        let mut guard = self.commands.lock().expect("command registry lock");
        if guard.contains_key(&def.name) {
            return Err(RegisterError::Duplicate);
        }
        let name = def.name.clone();
        guard.insert(
            name.clone(),
            Registered {
                descriptor: CommandDescriptor {
                    name: def.name,
                    description: def.description,
                    input_hint: def.input_hint,
                },
                handler: def.handler,
            },
        );
        drop(guard);
        Ok(Dispose {
            inner: Some(DisposeInner {
                commands: Arc::clone(&self.commands),
                name,
            }),
        })
    }

    /// Name-sorted handler-free descriptors.
    #[must_use]
    pub fn list(&self) -> Vec<CommandDescriptor> {
        let guard = self.commands.lock().expect("command registry lock");
        let mut descriptors: Vec<CommandDescriptor> = guard
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    /// Parse `line` without registry lookup.
    ///
    /// The slash must be at byte zero. The name is `[a-z][a-z0-9_-]*` and must
    /// be followed by end-of-string or ASCII space, tab, CR, or LF. The matched
    /// prefix excludes that separator, so `/foo bar` keeps `raw_input` ` bar`.
    #[must_use]
    pub fn parse(line: &str) -> Option<ParsedCommand> {
        let bytes = line.as_bytes();
        if bytes.first().copied() != Some(b'/') {
            return None;
        }
        let rest = bytes.get(1..)?;
        let first = rest.first().copied()?;
        if !first.is_ascii_lowercase() {
            return None;
        }
        let mut name_len = 1;
        while name_len < rest.len() {
            let byte = rest[name_len];
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-' {
                name_len += 1;
            } else {
                break;
            }
        }
        if name_len < rest.len() {
            match rest[name_len] {
                b'\t' | b'\n' | b'\r' | b' ' => {}
                _ => return None,
            }
        }
        let matched_len = 1 + name_len;
        Some(ParsedCommand {
            name: line[1..matched_len].to_string(),
            raw_input: line[matched_len..].to_string(),
        })
    }

    /// Parse and run a registered command. Syntax or name misses return `None` and log nothing.
    pub async fn execute(&self, line: &str) -> Option<CommandExecution> {
        let parsed = Self::parse(line)?;
        let handler = {
            let guard = self.commands.lock().expect("command registry lock");
            guard.get(&parsed.name)?.handler.clone()
        };
        let result = handler(parsed).await;
        Some(CommandExecution {
            command_id: mint_command_id(),
            result,
        })
    }
}

fn mint_command_id() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("cmd-{pid}-{nanos}")
}

#[cfg(test)]
mod tests {
    use super::{CommandDefinition, CommandHandler, CommandRegistry, CommandResult, RegisterError};
    use std::sync::Arc;

    fn stub_handler() -> CommandHandler {
        Arc::new(|_parsed| Box::pin(async { CommandResult::Success { text: None } }))
    }

    #[test]
    fn parse_foo_bar_keeps_leading_space() {
        let parsed = CommandRegistry::parse("/foo bar").unwrap();
        assert_eq!(parsed.name(), "foo");
        assert_eq!(parsed.raw_input(), " bar");
    }

    #[test]
    fn list_is_name_sorted() {
        let reg = CommandRegistry::new();
        let _b = reg
            .register(CommandDefinition::new("b", "B", stub_handler()))
            .expect("register b");
        let _a = reg
            .register(CommandDefinition::new("a", "A", stub_handler()))
            .expect("register a");
        let names: Vec<String> = reg
            .list()
            .into_iter()
            .map(|descriptor| descriptor.name().to_string())
            .collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[tokio::test]
    async fn execute_unknown_is_none() {
        let reg = CommandRegistry::new();
        assert!(reg.execute("/nope").await.is_none());
    }

    #[test]
    fn duplicate_register_fails() {
        let reg = CommandRegistry::new();
        let _first = reg
            .register(CommandDefinition::new("dup", "D", stub_handler()))
            .expect("first");
        let err = reg
            .register(CommandDefinition::new("dup", "D2", stub_handler()))
            .expect_err("duplicate");
        assert_eq!(err, RegisterError::Duplicate);
    }
}
