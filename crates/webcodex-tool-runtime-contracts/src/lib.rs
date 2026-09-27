//! Transport-neutral Tool Runtime result contracts, recorder metadata, and audit-safe projections.
//!
//! Canonical request/catalog/schema/policy ownership remains in `webcodex-tool-contracts`;
//! execution, authorization, Runner dispatch, Store, and HTTP/MCP adapters remain in the root
//! `webcodex` crate.

pub mod recorder_metadata;
pub mod tool_audit;
pub mod tool_result;

pub use recorder_metadata::parse_tool_call_with_recorder_metadata;
pub use tool_audit::ToolCallAuditProjection;
pub use tool_result::*;

#[cfg(test)]
mod recorder_metadata_tests;
#[cfg(test)]
mod tool_audit_integration_tests;
