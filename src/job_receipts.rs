//! Root adapter keeps SQLite out of the authoritative registry crate.
use std::sync::Arc;
use webcodex_runner_registry::{JobReceiptStore, RetainedJobReceipt, RunnerRegistry};

struct SqliteJobReceiptStore(Arc<crate::Database>);

impl std::fmt::Debug for SqliteJobReceiptStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SqliteJobReceiptStore")
    }
}

impl JobReceiptStore for SqliteJobReceiptStore {
    fn upsert(&self, receipt: &RetainedJobReceipt) -> Result<(), String> {
        self.0
            .upsert_job_receipt(receipt, chrono::Utc::now().timestamp())
            .map_err(|_| "Job receipt write failed".to_string())
    }
    fn load(&self, now: i64) -> Result<Vec<RetainedJobReceipt>, String> {
        self.0
            .load_job_receipts(now)
            .map_err(|_| "Job receipt read failed".to_string())
    }
    fn prune(&self, now: i64) -> Result<(), String> {
        self.0
            .prune_job_receipts(now)
            .map(|_| ())
            .map_err(|_| "Job receipt prune failed".to_string())
    }
}

/// Used before production starts accepting any Runner or tool traffic.
#[cfg(test)]
pub(crate) async fn production_registry(db: Arc<crate::Database>) -> RunnerRegistry {
    RunnerRegistry::with_job_receipt_store(
        crate::runner_http::tool_request_trace_telemetry(),
        Arc::new(SqliteJobReceiptStore(db)),
    )
    .await
}

/// Production wiring for E3: terminal Job facts share the authoritative registry
/// transition but are durably matched only after the registry lock is released.
pub(crate) async fn production_registry_with_terminal_attention(
    db: Arc<crate::Database>,
    controller: crate::job_terminal_attention::JobTerminalContinuationController,
) -> RunnerRegistry {
    RunnerRegistry::with_job_receipt_and_terminal_event_sink(
        crate::runner_http::tool_request_trace_telemetry(),
        Arc::new(SqliteJobReceiptStore(db.clone())),
        Arc::new(crate::job_terminal_attention::SqliteJobTerminalEventSink::new(db, controller)),
    )
    .await
}
