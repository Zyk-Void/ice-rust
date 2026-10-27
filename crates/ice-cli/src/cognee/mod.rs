//! Built-in Cognee memory. Ports the `ice-cognee` TypeScript package
//! (`docs/ice-gap/cognee.md`, COG-001..068).
//!
//! Cognee is an external memory service. icers recalls context for each
//! prompt, captures turns into the session cache, and asks Cognee to
//! improve the cache while idle. Nothing here is required for a session to
//! run: every failure degrades to no memory.

pub mod browse;
pub mod candidates;
pub mod client;
pub mod command;
pub mod config;
pub mod dream;
pub mod evidence;
pub mod helper;
pub mod helper_gate;
pub mod observer;
pub mod policy;
pub mod propose;
pub mod queue;
pub mod runtime;
pub mod text;
