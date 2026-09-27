use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("webcodex-mcp-desired-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn store(&self) -> McpProviderStore {
        McpProviderStore::load(&self.0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn request(revision: u64) -> McpProviderRequest {
    McpProviderRequest {
        id: None,
        expected_revision: revision,
        name: "Local test MCP".into(),
        command: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec!["--stdio".into()],
        cwd: None,
        enabled: true,
        env: BTreeMap::from([
            (
                "DATABASE_URL".into(),
                Some("postgres://fixture-only-private".into()),
            ),
            ("API_KEY".into(), Some("fixture-only-api-key".into())),
        ]),
    }
}

#[test]
fn desired_profiles_and_private_credentials_survive_desktop_restart() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let id = store.update(request(0)).unwrap();
    let restarted = fixture.store();
    assert!(!restarted.invalid);
    assert_eq!(restarted.snapshot(None).profiles[0].id, id);
    assert!(restarted.snapshot(None).restart_required);
    assert!(!restarted.snapshot(Some(1)).restart_required);
    let public = serde_json::to_string(&restarted.snapshot(None)).unwrap();
    let ordinary = fs::read_to_string(restarted.manifest_path()).unwrap();
    for secret in ["postgres://fixture-only-private", "fixture-only-api-key"] {
        assert!(!public.contains(secret));
        assert!(!ordinary.contains(secret));
    }
    assert!(!ordinary.contains("\"api_key\""));
    let mut command = Command::new("unused");
    restarted.apply_to_command(&mut command).unwrap();
    let injected = command
        .get_envs()
        .filter(|(key, _)| key.to_string_lossy().starts_with(PRIVATE_ENV_PREFIX))
        .collect::<Vec<_>>();
    assert_eq!(injected.len(), 2);
    assert!(injected
        .iter()
        .any(|(_, value)| value.unwrap() == "fixture-only-api-key"));
}

#[test]
fn update_retains_only_requested_existing_keys_and_remove_retires_private_values() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let id = store.update(request(0)).unwrap();
    let original_ref = store.manifest.profiles[0].secret_ref.clone().unwrap();
    let mut edit = request(1);
    edit.id = Some(id.clone());
    edit.name = "Database".into();
    edit.env = BTreeMap::from([
        ("API_KEY".into(), None),
        ("PASSWORD".into(), Some("new-fixture-password".into())),
    ]);
    store.update(edit).unwrap();
    assert!(!store.secret_path(&original_ref).exists());
    assert_eq!(
        store.snapshot(None).profiles[0].env_keys,
        ["API_KEY", "PASSWORD"]
    );
    let values = store.credentials(&store.manifest.profiles[0]).unwrap();
    assert_eq!(values["API_KEY"], "fixture-only-api-key");
    assert!(!values.contains_key("DATABASE_URL"));
    let current_ref = store.manifest.profiles[0].secret_ref.clone().unwrap();
    store.remove(&id, 2).unwrap();
    assert!(!store.secret_path(&current_ref).exists());
    let restarted = fixture.store();
    assert!(restarted.profiles().unwrap().is_empty());
    assert!(
        restarted.managed_ids().contains(&id),
        "old A/B slots need a removal tombstone"
    );
}

#[test]
fn stale_writers_and_invalid_requests_preserve_current_configuration() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.update(request(0)).unwrap();
    let mut stale = fixture.store();
    store.update(request(1)).unwrap();
    let current = fs::read(store.manifest_path()).unwrap();
    assert!(stale.update(request(1)).is_err());
    assert_eq!(fs::read(store.manifest_path()).unwrap(), current);
    let mut cases = Vec::new();
    let mut invalid = request(2);
    invalid.name.clear();
    cases.push(invalid);
    let mut invalid = request(2);
    invalid.command = "../not-a-command".into();
    cases.push(invalid);
    let mut invalid = request(2);
    invalid.args = vec!["x\0y".into()];
    cases.push(invalid);
    let mut invalid = request(2);
    invalid
        .env
        .insert("WEBCODEX_TOKEN".into(), Some("forbidden-fixture".into()));
    cases.push(invalid);
    let mut invalid = request(2);
    invalid
        .env
        .insert("AUTHORIZATION".into(), Some("forbidden-fixture".into()));
    cases.push(invalid);
    let mut invalid = request(2);
    invalid.env.insert("DATABASE_URL".into(), None);
    cases.push(invalid);
    let mut invalid = request(2);
    invalid.id = Some("../../outside".into());
    cases.push(invalid);
    for request in cases {
        let error = store.update(request).unwrap_err();
        assert!(!serde_json::to_string(&error).unwrap().contains("fixture"));
        assert_eq!(fs::read(store.manifest_path()).unwrap(), current);
    }
}

