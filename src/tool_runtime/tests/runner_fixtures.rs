//! Deterministic process-boundary regressions for the local Runner fixtures.

use crate::tool_runtime::helpers::{run_test_command_with_timeout, test_shell};
use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

fn stdin_reader() -> Command {
    let mut command = Command::new(test_shell());
    command.args(["-c", "cat"]);
    command
}

#[test]
fn test_command_without_payload_discards_preconfigured_stdin() {
    let mut ambient_input = tempfile::tempfile().unwrap();
    ambient_input.write_all(b"not fixture input\n").unwrap();
    ambient_input.seek(SeekFrom::Start(0)).unwrap();
    let mut command = stdin_reader();
    command.stdin(Stdio::from(ambient_input));

    let (code, stdout, stderr, _, timed_out) = run_test_command_with_timeout(command, None, 10);
    assert!(!timed_out, "missing payload must deliver EOF: {stderr}");
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.is_empty(),
        "ambient stdin leaked into fixture: {stdout:?}"
    );
}

#[test]
fn test_command_preserves_empty_and_large_explicit_stdin() {
    let large_payload = "fixture input\n".repeat(16_384);
    for payload in [b"".as_slice(), large_payload.as_bytes()] {
        let (code, stdout, stderr, _, timed_out) =
            run_test_command_with_timeout(stdin_reader(), Some(payload), 10);
        assert!(!timed_out, "explicit input did not reach EOF: {stderr}");
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stdout.as_bytes(), payload);
    }
}

#[test]
fn runner_fixture_unborn_baseline_ignores_parent_stdin() {
    // Re-execute exactly one existing workflow test with nonempty parent stdin.
    // This exercises the real RunnerRequest -> fixture -> `git mktree` path,
    // without changing this parallel test process's global stdin/environment.
    // Before the fix mktree consumed the invalid tree line and failed, even on
    // headless CI. A live console/open pipe instead blocked it awaiting EOF.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "tool_runtime::tests::coding_task::work_on_project_captures_repository_native_unborn_empty_tree_baseline",
        "--test-threads=1",
        "--nocapture",
    ]);
    let (code, stdout, stderr, _, timed_out) =
        run_test_command_with_timeout(command, Some(b"not a valid Git tree entry\n"), 45);
    assert!(
        !timed_out,
        "isolated workflow test timed out: {stdout}\n{stderr}"
    );
    assert_eq!(code, 0, "isolated workflow test failed: {stdout}\n{stderr}");
    assert!(
        stdout.contains("1 passed; 0 failed"),
        "the exact workflow regression must execute, not silently select zero tests: {stdout}"
    );
}
