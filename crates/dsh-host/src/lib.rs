//! GUI host `/api` trust fence and privileged-method set.
//!
//! The axum listener lands in later tasks. This crate does not listen and does not serialize RPC.

mod trust;

pub use trust::{
    TrustError, assert_trusted_authority, is_loopback_hostname, is_privileged_method,
    is_trusted_api_request, privileged_requires_loopback,
};
