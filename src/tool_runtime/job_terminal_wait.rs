use super::{ToolResult, ToolRuntime};
use crate::auth::AuthContext;
use crate::job_terminal_attention::{
    fact_from_event, metric as attention_metric, principal_for_auth, source_from_snapshot,
    JobTerminalDeliveryAttempt,
};
use serde_json::json;
use webcodex_store::{JobTerminalWaitRecord, JobTerminalWaitState, NewJobTerminalWait};

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct JobTerminalRegistrationTestHook {
    pub(crate) first_snapshot: std::sync::Arc<tokio::sync::Barrier>,
    pub(crate) resume_after_terminal: std::sync::Arc<tokio::sync::Barrier>,
}

#[cfg(test)]
impl JobTerminalRegistrationTestHook {
    pub(crate) fn new() -> Self {
        Self {
            first_snapshot: std::sync::Arc::new(tokio::sync::Barrier::new(2)),
            resume_after_terminal: std::sync::Arc::new(tokio::sync::Barrier::new(2)),
        }
    }
}

impl ToolRuntime {
    pub(crate) async fn wait_for_job_terminal(
        &self,
        job_id: String,
        idempotency_key: String,
        auth: Option<&AuthContext>,
    ) -> ToolResult {
        let Some(db) = self.job_terminal_db.as_ref() else {
            return unavailable();
        };
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };

