//! Validation source evidence, deliberately weaker than an immutable snapshot.
//!
//! A fence observes potential mutation dispatches in one canonical Project on one
//! live Control runtime. It does not cover arbitrary filesystem/process writes,
//! another Control, Project aliases of the same root, or Runner retargeting.
//! Consequently v1 has NO `current`/`fresh` state, even with an uncrossed fence.

use serde::{Deserialize, Serialize};

pub const MAX_SOURCE_GENERATION: u64 = (1u64 << 53) - 1;

/// Server-minted observation marker, not authority, retry, or execution identity.
/// The random epoch belongs to one registry entry for one resolved Project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationSourceFence {
    pub epoch: String,
    pub generation: u64,
    pub quiescent: bool,
}

impl ValidationSourceFence {
    pub fn is_valid(&self) -> bool {
        self.epoch.len() == 32
            && self.epoch.bytes().all(|b| b.is_ascii_hexdigit())
            && self.generation <= MAX_SOURCE_GENERATION
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedMutationFence {
    Uncrossed,
    Crossed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationFreshness {
    Unproven,
    Stale,
}

/// Execution pass/fail is separate. This projection NEVER certifies current
/// source. Missing/legacy evidence and loss of an epoch are explicitly unproven.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationSourceState {
    pub freshness: ValidationFreshness,
    pub observed_mutation_fence: ObservedMutationFence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_fence: Option<ValidationSourceFence>,
}

impl Default for ValidationSourceState {
    fn default() -> Self {
        Self::observe(None, None)
    }
}

impl ValidationSourceState {
    pub fn observe(
        start: Option<&ValidationSourceFence>,
        now: Option<&ValidationSourceFence>,
    ) -> Self {
        let start = start.filter(|fence| fence.is_valid());
        let observation = match (start, now.filter(|fence| fence.is_valid())) {
            (Some(start), Some(now)) if start.epoch == now.epoch => {
                if now.generation > start.generation {
                    ObservedMutationFence::Crossed
                } else if now.generation == start.generation && start.quiescent && now.quiescent {
                    ObservedMutationFence::Uncrossed
                } else {
                    ObservedMutationFence::Unknown
                }
            }
            _ => ObservedMutationFence::Unknown,
        };
        Self {
            freshness: if observation == ObservedMutationFence::Crossed {
                ValidationFreshness::Stale
            } else {
                ValidationFreshness::Unproven
            },
            observed_mutation_fence: observation,
            start_fence: start.cloned(),
        }
    }
}

#[cfg(test)]
#[path = "validation_source_tests.rs"]
mod tests;
