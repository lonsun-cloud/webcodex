use super::*;
use crate::connection_id::TunnelProfileId;
use crate::connections::{ConnectionsSnapshot, TunnelConnectionSnapshot};
use crate::tunnel_config::TunnelProfileRequest;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAction {
    Start,
    Stop,
    Restart,
    Delete,
}

impl AppState {
    pub async fn save_tunnel_profile(
        &self,
        request: TunnelProfileRequest,
    ) -> DesktopResult<DesktopStateSnapshot> {
        let (operation, cancellation, mut core, baseline) = self
            .begin_operation(DesktopOperationKind::TunnelConfigUpdate, false)
            .await?;
        let result = async {
            cancellation.check()?;
            let previous = core.tunnel_config.clone();
            let id = core
                .mutate_tunnel_config(move |config, path| config.update_profile(path, request))
                .await?;
            core.project_connections();
            core.publish_snapshot();
            let active =
                process_is_active(core.process_snapshot(ProcessKey::RegularTunnel(id)).await);
            if active && !core.tunnel_config.same_launch_as(&previous, id) {
                core.stop_connection_process(id)
                    .await
                    .map_err(tunnel_apply_error)?;
            }
            let enabled = core
                .tunnel_config
                .profiles()
                .iter()
                .any(|p| p.id == id && p.enabled);
            if enabled && core.snapshot.readiness.runtime_ready {
                core.start_connection_process(id, &cancellation)
                    .await
                    .map_err(tunnel_apply_error)?;
            }
            core.get_state().await
        }
        .await;
        self.finish_operation(operation, cancellation, core, baseline, result)
            .await
    }

    pub async fn tunnel_profile_action(
        &self,
        id: TunnelProfileId,
        action: ConnectionAction,
    ) -> DesktopResult<DesktopStateSnapshot> {
        let (operation, cancellation, mut core, baseline) = self
            .begin_operation(DesktopOperationKind::TunnelConfigUpdate, false)
            .await?;
        let result = async {
            cancellation.check()?;
            if !core.tunnel_config.profiles().iter().any(|p| p.id == id) {
                return Err(connection_missing());
            }
            match action {
                ConnectionAction::Start | ConnectionAction::Restart => {
                    core.mutate_tunnel_config(move |config, path| {
                        config.set_enabled(path, id, true)
                    })
                    .await?;
                    if matches!(action, ConnectionAction::Restart) {
                        core.stop_connection_process(id).await?;
                    }
                    core.start_connection_process(id, &cancellation).await?;
                }
                ConnectionAction::Stop => {
                    // Persist the user's stop intent before touching the owned process.
                    core.mutate_tunnel_config(move |config, path| {
                        config.set_enabled(path, id, false)
                    })
                    .await?;
                    core.stop_connection_process(id).await?;
                }
                ConnectionAction::Delete => {
                    // A failed stop must retain both identity and secret for recovery.
                    core.stop_connection_process(id).await?;
                    core.mutate_tunnel_config(move |config, path| config.remove(path, id))
                        .await?;
                    core.connections.remove(id);
                }
            }
            core.get_state().await
        }
        .await;
        self.finish_operation(operation, cancellation, core, baseline, result)
            .await
    }
}

