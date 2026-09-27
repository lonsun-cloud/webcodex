use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    runtime: StoredRuntime,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "desktop-settings-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runner.toml");
        std::fs::write(&path, "# keep this comment\nserver_url = 'http://127.0.0.1:7891'\nclient_id = 'fixture'\ntoken = 'fixture-secret'\n[policy]\nallowed_roots = ['/exact']\n[plugins]\n[[plugins.providers]]\nid = 'example'\nname = 'Example'\ncommand = 'node'\nargs = ['fixture-secret']\n").unwrap();
        Self {
            dir,
            runtime: StoredRuntime {
                server_url: "http://127.0.0.1:7891".into(),
                server_env_file: None,
                runner_config: Some(path),
                user_token_file: None,
                runner_client_id: Some("fixture".into()),
                project_id: None,
                runtime_project_id: None,
            },
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn empty() -> RunnerPaths {
    RunnerPaths {
        instruction_files: vec![],
        skill_roots: vec![],
    }
}

#[test]
fn settings_preserve_credentials_policy_plugins_and_comments() {
    let f = Fixture::new();
    let next = RunnerPaths {
        instruction_files: vec![f.dir.join("global.md").to_string_lossy().into_owned()],
        skill_roots: vec![f.dir.join("skills").to_string_lossy().into_owned()],
    };
    update(
        &f.runtime,
        SettingsUpdate {
            target: target(&f.runtime).unwrap(),
            expected: empty(),
            paths: next.clone(),
        },
    )
    .unwrap();
    let projection = inspect(&f.runtime, false).unwrap();
    assert!(projection.paths == next);
    assert_eq!(projection.plugin_ids, vec!["example"]);
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains("fixture-secret"));
    let text = read(f.runtime.runner_config.as_ref().unwrap()).unwrap();
    for preserved in [
        "# keep this comment",
        "token = 'fixture-secret'",
        "allowed_roots = ['/exact']",
        "args = ['fixture-secret']",
    ] {
        assert!(text.contains(preserved));
    }
    assert!(update(
        &f.runtime,
        SettingsUpdate {
            target: target(&f.runtime).unwrap(),
            expected: empty(),
            paths: empty()
        }
    )
    .is_err());
    update(
        &f.runtime,
        SettingsUpdate {
            target: target(&f.runtime).unwrap(),
            expected: next,
            paths: empty(),
        },
    )
    .unwrap();
    assert!(inspect(&f.runtime, false).unwrap().paths == empty());
}

#[test]
fn settings_reject_identity_changes_traversal_duplicates_and_bounds() {
    let mut f = Fixture::new();
    for invalid in [
        vec!["relative".into()],
        vec![f.dir.join("../escape").to_string_lossy().into_owned()],
        vec![f.dir.to_string_lossy().into_owned(); 2],
        vec![f.dir.to_string_lossy().into_owned(); 17],
    ] {
        assert!(update(
            &f.runtime,
            SettingsUpdate {
                target: target(&f.runtime).unwrap(),
                expected: empty(),
                paths: RunnerPaths {
                    instruction_files: invalid,
                    skill_roots: vec![]
                }
            }
        )
        .is_err());
    }
    f.runtime.runner_client_id = Some("other".into());
    assert!(inspect(&f.runtime, false).is_err());
    assert!(update(
        &f.runtime,
        SettingsUpdate {
            target: target(&f.runtime).unwrap(),
            expected: empty(),
            paths: empty()
        }
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn settings_refuse_symlink_and_write_private_permissions() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let mut f = Fixture::new();
    update(
        &f.runtime,
        SettingsUpdate {
            target: target(&f.runtime).unwrap(),
            expected: empty(),
            paths: empty(),
        },
    )
    .unwrap();
    let path = f.runtime.runner_config.clone().unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = f.dir.join("link.toml");
    symlink(path, &link).unwrap();
    f.runtime.runner_config = Some(link);
    assert!(inspect(&f.runtime, false).is_err());
}
