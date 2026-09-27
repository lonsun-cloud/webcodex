use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
fn dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "desktop-reconfiguration-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn credentials(id: &str) -> TunnelConfigRequest {
    TunnelConfigRequest::Save {
        tunnel_id: id.into(),
        api_key: Some("fixture_api_key_470".into()),
    }
}

#[tokio::test]
async fn saved_project_inventory_survives_selection_and_reload() {
    let data = dir();
    let mut core = DesktopCore::new(data.clone(), data.join("resources")).unwrap();
    core.config.runtime = Some(StoredRuntime {
        server_url: "http://127.0.0.1:1".into(),
        runner_config: Some(data.join("runner.toml")),
        runner_client_id: Some("fixture".into()),
        server_env_file: None,
        user_token_file: None,
        project_id: None,
        runtime_project_id: None,
    });
    for name in ["alpha", "beta", "alpha"] {
        let path = data.join(name);
        std::fs::create_dir_all(&path).unwrap();
        let path = path.to_string_lossy().into_owned();
        core.config.project = Some(ProjectSelection {
            path: path.clone(),
            allowed_root: path,
            is_git_repository: false,
            runtime_project_id: None,
        });
        core.save_config().await.unwrap();
    }
    assert_eq!(core.config.saved_projects.len(), 2);
    assert!(core
        .config
        .saved_projects
        .iter()
        .all(|p| p.project.path == p.project.allowed_root));
    let reloaded = DesktopCore::new(data.clone(), data.join("resources")).unwrap();
    assert_eq!(reloaded.snapshot.saved_projects.len(), 2);
    assert!(reloaded.snapshot.project.unwrap().path.ends_with("alpha"));
    std::fs::remove_dir_all(data).unwrap();
}

// This fixture uses only owned cat processes. It never starts a real runtime or tunnel.
#[cfg(unix)]
#[tokio::test]
async fn credential_save_and_failed_tunnel_replacement_preserve_server_runner_pids() {
    let data = dir();
    let state = AppState::new(data.clone(), data.join("resources")).unwrap();
    let kinds = [ProcessKey::LocalServer, ProcessKey::LocalRunner];
    let mut pids = Vec::new();
    for kind in kinds {
        let mut supervisor = state.supervisor.lock().await;
        supervisor
            .spawn_owned(kind, std::process::Command::new("/bin/cat"), false)
            .await
            .unwrap();
        pids.push(supervisor.snapshot(kind).unwrap().pid);
    }
    let saved = state
        .update_tunnel_config(credentials("tunnel_first"))
        .await;
    state
        .supervisor
        .lock()
        .await
        .spawn_owned(
            ProcessKey::RegularTunnel(crate::connection_id::TunnelProfileId::DEFAULT),
            std::process::Command::new("/bin/cat"),
            false,
        )
        .await
        .unwrap();
    // No topology is configured: replacement must fail before binaries/network are touched.
    let replacement = state
        .update_tunnel_config(credentials("tunnel_second"))
        .await;
    let after = state.get_state();
    let mut remaining = Vec::new();
    for kind in kinds {
        remaining.push(state.supervisor.lock().await.snapshot(kind).map(|p| p.pid));
    }
    let tunnel_stopped = state
        .supervisor
        .lock()
        .await
        .snapshot(ProcessKey::RegularTunnel(
            crate::connection_id::TunnelProfileId::DEFAULT,
        ))
        .is_none();
    state.shutdown().await;
    std::fs::remove_dir_all(data).unwrap();
    assert!(saved.is_ok());
    assert_eq!(
        replacement.err().unwrap().code,
        "tunnel_config_apply_failed"
    );
    assert_eq!(remaining, pids.into_iter().map(Some).collect::<Vec<_>>());
    assert!(tunnel_stopped);
    assert_eq!(
        after.openai_tunnel_config.saved_tunnel_id.as_deref(),
        Some("tunnel_second")
    );
    assert!(!serde_json::to_string(&after)
        .unwrap()
        .contains("fixture_api_key_470"));
}
