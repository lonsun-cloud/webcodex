//! Recorder-only metadata extraction around canonical tool request parsing.

use serde_json::Value;
use webcodex_tool_contracts::ToolCall;
use webcodex_workflow_session::ToolCallRecorderMetadata;

/// Parse one public/model request while retaining recorder-only expectation metadata.
/// Business arguments are parsed by the canonical ToolCall contract; wrapper metadata never becomes
/// business input or part of the generated request schema.
pub fn parse_tool_call_with_recorder_metadata(
    name: &str,
    arguments: Value,
) -> Result<(ToolCall, ToolCallRecorderMetadata), String> {
    let recorder_metadata = ToolCallRecorderMetadata::from_business_arguments(&arguments);
    let call = ToolCall::from_tool_name(name, arguments)?;
    Ok((call, recorder_metadata))
}
