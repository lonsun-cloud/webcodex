use serde_json::Value;
use webcodex_core::runner_protocol::RunnerConfigOperationResponse;

use super::common::wrapped_typed_output_schema;

pub fn output_schema_for_tool(name: &str) -> Option<Value> {
    matches!(name, "runner_config_check" | "runner_config_reload")
        .then(|| wrapped_typed_output_schema::<RunnerConfigOperationResponse>(Vec::new()))
}
