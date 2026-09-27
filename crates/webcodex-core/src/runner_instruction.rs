use crate::project_instructions::{
    InstructionSourceScope, ProjectInstructionFile, MAX_LINES_PER_FILE, MAX_TOTAL_CHARS,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const RUNNER_INSTRUCTION_REQUEST_KIND: &str = "runner_instruction";
pub const RUNNER_INSTRUCTION_REQUEST_MAX_BYTES: usize = 128;
pub const RUNNER_INSTRUCTION_RESPONSE_MAX_BYTES: usize = 192 * 1024;
pub const RUNNER_INSTRUCTION_RESPONSE_MAX_FILES: usize = 16;
pub const RUNNER_INSTRUCTION_RESPONSE_FORMAT: &str = "webcodex.runner_instruction_snapshot.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerInstructionAction {
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerInstructionRequest {
    pub action: RunnerInstructionAction,
}

impl RunnerInstructionRequest {
    pub fn snapshot() -> Self {
        Self {
            action: RunnerInstructionAction::Snapshot,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self.action {
            RunnerInstructionAction::Snapshot => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerInstructionSnapshotResponse {
    pub format: String,
    pub generation: u64,
    pub scan_complete: bool,
    pub files: Vec<ProjectInstructionFile>,
}

impl RunnerInstructionSnapshotResponse {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.format != RUNNER_INSTRUCTION_RESPONSE_FORMAT {
            return Err("unexpected instruction response format");
        }
        if self.generation == 0 {
            return Err("instruction config generation must be positive");
        }
        if self.files.len() > RUNNER_INSTRUCTION_RESPONSE_MAX_FILES {
            return Err("Runner instruction response contains too many sources");
        }
        let mut sources = HashSet::with_capacity(self.files.len());
        for file in &self.files {
            if file.source_scope != InstructionSourceScope::Runner {
                return Err("Runner instruction response contains non-Runner source");
            }
            if !valid_runner_logical_source(&file.path)
                || !sources.insert(file.path.split('/').nth(1))
            {
                return Err("Runner instruction response contains invalid logical source");
            }
            if file.read_more.is_some() {
                return Err("Runner instruction response must not expose read_more");
            }
            if file.start_line != 1
                || file.limit != MAX_LINES_PER_FILE
                || file.chars != file.content.chars().count()
                || file.chars > MAX_TOTAL_CHARS
                || file.total_lines < file.content.lines().count()
                || !is_lower_hex_sha256(&file.fingerprint)
            {
                return Err("Runner instruction response contains invalid bounded source metadata");
            }
        }
        Ok(())
    }

    /// Bind the Runner-reported full-source fingerprint to the exact bounded
    /// source body observed by Control. The upstream fingerprint retains
    /// sensitivity to content beyond the projection bound, while the second
    /// domain-separated digest prevents a stale or malformed Runner response
    /// from reusing an old fingerprint for different visible content.
    pub fn bind_visible_fingerprints(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        for file in &mut self.files {
            let upstream = file.fingerprint.clone();
            let mut hasher = Sha256::new();
            hasher.update(b"webcodex.runner-instruction-visible-binding.v1\0");
            for value in [
                file.path.as_bytes(),
                upstream.as_bytes(),
                file.content.as_bytes(),
            ] {
                hasher.update((value.len() as u64).to_be_bytes());
                hasher.update(value);
            }
            hasher.update((file.total_lines as u64).to_be_bytes());
            hasher.update([u8::from(file.truncated)]);
            file.fingerprint = format!("{:x}", hasher.finalize());
        }
        Ok(())
    }
}

fn valid_runner_logical_source(path: &str) -> bool {
    if path.contains('\\') || path.contains('\0') {
        return false;
    }
    let mut parts = path.split('/');
    let scope = parts.next();
    let index = parts.next();
    let basename = parts.next();
    scope == Some("runner")
        && index.is_some_and(|index| {
            index.parse::<usize>().is_ok_and(|slot| {
                slot < RUNNER_INSTRUCTION_RESPONSE_MAX_FILES && slot.to_string() == index
            })
        })
        && basename.is_some_and(|name| {
            !name.is_empty() && name.len() <= 255 && name != "." && name != ".."
        })
        && parts.next().is_none()
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_binding_detects_content_change_with_stale_upstream_fingerprint() {
        let file = |content: &str| ProjectInstructionFile {
            source_scope: InstructionSourceScope::Runner,
            path: "runner/0/AGENTS.md".to_string(),
            fingerprint: "a".repeat(64),
            content: content.to_string(),
            chars: content.chars().count(),
            total_lines: 1,
            start_line: 1,
            limit: MAX_LINES_PER_FILE,
            truncated: false,
            read_more: None,
        };
        let response = |content: &str| RunnerInstructionSnapshotResponse {
            format: RUNNER_INSTRUCTION_RESPONSE_FORMAT.to_string(),
            generation: 7,
            scan_complete: true,
            files: vec![file(content)],
        };
        let mut first = response("runner global v1");
        let mut second = response("runner global v2");
        first.bind_visible_fingerprints().unwrap();
        second.bind_visible_fingerprints().unwrap();
        assert_ne!(first.files[0].fingerprint, second.files[0].fingerprint);
        assert_ne!(first.files[0].fingerprint, "a".repeat(64));
    }

    #[test]
    fn logical_runner_sources_cannot_encode_native_or_traversal_paths() {
        assert!(valid_runner_logical_source("runner/0/AGENTS.md"));
        assert!(valid_runner_logical_source("runner/15/company-guidance.md"));
        assert!(!valid_runner_logical_source(
            "/Users/alice/.codex/AGENTS.md"
        ));
        assert!(!valid_runner_logical_source(
            "runner/0//Users/alice/AGENTS.md"
        ));
        assert!(!valid_runner_logical_source("runner/0/../AGENTS.md"));
        assert!(!valid_runner_logical_source(
            "runner/0/C:\\Users\\alice\\AGENTS.md"
        ));
        assert!(!valid_runner_logical_source(
            "runner/not-an-index/AGENTS.md"
        ));
    }

    #[test]
    fn logical_runner_source_indices_are_bounded_and_unique() {
        for index in ["16".to_string(), "01".to_string(), "9".repeat(1024)] {
            assert!(!valid_runner_logical_source(&format!(
                "runner/{index}/rules.md"
            )));
        }
        let files = crate::project_instructions::ProjectInstructionsSnapshot::from_candidates(
            ["runner/0/first.md", "runner/0/second.md"]
                .into_iter()
                .map(
                    |path| crate::project_instructions::LoadedInstructionCandidate {
                        source_scope: InstructionSourceScope::Runner,
                        path: path.into(),
                        content: "rule".into(),
                        total_lines: 1,
                        full_sha256: None,
                    },
                )
                .collect(),
            true,
        )
        .files;
        let response = RunnerInstructionSnapshotResponse {
            format: RUNNER_INSTRUCTION_RESPONSE_FORMAT.into(),
            generation: 1,
            scan_complete: true,
            files,
        };
        assert!(response.validate().is_err());
    }
}
