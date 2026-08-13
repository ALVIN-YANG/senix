//! Temporary semver compatibility layer for Pingora 0.8.1.
//!
//! Pingora's published manifest still requests `prometheus` 0.13, which pulls vulnerable
//! `protobuf` 2.28. The Pingora upstream fix moved to 0.14. This crate exposes that fixed public
//! API under the version range required by the current Pingora release.

pub use prometheus_upstream::*;
