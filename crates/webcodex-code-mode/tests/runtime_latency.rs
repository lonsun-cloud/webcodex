//! Explicit synthetic runtime probe, not a model/Runner throughput benchmark.
#![cfg(feature = "v8-runtime")]

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use webcodex_code_mode::{
    execute, CodeModeExecuteRequest, CodeModeHost, CodeModeHostError, CodeModeHostFuture,
    CodeModeToolRequest, CodeModeToolResponse,
};

struct ProbeHost {
    payload: Value,
    delay: Duration,
}

impl CodeModeHost for ProbeHost {
    fn invoke_tool(
        &self,
        _request: CodeModeToolRequest,
    ) -> CodeModeHostFuture<'_, Result<CodeModeToolResponse, CodeModeHostError>> {
        Box::pin(async move {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            Ok(CodeModeToolResponse {
                success: true,
                output: self.payload.clone(),
                error: None,
            })
        })
    }
}

struct Case {
    name: &'static str,
    source: &'static str,
    payload: Value,
    delay_ms: u64,
    calls: usize,
    expected: &'static str,
}

async fn sample(case: &Case, host: Arc<dyn CodeModeHost>) -> u128 {
    let request = CodeModeExecuteRequest {
        source: case.source.to_string(),
        allowed_tools: vec!["probe".to_string()],
        timeout_ms: Some(30_000),
    };
    let started = Instant::now();
    let result = execute(host, request).await.expect(case.name);
    let elapsed_us = started.elapsed().as_micros();
    assert_eq!(result.stats.tool_calls, case.calls, "{}", case.name);
    assert_eq!(result.content, [case.expected], "{}", case.name);
    elapsed_us
}

fn percentile(sorted: &[u128], percent: usize) -> u128 {
    sorted[(sorted.len() * percent).div_ceil(100) - 1]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "manual latency probe; no timing thresholds; run alone with --nocapture"]
async fn runtime_latency_probe() {
    const SAMPLES: usize = 21;
    let cases = [
        Case {
            name: "noop",
            source: "text(1)",
            payload: Value::Null,
            delay_ms: 0,
            calls: 0,
            expected: "1",
        },
        Case {
            name: "eight_immediate_sequential",
            source: "for (let i=0;i<8;i++) await tools.probe({}); text(8)",
            payload: json!({"ok": true}),
            delay_ms: 0,
            calls: 8,
            expected: "8",
        },
        Case {
            name: "eight_immediate_parallel",
            source: "await Promise.all(Array.from({length:8},()=>tools.probe({}))); text(8)",
            payload: json!({"ok": true}),
            delay_ms: 0,
            calls: 8,
            expected: "8",
        },
        Case {
            name: "eight_20ms_sequential",
            source: "for (let i=0;i<8;i++) await tools.probe({}); text(8)",
            payload: json!({"ok": true}),
            delay_ms: 20,
            calls: 8,
            expected: "8",
        },
        Case {
            name: "eight_20ms_parallel",
            source: "await Promise.all(Array.from({length:8},()=>tools.probe({}))); text(8)",
            payload: json!({"ok": true}),
            delay_ms: 20,
            calls: 8,
            expected: "8",
        },
        Case {
            name: "numeric_array_16384",
            source: "const r=await tools.probe({}); text([r.output.length,r.output[16383]])",
            payload: json!((0..16_384).collect::<Vec<_>>()),
            delay_ms: 0,
            calls: 1,
            expected: "[16384,16383]",
        },
        Case {
            name: "search_rows_2048",
            source: "const r=await tools.probe({}); text([r.output.length,r.output[2047].line])",
            payload: Value::Array(
                (0..2048)
                    .map(|line| {
                        json!({
                            "path": format!("src/module_{line}.rs"),
                            "line": line,
                            "text": "pub fn inspect_example() -> bool { true }"
                        })
                    })
                    .collect(),
            ),
            delay_ms: 0,
            calls: 1,
            expected: "[2048,2047]",
        },
    ];
    println!("scope=synthetic_runtime samples={SAMPLES} unit=us; includes host payload cloning; excludes model, ToolRuntime, Session, transport and Runner");
    for case in cases {
        let payload_bytes = serde_json::to_vec(&case.payload).unwrap().len();
        let host: Arc<dyn CodeModeHost> = Arc::new(ProbeHost {
            payload: case.payload.clone(),
            delay: Duration::from_millis(case.delay_ms),
        });
        let first_us = sample(&case, host.clone()).await;
        // Only the first noop sample includes process-global V8 initialization.
        for _ in 0..3 {
            sample(&case, host.clone()).await;
        }
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            samples.push(sample(&case, host.clone()).await);
        }
        samples.sort_unstable();
        println!(
            "case={} first_us={first_us} p50_us={} p95_us={} payload_bytes={payload_bytes}",
            case.name,
            percentile(&samples, 50),
            percentile(&samples, 95)
        );
    }
}
