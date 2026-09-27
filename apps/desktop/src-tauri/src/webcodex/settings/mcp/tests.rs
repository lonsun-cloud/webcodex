use super::*;
use crate::mcp_providers::McpProviderRequest;
use std::collections::BTreeMap;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("webcodex-mcp-reconcile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn runtime(&self, slot: &str, client: &str) -> StoredRuntime {
        let path = self.0.join(format!("{slot}.toml"));
        std::fs::write(&path, format!("# enrollment identity\nclient_id = {client:?}\nserver_url = \"http://127.0.0.1:62645\"\ntoken = \"private-runner-{slot}\" # preserve token comment\nowner = \"fixture\"\n[policy]\nallowed_roots = [\"/fixture\"] # roots comment\nallow_raw_shell = false\n[unrelated]\nkeep = 42 # unrelated comment\n[mcp]\nrequest_timeout_secs = 17 # global timeout\n[[mcp.providers]]\nid = \"operator\"\nname = \"Operator provider\"\nexecutable = \"/operator/existing\" # provider comment\nargs = [\"unchanged\"]\n")).unwrap();
        StoredRuntime {
            server_url: "http://127.0.0.1:62645".into(),
            server_env_file: None,
            runner_config: Some(path),
            user_token_file: None,
            runner_client_id: Some(client.into()),
            project_id: None,
            runtime_project_id: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn request(revision: u64) -> McpProviderRequest {
    McpProviderRequest {
        id: None,
        expected_revision: revision,
        name: "Browser automation".into(),
        command: std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        args: vec!["--stdio".into()],
        cwd: None,
        enabled: true,
        env: BTreeMap::from([("API_KEY".into(), Some("mcp-fixture-secret".into()))]),
    }
}
fn text(runtime: &StoredRuntime) -> String {
    std::fs::read_to_string(runtime.runner_config.as_ref().unwrap()).unwrap()
}

#[test]
fn restart_repair_and_ab_slot_switch_replay_desired_state_without_identity_loss() {
    let fixture = Fixture::new();
    let mut store = McpProviderStore::load(&fixture.0);
    let id = store.update(request(0)).unwrap();
    for (slot, client) in [
        ("A", "client-a"),
        ("B", "client-b"),
        ("repaired", "client-new"),
    ] {
        let runtime = fixture.runtime(slot, client);
        reconcile_mcp(&runtime, &McpProviderStore::load(&fixture.0)).unwrap();
        let after = text(&runtime);
        let doc = after.parse::<DocumentMut>().unwrap();
        assert_eq!(doc["client_id"].as_str(), Some(client));
        assert_eq!(
            doc["token"].as_str(),
            Some(format!("private-runner-{slot}").as_str())
        );
        assert_eq!(doc["server_url"].as_str(), Some("http://127.0.0.1:62645"));
        assert_eq!(doc["policy"]["allow_raw_shell"].as_bool(), Some(false));
        assert_eq!(doc["policy"]["allowed_roots"].as_array().unwrap().len(), 1);
        assert_eq!(doc["unrelated"]["keep"].as_integer(), Some(42));
        assert_eq!(doc["mcp"]["request_timeout_secs"].as_integer(), Some(17));
        assert!(after.contains(&id));
        assert!(!after.contains("mcp-fixture-secret"));
        for comment in [
            "# enrollment identity",
            "# preserve token comment",
            "# roots comment",
            "# unrelated comment",
            "# global timeout",
            "# provider comment",
        ] {
            assert!(after.contains(comment), "lost {comment}");
        }
        let providers = doc["mcp"]["providers"].as_array_of_tables().unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(
            providers.get(0).unwrap()["executable"].as_str(),
            Some("/operator/existing")
        );
        reconcile_mcp(&runtime, &store).unwrap();
        assert_eq!(text(&runtime), after, "reconciliation is idempotent");
    }
    store.remove(&id, 1).unwrap();
    for (slot, client) in [
        ("A", "client-a"),
        ("B", "client-b"),
        ("repaired", "client-new"),
    ] {
        let runtime = StoredRuntime {
            server_url: "http://127.0.0.1:62645".into(),
            server_env_file: None,
            runner_config: Some(fixture.0.join(format!("{slot}.toml"))),
            user_token_file: None,
            runner_client_id: Some(client.into()),
            project_id: None,
            runtime_project_id: None,
        };
        reconcile_mcp(&runtime, &McpProviderStore::load(&fixture.0)).unwrap();
        assert!(!text(&runtime).contains(&id));
        assert!(text(&runtime).contains("Operator provider"));
    }
}

#[test]
fn provider_update_preserves_managed_comments_and_unrelated_provider_options() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime("A", "a");
    let mut store = McpProviderStore::load(&fixture.0);
    let id = store.update(request(0)).unwrap();
    reconcile_mcp(&runtime, &store).unwrap();
    let initial = text(&runtime).replace(
        "name = \"Browser automation\"",
        "name = \"Browser automation\" # managed name comment",
    );
    std::fs::write(runtime.runner_config.as_ref().unwrap(), initial).unwrap();
    let mut edit = request(1);
    edit.id = Some(id);
    edit.name = "Changed name".into();
    edit.env.insert("API_KEY".into(), None);
    store.update(edit).unwrap();
    reconcile_mcp(&runtime, &store).unwrap();
    assert!(text(&runtime).contains("name = \"Changed name\" # managed name comment"));
    assert!(text(&runtime).contains("# provider comment"));
}

#[test]
fn inline_provider_arrays_and_disabled_removal_preserve_foreign_entries() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime("inline", "a");
    let mut store = McpProviderStore::load(&fixture.0);
    std::fs::write(runtime.runner_config.as_ref().unwrap(), "client_id='a'\nserver_url='http://127.0.0.1:62645'\ntoken='fixture-token'\nmcp.providers = [{id='operator', name='Untouched', executable='/operator'}] # inline comment\n").unwrap();
    let id = store.update(request(0)).unwrap();
    reconcile_mcp(&runtime, &store).unwrap();
    assert!(text(&runtime).contains("# inline comment"));
    assert_eq!(
        text(&runtime).parse::<DocumentMut>().unwrap()["mcp"]["providers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut edit = request(1);
    edit.id = Some(id);
    edit.enabled = false;
    store.update(edit).unwrap();
    reconcile_mcp(&runtime, &store).unwrap();
    let doc = text(&runtime).parse::<DocumentMut>().unwrap();
    let providers = doc["mcp"]["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(
        providers.get(0).unwrap().as_inline_table().unwrap()["id"].as_str(),
        Some("operator")
    );
}

#[test]
fn inline_mcp_parent_with_no_providers_remains_valid_toml() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime("inline-parent", "a");
    let mut store = McpProviderStore::load(&fixture.0);
    std::fs::write(runtime.runner_config.as_ref().unwrap(), "client_id='a'\nserver_url='http://127.0.0.1:62645'\ntoken='fixture-token'\nmcp = {request_timeout_secs=17} # inline MCP\n").unwrap();
    store.update(request(0)).unwrap();
    reconcile_mcp(&runtime, &store).unwrap();
    let after = text(&runtime);
    let doc = after.parse::<DocumentMut>().unwrap();
    assert_eq!(doc["mcp"]["request_timeout_secs"].as_integer(), Some(17));
    assert_eq!(doc["mcp"]["providers"].as_array().unwrap().len(), 1);
    assert!(after.contains("# inline MCP"));
    reconcile_mcp(&runtime, &store).unwrap();
    assert_eq!(text(&runtime), after);
}

#[test]
fn wrong_identity_duplicate_ids_and_capacity_never_rewrite_runner_config() {
    let fixture = Fixture::new();
    let mut runtime = fixture.runtime("A", "a");
    let mut store = McpProviderStore::load(&fixture.0);
    store.update(request(0)).unwrap();
    let original = text(&runtime);
    runtime.runner_client_id = Some("other".into());
    assert!(reconcile_mcp(&runtime, &store).is_err());
    assert_eq!(text(&runtime), original);
    runtime.runner_client_id = Some("a".into());
    let duplicate = format!("{original}\n[[mcp.providers]]\nid='operator'\n");
    std::fs::write(runtime.runner_config.as_ref().unwrap(), &duplicate).unwrap();
    assert!(reconcile_mcp(&runtime, &store).is_err());
    assert_eq!(text(&runtime), duplicate);
    std::fs::write(runtime.runner_config.as_ref().unwrap(), &original).unwrap();
    for revision in 1..MCP_GATEWAY_MAX_PROVIDERS as u64 {
        store.update(request(revision)).unwrap();
    }
    assert_eq!(
        reconcile_mcp(&runtime, &store).unwrap_err().code,
        "mcp_provider_capacity"
    );
    assert_eq!(text(&runtime), original);
}
