//! MCP stdio byte streams. Child-process spawn is not implemented here.

use tokio::io::{AsyncRead, AsyncWrite};

use crate::client::McpSession;

impl McpSession {
    /// Bind an MCP session to already-open server stdout and stdin streams.
    ///
    /// Does not spawn a child. `server_stdout` is the MCP read stream;
    /// `server_stdin` is the MCP write stream.
    ///
    /// # Parameters
    ///
    /// * `server_stdout` - Bytes from the server (child stdout or a duplex end).
    /// * `server_stdin` - Bytes to the server (child stdin or a duplex end).
    ///
    /// # Returns
    ///
    /// A session that speaks Content-Length JSON-RPC on those streams.
    #[must_use]
    pub fn from_stdio<R, W>(server_stdout: R, server_stdin: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::new(server_stdout, server_stdin)
    }
}
