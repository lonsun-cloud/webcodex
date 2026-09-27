use super::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    dir: PathBuf,
    runtime: StoredRuntime,
}
impl Fixture {
    fn new(extra: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "desktop-settings-regression-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runner.toml");
        std::fs::write(&path, format!("server_url = 'http://127.0.0.1:1'\nclient_id = 'fixture'\ntoken = 'fixture-credential'\n{extra}")).unwrap();
        Self {
            dir,
            runtime: StoredRuntime {
                server_url: "http://127.0.0.1:1".into(),
                runner_config: Some(path),
                runner_client_id: Some("fixture".into()),
                server_env_file: None,
                user_token_file: None,
                project_id: None,
                runtime_project_id: None,
            },
        }
    }
    fn paths_request(&self) -> SettingsUpdate {
        SettingsUpdate {
            target: target(&self.runtime).unwrap(),
            expected: RunnerPaths {
                instruction_files: vec![],
                skill_roots: vec![],
            },
            paths: RunnerPaths {
                instruction_files: vec![],
                skill_roots: vec![],
            },
        }
    }
    fn plugin(&self, id: &str) -> PluginAddRequest {
        PluginAddRequest {
            target: target(&self.runtime).unwrap(),
            provider: PluginRegistration {
                id: id.into(),
                name: "Fixture".into(),
                command: "node".into(),
                args: vec!["fixture-sensitive-argument".into()],
                cwd: Some(self.dir.to_string_lossy().into()),
            },
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn stale_settings_cannot_retarget_another_runner_with_identical_empty_paths() {
    let first = Fixture::new("");
    let other = Fixture::new("");
    assert!(update(&other.runtime, first.paths_request()).is_err());
    assert!(add_plugin(&other.runtime, first.plugin("new-plugin")).is_err());
    assert!(verify_target(&other.runtime, &target(&first.runtime).unwrap()).is_err());
}

#[test]
fn malformed_sections_fail_without_panicking_or_mutating() {
    for section in [
        "instructions = false",
        "skills = 42",
        "instructions = { files = 7 }",
    ] {
        let f = Fixture::new(section);
        let before = read(f.runtime.runner_config.as_ref().unwrap()).unwrap();
        assert!(update(&f.runtime, f.paths_request()).is_err());
        assert!(read(f.runtime.runner_config.as_ref().unwrap()).unwrap() == before);
    }
}

#[test]
fn plugin_add_handles_table_and_inline_registrations_without_disclosing_arguments() {
    for prior in [
        "[plugins]\n[[plugins.providers]]\nid = 'original'\nname = 'Original'\ncommand = 'node'\n",
        "[plugins]\nproviders = [{id = 'original', name = 'Original', command = 'node'}]\n",
    ] {
        let f = Fixture::new(prior);
        add_plugin(&f.runtime, f.plugin("extra")).unwrap();
        let projection = inspect(&f.runtime, false).unwrap();
        assert_eq!(projection.plugin_ids, vec!["original", "extra"]);
        let json = serde_json::to_string(&projection).unwrap();
        assert!(
            !json.contains("fixture-sensitive-argument") && !json.contains("fixture-credential")
        );
        assert!(add_plugin(&f.runtime, f.plugin("extra")).is_err());
        assert!(read(f.runtime.runner_config.as_ref().unwrap())
            .unwrap()
            .contains("token = 'fixture-credential'"));
    }
}

#[test]
fn plugin_registration_is_bounded_and_never_launches_programs() {
    let f = Fixture::new("");
    let mut invalid = f.plugin("extra");
    invalid.provider.args = vec!["x".repeat(4097)];
    assert!(add_plugin(&f.runtime, invalid).is_err());
    for i in 0..PLUGIN_MAX_PROVIDERS {
        add_plugin(&f.runtime, f.plugin(&format!("fixture-{i}"))).unwrap();
    }
    assert!(add_plugin(&f.runtime, f.plugin("overflow")).is_err());
    assert_eq!(
        inspect(&f.runtime, false).unwrap().plugin_ids.len(),
        PLUGIN_MAX_PROVIDERS
    );
}