        let access = crate::runner_http::runner_access_from_auth(auth);
        let first = match self
            .runner_registry
            .job_terminal_registration_snapshot_for_auth(access.as_ref(), &job_id)
            .await
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ToolResult::err_with_output(error, json!({"error_kind": "unknown_job"}))
            }
        };
        #[cfg(test)]
        if let Some(hook) = &self.job_terminal_registration_test_hook {
            hook.first_snapshot.wait().await;
            hook.resume_after_terminal.wait().await;
        }
        let principal = principal_for_auth(auth);
        let source = source_from_snapshot(&first);
        let now = chrono::Utc::now().timestamp();
        let already_terminal = first.terminal_event.as_ref().map(fact_from_event);
        let mutation = match db.create_job_terminal_wait(
            &principal,
            NewJobTerminalWait {
                source: source.clone(),
                idempotency_key,
                expires_at: first.wait_expires_at,
                already_terminal,
            },
            now,
        ) {
            Ok(mutation) => mutation,
            Err(error) => return store_failure(error.code),
        };

        if mutation.replayed {
            attention_metric("registration_replayed");
        } else {
            attention_metric("armed");
            if mutation.wait.state == JobTerminalWaitState::Triggered {
                attention_metric("already_terminal_immediate_match");
            }
        }

        let mut state_changed = mutation.state_changed;
        let mut wait = mutation.wait;

        // Registration/terminalization race handshake. The first snapshot proves
        // visibility and exact source identity; the durable insert closes the
        // waiting side; this second canonical snapshot closes the already-terminal
        // side. The post-lock Runner sink handles all later terminal transitions.
        if wait.state == JobTerminalWaitState::Waiting {
            let second = match self
                .runner_registry
                .job_terminal_registration_snapshot_for_auth(access.as_ref(), &job_id)
                .await
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return ToolResult::err_with_output(
                        error,
                        json!({
                            "error_kind": "unknown_job",
                            "wait_id": wait.wait_id,
                        }),
                    )
                }
            };
            if source_from_snapshot(&second) != source {
                return ToolResult::err_with_output(
                    "Job identity changed while terminal attention was being armed",
                    json!({
                        "error_kind": "job_identity_changed",
                        "wait_id": wait.wait_id,
                    }),
                );
            }
            if let Some(event) = second.terminal_event.as_ref() {
                match db.match_job_terminal_fact(
                    &fact_from_event(event),
                    chrono::Utc::now().timestamp(),
                ) {
                    Ok(matched) => {
                        state_changed |= matched.matched_count > 0;
                        for (owner, candidate_wait_id) in matched.delivery_candidates {
                            let _ = controller.attempt_delivery(
                                &owner,
                                &candidate_wait_id,
                                chrono::Utc::now().timestamp(),
                            );
                        }
                    }
                    Err(error) => return store_failure(error.code),
                }
            }
        }

        wait = match db.read_job_terminal_wait(
            &principal,
            &wait.wait_id,
            chrono::Utc::now().timestamp(),
        ) {
            Ok(wait) => wait,
            Err(error) => return store_failure(error.code),
        };

        if wait.state == JobTerminalWaitState::Triggered {
            match controller.attempt_delivery(
                &principal,
                &wait.wait_id,
                chrono::Utc::now().timestamp(),
            ) {
                Ok(
                    JobTerminalDeliveryAttempt::Delivered
                    | JobTerminalDeliveryAttempt::DeliveryUnknown,
                ) => {
                    state_changed = true;
                }
                Ok(
                    JobTerminalDeliveryAttempt::NoCarrier
                    | JobTerminalDeliveryAttempt::PreflightFailed
                    | JobTerminalDeliveryAttempt::Deduplicated,
                ) => {}
                Err(error) => return store_failure(error.code),
            }
            wait = match db.read_job_terminal_wait(
                &principal,
                &wait.wait_id,
                chrono::Utc::now().timestamp(),
            ) {
                Ok(wait) => wait,
                Err(error) => return store_failure(error.code),
            };
        }

        ToolResult::ok(wait_output(
            &wait,
            mutation.replayed,
            state_changed,
            controller.automatic_resume_available_for_wait(
                &principal,
                &wait.wait_id,
                chrono::Utc::now().timestamp(),
            ),
        ))
    }

    async fn authorize_exact_job_terminal_wait(
        &self,
        auth: Option<&AuthContext>,
        wait_id: &str,
    ) -> Result<
        (
            webcodex_store::JobTerminalWaitPrincipal,
            JobTerminalWaitRecord,
        ),
        ToolResult,
    > {
        let Some(db) = self.job_terminal_db.as_ref() else {
            return Err(unavailable());
        };
        let principal = principal_for_auth(auth);
        let now = chrono::Utc::now().timestamp();
        let wait = db
            .read_job_terminal_wait(&principal, wait_id, now)
            .map_err(|_| hidden_wait_failure())?;
        let access = crate::runner_http::runner_access_from_auth(auth);
        let snapshot = self
            .runner_registry
            .job_terminal_registration_snapshot_for_auth(access.as_ref(), &wait.source.job_id)
            .await
            .map_err(|_| hidden_wait_failure())?;
        if source_from_snapshot(&snapshot) != wait.source {
            return Err(hidden_wait_failure());
        }
        Ok((principal, wait))
    }

    pub(crate) async fn present_job_terminal_continuation(
        &self,
        auth: Option<&AuthContext>,
        wait_id: String,
    ) -> ToolResult {
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, wait) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        ToolResult::ok(json!({
            "job_terminal_continuation": wait_projection(
                &wait,
                controller.automatic_resume_available_for_wait(
                    &principal,
                    &wait.wait_id,
                    chrono::Utc::now().timestamp(),
                ),
            )
        }))
    }

    pub(crate) async fn job_terminal_continuation_bind_for_window(
        &self,
        auth: Option<&AuthContext>,
        window: Option<&crate::client_window::ClientWindow>,
        wait_id: String,
        binding_id: String,
    ) -> ToolResult {
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, _) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        match controller.bind_mcp_app(
            &principal,
            &wait_id,
            binding_id,
            window.map(crate::client_window::ClientWindow::key),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(bound) => ToolResult::ok(json!({
                "job_terminal_continuation": wait_projection(
                    &bound.wait,
                    controller.automatic_resume_available_for_wait(
                        &principal,
                        &wait_id,
                        chrono::Utc::now().timestamp(),
                    ),
                ),
                "host_binding": { "bound": true },
                "state_changed": bound.state_changed,
            })),
            Err(error) => app_failure(error.code),
        }
    }

    pub(crate) async fn job_terminal_continuation_state_for_window(
        &self,
        auth: Option<&AuthContext>,
        window: Option<&crate::client_window::ClientWindow>,
        wait_id: String,
        binding_id: String,
    ) -> ToolResult {
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, _) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        match controller.mcp_app_state(
            &principal,
            &wait_id,
            &binding_id,
            window.map(crate::client_window::ClientWindow::key),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(state) => ToolResult::ok(json!({
                "job_terminal_continuation": wait_projection(
                    &state.wait,
                    controller.automatic_resume_available_for_wait(
                        &principal,
                        &wait_id,
                        chrono::Utc::now().timestamp(),
                    ),
                ),
                "host_binding": { "bound": true },
                "app_protocol": {
                    "prepared_attempt_id": state.prepared_attempt_id,
                },
            })),
            Err(error) => app_failure(error.code),
        }
    }

    pub(crate) async fn job_terminal_continuation_prepare_for_window(
        &self,
        auth: Option<&AuthContext>,
        window: Option<&crate::client_window::ClientWindow>,
        wait_id: String,
        binding_id: String,
    ) -> ToolResult {
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, _) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        match controller.prepare_mcp_app_delivery(
            &principal,
            &wait_id,
            &binding_id,
            window.map(crate::client_window::ClientWindow::key),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(prepared) => ToolResult::ok(json!({
                "wait_id": prepared.wait.wait_id,
                "job_id": prepared.wait.source.job_id,
                "delivery_state": prepared.wait.delivery_state.as_str(),
                "attempt_id": prepared.attempt_id,
                "dispatch_observation": "dispatch_prepared",
                "state_changed": true,
                "app_protocol": {
                    "automatic_message": prepared.automatic_message,
                },
            })),
            Err(error) => app_failure(error.code),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn job_terminal_continuation_finish_for_window(
        &self,
        auth: Option<&AuthContext>,
        window: Option<&crate::client_window::ClientWindow>,
        wait_id: String,
        binding_id: String,
        attempt_id: String,
        outcome: String,
    ) -> ToolResult {
        let dispatch_accepted = match outcome.as_str() {
            "dispatch_accepted" => true,
            "delivery_unknown" => false,
            _ => {
                return ToolResult::err_with_output(
                    "outcome must be dispatch_accepted or delivery_unknown",
                    json!({"error_kind": "invalid_dispatch_outcome", "state_changed": false}),
                )
            }
        };
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, _) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        match controller.finish_mcp_app_delivery(
            &principal,
            &wait_id,
            &binding_id,
            window.map(crate::client_window::ClientWindow::key),
            &attempt_id,
            dispatch_accepted,
            chrono::Utc::now().timestamp(),
        ) {
            Ok(finished) => ToolResult::ok(json!({
                "wait_id": finished.wait.wait_id,
                "job_id": finished.wait.source.job_id,
                "attempt_id": attempt_id,
                "delivery_state": finished.wait.delivery_state.as_str(),
                "dispatch_observation": if dispatch_accepted { "dispatch_accepted" } else { "delivery_unknown" },
                "state_changed": finished.state_changed,
            })),
            Err(error) => app_failure(error.code),
        }
    }

    pub(crate) async fn job_terminal_continuation_unbind_for_window(
        &self,
        auth: Option<&AuthContext>,
        window: Option<&crate::client_window::ClientWindow>,
        wait_id: String,
        binding_id: String,
    ) -> ToolResult {
        let Some(controller) = self.job_terminal_continuations.as_ref() else {
            return unavailable();
        };
        let (principal, _) = match self.authorize_exact_job_terminal_wait(auth, &wait_id).await {
            Ok(value) => value,
            Err(result) => return result,
        };
        match controller.unbind_mcp_app(
            &principal,
            &wait_id,
            &binding_id,
            window.map(crate::client_window::ClientWindow::key),
            chrono::Utc::now().timestamp(),
        ) {
            Ok(unbound) => ToolResult::ok(json!({
                "job_terminal_continuation": wait_projection(&unbound.wait, false),
                "host_binding": { "bound": false },
                "state_changed": unbound.state_changed,
            })),
            Err(error) => app_failure(error.code),
        }
    }
}

