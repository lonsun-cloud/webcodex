//! Public runtime coverage for host JSON conversion, including hostile prototypes.
#![cfg(feature = "v8-runtime")]

use serde_json::{json, Value};
use std::sync::Arc;
use webcodex_code_mode::{
    execute, CodeModeExecuteRequest, CodeModeHost, CodeModeHostError, CodeModeHostFuture,
    CodeModeToolRequest, CodeModeToolResponse,
};

struct PayloadHost(Value);

impl CodeModeHost for PayloadHost {
    fn invoke_tool(
        &self,
        _request: CodeModeToolRequest,
    ) -> CodeModeHostFuture<'_, Result<CodeModeToolResponse, CodeModeHostError>> {
        Box::pin(async move {
            Ok(CodeModeToolResponse {
                success: true,
                output: self.0.clone(),
                error: None,
            })
        })
    }
}

async fn converted(payload: Value, source: &str) -> Value {
    let result = execute(
        Arc::new(PayloadHost(payload)),
        CodeModeExecuteRequest {
            source: source.to_string(),
            allowed_tools: vec!["probe".to_string()],
            timeout_ms: Some(5_000),
        },
    )
    .await
    .expect("host JSON conversion must succeed");
    assert_eq!(result.stats.tool_calls, 1);
    assert_eq!(result.content.len(), 1);
    serde_json::from_str(&result.content[0]).unwrap()
}

#[tokio::test]
async fn arrays_preserve_empty_nested_and_mixed_json_values() {
    let payload = json!([
        null, false, true, -42, 1.5, "Unicode: \u{4e2d}\u{6587}\n",
        [], [0, {"__proto__": {"polluted": true}, "value": [2, 3]}]
    ]);
    let actual = converted(
        payload.clone(),
        "const r = await tools.probe({}); text(r.output);",
    )
    .await;
    assert_eq!(actual, payload);
}

#[tokio::test]
async fn array_conversion_bypasses_inherited_index_accessors() {
    let actual = converted(
        json!([7, {"items": [8, 9]}, []]),
        r#"
        Object.defineProperty(Array.prototype, "0", {
            configurable: true,
            get() { throw new Error("inherited index getter invoked"); },
            set() { throw new Error("inherited index setter invoked"); }
        });
        const r = await tools.probe({});
        const descriptor = Object.getOwnPropertyDescriptor(r.output, "0");
        text({
            value: r.output,
            own: Object.prototype.hasOwnProperty.call(r.output, "0"),
            writable: descriptor.writable,
            enumerable: descriptor.enumerable,
            configurable: descriptor.configurable,
            arrayPrototype: Object.getPrototypeOf(r.output) === Array.prototype
        });
        "#,
    )
    .await;
    assert_eq!(
        actual,
        json!({
            "value": [7, {"items": [8, 9]}, []],
            "own": true,
            "writable": true,
            "enumerable": true,
            "configurable": true,
            "arrayPrototype": true
        })
    );
}
