use super::*;
use webcodex_core::validation_source::{ValidationSourceFence, ValidationSourceState};

fn marker(generation: u64) -> ValidationSourceFence {
    ValidationSourceFence {
        epoch: "a".repeat(32),
        generation,
        quiescent: true,
    }
}

fn check(source: Value, success: bool) -> Value {
    let store = SessionStore::default();
    let session = store.start_session(Some("agent:eval:demo".into()), None);
    record_finished_tool(
        &store,
        &session.session_id,
        "cargo_check",
        json!({"project":"agent:eval:demo"}),
        success,
        json!({"exit_code": if success {0} else {101}, "stdout_tail":"", "stderr_tail":"",
            "execution_state":"completed", "source_state": source}),
    );
    let summary = store.summary(&session.session_id, Some(50)).unwrap();
    let current = current_validation_evidence_for_session(&summary, 50);
    assert_ne!(current.current_validation["status"], "passed");
    assert_eq!(
        current
            .current_validation
            .get("successes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        0
    );
    validation_summary_for_session(&summary)
}

#[test]
fn evidence_source_missing_legacy_and_uncrossed_are_never_current_success() {
    for source in [
        Value::Null,
        json!(ValidationSourceState::default()),
        json!(ValidationSourceState::observe(
            Some(&marker(1)),
            Some(&marker(1))
        )),
    ] {
        let result = check(source, true);
        assert_eq!(
            result["status"], "passed",
            "historical execution fact stays true"
        );
        assert_eq!(result["latest"]["validation_passed"], true);
        assert_eq!(result["latest"]["source_state"]["freshness"], "unproven");
        assert_eq!(result["current_evidence"]["status"], "unproven");
        assert_eq!(
            result["current_evidence"]["reason"],
            "validation_source_unproven"
        );
        assert_eq!(result["current_evidence"]["successes"], 0);
    }
}

#[test]
fn evidence_source_crossing_rejects_success_without_any_session_mutation_event() {
    let result = check(
        json!(ValidationSourceState::observe(
            Some(&marker(1)),
            Some(&marker(3))
        )),
        true,
    );
    assert_eq!(result["status"], "passed");
    assert_eq!(result["latest"]["source_state"]["freshness"], "stale");
    assert_eq!(result["current_evidence"]["status"], "stale");
    assert_eq!(
        result["current_evidence"]["reason"],
        "validation_source_fence_crossed"
    );
}

#[test]
fn evidence_source_spoofed_current_or_malformed_marker_fails_closed() {
    for source in [
        json!({"freshness":"current","observed_mutation_fence":"uncrossed"}),
        json!({"freshness":"unproven","observed_mutation_fence":"uncrossed",
            "start_fence":{"epoch":"not-a-marker","generation":0,"quiescent":true}}),
    ] {
        let result = check(source, true);
        assert_eq!(result["current_evidence"]["status"], "unproven");
        assert_eq!(
            result["latest"]["source_state"]["observed_mutation_fence"],
            "unknown"
        );
    }
}

#[test]
fn evidence_source_unproven_does_not_turn_terminal_failure_into_unknown_outcome() {
    let result = check(json!(ValidationSourceState::default()), false);
    assert_eq!(result["latest"]["validation_passed"], false);
    assert_eq!(
        result["latest"]["failure_class"],
        "execution_or_correctness"
    );
    assert_eq!(result["current_evidence"]["status"], "failed");
}