fn wait_projection(
    wait: &JobTerminalWaitRecord,
    automatic_resume_available: bool,
) -> serde_json::Value {
    json!({
        "version": 1,
        "wait_id": wait.wait_id,
        "job_id": wait.source.job_id,
        "state": wait.state.as_str(),
        "delivery_state": wait.delivery_state.as_str(),
        "terminal_status": wait.terminal_status,
        "terminal_outcome": wait.terminal_outcome,
        "automatic_resume_available": automatic_resume_available,
        "expires_at": wait.expires_at,
        "fallback_tool": "observe_jobs",
    })
}

fn wait_output(
    wait: &JobTerminalWaitRecord,
    replayed: bool,
    state_changed: bool,
    automatic_resume_available: bool,
) -> serde_json::Value {
    json!({
        "wait_id": wait.wait_id,
        "job_id": wait.source.job_id,
        "state": wait.state.as_str(),
        "delivery_state": wait.delivery_state.as_str(),
        "terminal_status": wait.terminal_status,
        "terminal_outcome": wait.terminal_outcome,
        "replayed": replayed,
        "state_changed": state_changed,
        "automatic_resume_available": automatic_resume_available,
        "expires_at": wait.expires_at,
        "fallback_tool": "observe_jobs",
    })
}

fn unavailable() -> ToolResult {
    ToolResult::err_with_output(
        "Job terminal attention is not configured",
        json!({"error_kind": "job_terminal_attention_unavailable"}),
    )
}

fn hidden_wait_failure() -> ToolResult {
    ToolResult::err_with_output(
        "Job terminal wait does not exist",
        json!({"error_kind": "job_terminal_wait_not_found", "state_changed": false}),
    )
}

fn app_failure(code: &'static str) -> ToolResult {
    ToolResult::err_with_output(
        "Job terminal continuation App operation failed",
        json!({"error_kind": code, "state_changed": false}),
    )
}

fn store_failure(code: &'static str) -> ToolResult {
    ToolResult::err_with_output(
        "Job terminal attention operation failed",
        json!({"error_kind": code}),
    )
}