impl DesktopCore {
    pub(super) async fn mutate_tunnel_config<T: Send + 'static>(
        &mut self,
        change: impl FnOnce(&mut TunnelConfig, &Path) -> DesktopResult<T> + Send + 'static,
    ) -> DesktopResult<T> {
        let mut config = self.tunnel_config.clone();
        let path = self.data_dir.join("secrets/tunnel-config.json");
        let (config, result) = tokio::task::spawn_blocking(move || {
            let result = change(&mut config, &path)?;
            Ok::<_, DesktopError>((config, result))
        })
        .await
        .map_err(|_| {
            DesktopError::new(
                "tunnel_config_unavailable",
                "Connection settings could not be saved",
                "Refresh Desktop before retrying.",
            )
        })??;
        self.tunnel_config = config;
        Ok(result)
    }

    pub(super) fn project_connections(&mut self) {
        self.snapshot.connections = ConnectionsSnapshot {
            profiles: self
                .tunnel_config
                .profiles()
                .into_iter()
                .map(|config| TunnelConnectionSnapshot {
                    config,
                    runtime: Default::default(),
                })
                .collect(),
            config_error: self.tunnel_config.is_invalid(),
            ..Default::default()
        };
        self.connections.project(
            &mut self.snapshot.connections,
            self.snapshot.readiness.runtime_ready,
        );
    }

    pub(super) async fn start_connection_process(
        &mut self,
        id: TunnelProfileId,
        cancellation: &CancellationContext,
    ) -> DesktopResult<()> {
        let result = self.spawn_connection(id, cancellation).await;
        if result.is_err() {
            self.connections.fail_start(id);
        }
        result
    }

    async fn spawn_connection(
        &mut self,
        id: TunnelProfileId,
        cancellation: &CancellationContext,
    ) -> DesktopResult<()> {
        cancellation.check()?;
        if process_is_active(self.process_snapshot(ProcessKey::RegularTunnel(id)).await) {
            return Ok(());
        }
        if self.snapshot.quick_share.is_some()
            || !self.config.topology.as_ref().is_some_and(|t| {
                t.experience == Experience::Full && matches!(t.server, ServerTopology::Local)
            })
        {
            return Err(DesktopError::new(
                "unsupported_topology",
                "Connections require the shared local full runtime",
                "Start the local Server and Runner before starting a connection.",
            ));
        }
        if !self.snapshot.readiness.runtime_ready {
            return Err(DesktopError::new(
                "runtime_not_ready",
                "The local runtime is not ready",
                "Restore Server and Runner readiness before starting the connection.",
            ));
        }
        let runtime = self
            .config
            .runtime
            .as_ref()
            .ok_or_else(connection_missing)?;
        let env_file = runtime
            .server_env_file
            .clone()
            .filter(|path| path.is_file())
            .ok_or_else(|| {
                DesktopError::new(
                    "server_unavailable",
                    "Local Server configuration is unavailable",
                    "Run Local Setup again.",
                )
            })?;
        let expected_root = env_file
            .parent()
            .ok_or_else(connection_missing)?
            .join("regular-tunnel-runtime");
        let local_mcp_url = format!("{}/mcp", runtime.server_url.trim_end_matches('/'));
        self.adapter.ensure_binaries(cancellation).await?;
        let proxy = effective_tunnel_proxy(&self.config.tunnel_proxy)?;
        let mut command = self
            .adapter
            .regular_tunnel_command(&env_file, proxy.url.as_deref())?;
        // Use the credential belonging to this exact Desktop-managed Server file,
        // not a bootstrap credential inherited from the shell that launched Desktop.
        command.env_remove("WEBCODEX_TOKEN");
        self.tunnel_config
            .apply_profile_to_command(id, &mut command)?;
        let key = ProcessKey::RegularTunnel(id);
        let mut supervisor = self.supervisor.lock().await;
        cancellation.check()?;
        let events = supervisor
            .spawn_owned(key, command, true)
            .await?
            .ok_or_else(connection_missing)?;
        let process = supervisor.snapshot(key).ok_or_else(connection_missing)?;
        self.connections.start(
            id,
            process,
            events,
            expected_root,
            local_mcp_url,
            self.supervisor.clone(),
            self.activity.clone(),
        );
        Ok(())
    }

    pub(super) async fn stop_connection_process(
        &mut self,
        id: TunnelProfileId,
    ) -> DesktopResult<()> {
        self.connections.stopping(id);
        let result = self
            .supervisor
            .lock()
            .await
            .stop_checked(ProcessKey::RegularTunnel(id))
            .await;
        self.connections.stopped(id, result.is_ok());
        result
    }

    pub(super) async fn stop_all_connection_processes(&mut self) -> DesktopResult<()> {
        self.connections.cancel_all();
        let keys = self.supervisor.lock().await.keys();
        let mut failure = None;
        for key in keys {
            if let ProcessKey::RegularTunnel(id) = key {
                if let Err(error) = self.stop_connection_process(id).await {
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(super) async fn autostart_connections(
        &mut self,
        cancellation: &CancellationContext,
    ) -> DesktopResult<()> {
        for profile in self.tunnel_config.profiles() {
            cancellation.check()?;
            if profile.enabled && profile.autostart {
                // A profile failure is local to that connection, not a setup failure.
                let _ = self
                    .start_connection_process(profile.id, cancellation)
                    .await;
            }
        }
        cancellation.check()
    }

    /// Legacy onboarding addresses only the reserved default profile.
    pub async fn start_regular_tunnel(
        &mut self,
        cancellation: &CancellationContext,
    ) -> DesktopResult<DesktopStateSnapshot> {
        self.mutate_tunnel_config(|config, path| config.enable_default_onboarding(path))
            .await?;
        self.start_connection_process(TunnelProfileId::DEFAULT, cancellation)
            .await?;
        self.get_state().await
    }
    pub async fn stop_regular_tunnel(
        &mut self,
        _cancellation: &CancellationContext,
    ) -> DesktopResult<DesktopStateSnapshot> {
        self.mutate_tunnel_config(|config, path| {
            config.set_enabled(path, TunnelProfileId::DEFAULT, false)
        })
        .await?;
        self.stop_connection_process(TunnelProfileId::DEFAULT)
            .await?;
        self.get_state().await
    }
}

fn connection_missing() -> DesktopError {
    DesktopError::new(
        "tunnel_profile_missing",
        "This connection is unavailable",
        "Refresh Connections before retrying.",
    )
}