#[test]
fn disabled_providers_are_persistent_but_not_injected_or_resolved() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let mut saved = request(0);
    saved.enabled = false;
    saved.command = "not-installed-yet".into();
    store.update(saved).unwrap();
    let restarted = fixture.store();
    assert!(!restarted.profiles().unwrap()[0].enabled);
    let mut command = Command::new("unused");
    restarted.apply_to_command(&mut command).unwrap();
    assert!(!command
        .get_envs()
        .any(
            |(key, value)| key.to_string_lossy().starts_with(PRIVATE_ENV_PREFIX) && value.is_some()
        ));
}

#[test]
fn duplicate_ids_corruption_and_missing_secret_fail_closed_without_secret_projection() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.update(request(0)).unwrap();
    let original = fs::read(store.manifest_path()).unwrap();
    let mut duplicate = store.manifest.clone();
    duplicate.profiles.push(duplicate.profiles[0].clone());
    fs::write(
        store.manifest_path(),
        serde_json::to_vec(&duplicate).unwrap(),
    )
    .unwrap();
    let invalid = fixture.store();
    assert!(invalid.snapshot(None).config_error);
    assert!(invalid.snapshot(None).profiles.is_empty());
    fs::write(store.manifest_path(), &original).unwrap();
    let reference = store.manifest.profiles[0].secret_ref.as_ref().unwrap();
    fs::write(
        store.secret_path(reference),
        "malformed-fixture-private-value",
    )
    .unwrap();
    let invalid = fixture.store();
    assert!(invalid.snapshot(None).config_error);
    assert!(!serde_json::to_string(&invalid.snapshot(None))
        .unwrap()
        .contains("fixture-private"));
    assert!(invalid
        .apply_to_command(&mut Command::new("unused"))
        .is_err());
}

#[test]
fn private_candidate_cannot_retire_previous_configuration_when_manifest_commit_fails() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.update(request(0)).unwrap();
    let old_reference = store.manifest.profiles[0].secret_ref.clone().unwrap();
    let old_manifest = fs::read(store.manifest_path()).unwrap();
    let candidate = uuid::Uuid::new_v4().to_string();
    store
        .write_credentials(
            &candidate,
            &BTreeMap::from([("API_KEY".into(), "unpublished-fixture".into())]),
        )
        .unwrap();
    let result =
        crate::state::write_atomic_file_with_hook(&store.manifest_path(), b"candidate", |_| {
            Err(std::io::ErrorKind::Interrupted.into())
        });
    assert!(result.is_err());
    assert_eq!(fs::read(store.manifest_path()).unwrap(), old_manifest);
    assert!(store.secret_path(&old_reference).exists());
    let restarted = fixture.store();
    assert!(!restarted.invalid);
    assert_eq!(
        restarted
            .credentials(&restarted.manifest.profiles[0])
            .unwrap()["API_KEY"],
        "fixture-only-api-key"
    );
}

#[test]
fn private_plus_bootstrap_environment_obeys_runner_mapping_capacity() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    let mut too_many = request(0);
    too_many.env = (0..64)
        .map(|index| (format!("FIXTURE_{index}"), Some("fixture".into())))
        .collect();
    assert!(store.update(too_many).is_err());
    assert!(!store.manifest_path().exists());
    let names = vec!["PATH".to_string()];
    assert!(!bootstrap_environment(&names).contains(&"PATH"));
}

#[test]
fn enabled_capacity_matches_runner_and_rejects_without_partial_commit() {
    let fixture = Fixture::new();
    let mut store = fixture.store();
    for revision in 0..MCP_GATEWAY_MAX_PROVIDERS as u64 {
        store.update(request(revision)).unwrap();
    }
    let before = fs::read(store.manifest_path()).unwrap();
    assert!(store
        .update(request(MCP_GATEWAY_MAX_PROVIDERS as u64))
        .is_err());
    assert_eq!(fs::read(store.manifest_path()).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn private_files_have_owner_only_permissions_and_symlinks_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = Fixture::new();
    let mut store = fixture.store();
    store.update(request(0)).unwrap();
    let reference = store.manifest.profiles[0].secret_ref.clone().unwrap();
    let secret = store.secret_path(&reference);
    assert_eq!(
        fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(secret.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    fs::remove_file(&secret).unwrap();
    symlink(store.manifest_path(), &secret).unwrap();
    assert!(fixture.store().invalid);
}
