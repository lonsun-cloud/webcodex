use super::*;

fn fence(generation: u64) -> ValidationSourceFence {
    ValidationSourceFence {
        epoch: "a".repeat(32),
        generation,
        quiescent: true,
    }
}

#[test]
fn uncrossed_observation_is_not_current_source_proof() {
    let start = fence(1);
    let state = ValidationSourceState::observe(Some(&start), Some(&start));
    assert_eq!(
        state.observed_mutation_fence,
        ObservedMutationFence::Uncrossed
    );
    assert_eq!(state.freshness, ValidationFreshness::Unproven);
    assert!(
        serde_json::from_value::<ValidationSourceState>(serde_json::json!({
            "freshness": "current", "observed_mutation_fence": "uncrossed"
        }))
        .is_err()
    );
}

#[test]
fn generation_crossing_catches_reverted_and_noop_attempts() {
    let state = ValidationSourceState::observe(Some(&fence(2)), Some(&fence(4)));
    assert_eq!(state.freshness, ValidationFreshness::Stale);
    assert_eq!(
        state.observed_mutation_fence,
        ObservedMutationFence::Crossed
    );
}

#[test]
fn missing_epoch_restart_active_writer_and_counter_regression_fail_closed() {
    let start = fence(5);
    let mut other = start.clone();
    other.epoch = "b".repeat(32);
    for now in [
        None,
        Some(other),
        Some(fence(4)),
        Some(ValidationSourceFence {
            quiescent: false,
            ..start.clone()
        }),
    ] {
        let state = ValidationSourceState::observe(Some(&start), now.as_ref());
        assert_eq!(state.freshness, ValidationFreshness::Unproven);
        assert_eq!(
            state.observed_mutation_fence,
            ObservedMutationFence::Unknown
        );
    }
    assert_eq!(
        ValidationSourceState::default().observed_mutation_fence,
        ObservedMutationFence::Unknown
    );
}

#[test]
fn malformed_and_oversized_markers_are_not_evidence() {
    let invalid = ValidationSourceFence {
        epoch: "bad".into(),
        ..fence(0)
    };
    assert!(!invalid.is_valid());
    assert!(
        ValidationSourceState::observe(Some(&invalid), Some(&invalid))
            .start_fence
            .is_none()
    );
    assert!(!fence(MAX_SOURCE_GENERATION + 1).is_valid());
}
