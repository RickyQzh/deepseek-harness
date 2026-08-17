//! GUI four-quadrant RPC envelopes (`RpcMessage`, `RpcResult`, `RpcReceipt`).
//!
//! This crate is the GUI wire, not SDK JSON-RPC. Details stay [`serde_json::Value`].

mod error;
mod message;

pub use error::{RpcError, RpcErrorCode};
pub use message::{ReceiptReject, RpcId, RpcMessage, RpcReceipt, RpcResult};
