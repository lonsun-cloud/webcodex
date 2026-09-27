use super::*;

fn source(scope: InstructionSourceScope, path: &str, body: &str) -> ProjectInstructionsSnapshot {
    ProjectInstructionsSnapshot::from_candidates(
        vec![LoadedInstructionCandidate {
            source_scope: scope,
            path: path.into(),
            content: body.into(),
            total_lines: body.lines().count(),
            full_sha256: None,
        }],
        true,
    )
}

#[test]
fn global_character_budget_cannot_erase_project_guidance() {
    let runner = source(
        InstructionSourceScope::Runner,
        "runner/0/rules.md",
        &"x".repeat(MAX_TOTAL_CHARS),
    );
    let project = source(
        InstructionSourceScope::Project,
        "AGENTS.md",
        "local guidance\nsecond line",
    );
    let expected = project.files[0].clone();
    let combined = ProjectInstructionsSnapshot::with_runner_files(runner.files, project, true);
    assert_eq!(
        combined.files[0].source_scope,
        InstructionSourceScope::Runner
    );
    assert!(combined.files[0].truncated);
    assert!(combined.files[0].read_more.is_none());
    assert_eq!(combined.files[1].content, expected.content);
    assert_eq!(combined.files[1].fingerprint, expected.fingerprint);
    assert!(!combined.files[1].truncated);
    assert!(combined.total_chars <= MAX_TOTAL_CHARS);
}

#[test]
fn composing_runner_sources_preserves_project_continuation() {
    let body = "local\n".repeat(MAX_LINES_PER_FILE + 1);
    let project = source(InstructionSourceScope::Project, "AGENTS.md", &body);
    let original_hint = project.files[0].read_more.as_ref().unwrap().start_line;
    let runner = source(
        InstructionSourceScope::Runner,
        "runner/0/rules.md",
        &"global\n".repeat(400),
    );
    let combined = ProjectInstructionsSnapshot::with_runner_files(runner.files, project, true);
    assert_eq!(
        combined.files[1].read_more.as_ref().unwrap().start_line,
        original_hint
    );
    assert!(combined.files[1].truncated);
    assert!(combined.files[0].read_more.is_none());
    assert_eq!(
        combined.files[1].content.lines().count(),
        MAX_LINES_PER_FILE
    );
}

#[test]
fn pre_projection_runner_sources_have_an_independent_aggregate_bound() {
    let runner_files = (0..2)
        .flat_map(|index| {
            source(
                InstructionSourceScope::Runner,
                &format!("runner/{index}/rules.md"),
                &"界".repeat(MAX_TOTAL_CHARS),
            )
            .files
        })
        .collect();
    let project = source(
        InstructionSourceScope::Project,
        "AGENTS.md",
        "local guidance",
    );
    let snapshot = ProjectInstructionsSnapshot::with_runner_files(runner_files, project, true);
    let sources = &snapshot.scan.as_ref().unwrap().runner_source_files;
    assert_eq!(
        sources.iter().map(|file| file.chars).sum::<usize>(),
        MAX_TOTAL_CHARS
    );
    assert_eq!(sources[0].content.chars().count(), MAX_TOTAL_CHARS);
    assert!(sources[1].content.is_empty());
    assert!(sources[1].truncated);
    assert!(sources.iter().all(|file| file.read_more.is_none()));
    assert!(snapshot.total_chars <= MAX_TOTAL_CHARS);
    assert_eq!(snapshot.files.last().unwrap().content, "local guidance");
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert!(serialized.get("scan").is_none());
    assert!(!serde_json::to_string(&serialized).unwrap().contains('界'));
    let restored: ProjectInstructionsSnapshot = serde_json::from_value(serialized).unwrap();
    assert!(restored.scan.is_none());
}
