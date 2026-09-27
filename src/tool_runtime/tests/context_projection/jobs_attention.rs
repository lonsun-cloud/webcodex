use super::super::jobs::{
    mark_next_agent_job_running, register_job_agent_for_auth, start_agent_runtime_job_in_session,
};
use super::*;
use crate::tool_runtime::context_projection::{
    ContextMaterialCapabilities, MAX_CONTEXT_PROJECTION_BYTES,
};

#[tokio::test]
async fn jobs_attention_is_explicit_deduplicated_and_coexists_with_guidance() {
    let root = tempfile::tempdir().unwrap();
    init_git_repo(root.path());
    std::fs::write(
        root.path().join("AGENTS.md"),
        "# Project rules\nKeep effects explicit.\n",
    )
    .unwrap();
    let runtime = ToolRuntime::new_for_tests();
    let client = "attention-read";
    let project = register_runner_project_at_path(&runtime, client, "repo", root.path()).await;
    let call = || ToolCall::GitStatus {
        project: project.clone(),
        session_id: None,
    };
    let without = dispatch_with_context_and_local_agent(&runtime, client, call(), vec![]).await;
    assert!(without.success);
    assert!(without.output.get("context_projection").is_none());
    let result = dispatch_with_context_and_local_agent(
        &runtime,
        client,
        call(),
        vec![
            "jobs.attention".into(),
            "jobs.attention".into(),
            "project.instructions".into(),
            "webcodex.workflow".into(),
        ],
    )
    .await;
    assert!(result.success, "{:?}", result.error);
    assert_eq!(
        result.output["context_projection"]["materials"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for key in [
        "jobs.attention",
        "project.instructions",
        "webcodex.workflow",
    ] {
        assert_eq!(context_material(&result, key)["status"], "available");
    }
    let jobs = &context_material(&result, "jobs.attention")["projection"];
    for count in [
        "active_count",
        "running_count",
        "recovering_count",
        "stop_requested_count",
        "terminal_pending_count",
        "blocking_active_count",
        "nonblocking_active_count",
    ] {
        assert_eq!(jobs[count], 0);
    }
    assert_eq!(jobs["recent"], json!([]));
    assert_eq!(jobs["recent_limit"], 8);
    assert_eq!(jobs["truncated"], false);
    assert!(
        serde_json::to_vec(&result.output["context_projection"])
            .unwrap()
            .len()
            <= MAX_CONTEXT_PROJECTION_BYTES
    );
}

#[tokio::test]
async fn jobs_attention_requires_project_and_inventory_scope_not_recorder_inference() {
    let root = tempfile::tempdir().unwrap();
    let runtime = ToolRuntime::new_for_tests();
    let id =
        register_runner_project_at_path(&runtime, "attention-scope", "repo", root.path()).await;
    let auth = auth_context(None, true);
    let project = runtime
        .resolve_project_input_for_auth(&id, Some(&auth))
        .await
        .unwrap();
    let mut read_only = shared_key_auth_context("attention-reader");
    read_only
        .scopes
        .retain(|scope| scope != crate::auth::SCOPE_RUNTIME_READ);
    assert!(read_only.has_scope(crate::auth::SCOPE_PROJECT_READ));
    assert!(!read_only.has_scope(crate::auth::SCOPE_RUNTIME_READ));
    for main_success in [true, false] {
        let mut result = if main_success {
            ToolResult::ok(json!({"main": "unchanged"}))
        } else {
            ToolResult::err("main failed".to_string())
        };
        result.output = json!({"main": "unchanged"});
        let error = result.error.clone();
        runtime
            .add_requested_context_projection(
                &mut result,
                &["jobs.attention".into()],
                Some(&project),
                Some(&read_only),
                ContextMaterialCapabilities::default(),
            )
            .await;
        assert_eq!(result.success, main_success);
        assert_eq!(result.error, error);
        assert_eq!(result.output["main"], "unchanged");
        assert_eq!(
            context_material(&result, "jobs.attention")["status"],
            "unavailable"
        );
        assert_eq!(
            context_material(&result, "jobs.attention")["reason_code"],
            "context_material_scope_unavailable"
        );
    }
    let session = runtime.sessions.start_session(Some(id), None);
    let result = runtime
        .call_tool_with_invocation_metadata(
            ToolCallRequest {
                tool_name: "list_tools".into(),
                arguments: json!({}),
            },
            ToolCallContext {
                transport: ToolTransport::Mcp,
                session_id: Some(&session.session_id),
                auth: Some(&auth),
                window: None,
                record_oauth_scope_denials: true,
                host_file_import_trust: HostFileImportTrust::Untrusted,
            },
            ToolInvocationMetadata {
                context_request: vec!["jobs.attention".into()],
                ..Default::default()
            },
            ToolProtocolCapabilities {
                context_sidecar: true,
                ..Default::default()
            },
        )
        .await
        .result
        .unwrap();
    assert!(result.success);
    assert_eq!(
        context_material(&result, "jobs.attention")["reason_code"],
        "project_target_unavailable"
    );
}

#[tokio::test]
async fn jobs_attention_filters_authority_and_project_before_bound_and_reuses_counts() {
    let runtime = ToolRuntime::new_for_tests();
    let auth = shared_key_auth_context("attention-owner");
    let foreign = shared_key_auth_context("attention-foreign");
    let client = "attention-target";
    let id = format!("agent:{client}:repo");
    super::super::jobs::register_job_agent_for_auth_with_reconciliation(
        &runtime, client, "repo", &auth, true,
    )
    .await;
    register_job_agent_for_auth(&runtime, "attention-other", "repo", &auth).await;
    register_job_agent_for_auth(&runtime, "attention-foreign", "repo", &foreign).await;
    let session = runtime.sessions.start_session(Some(id.clone()), None);
    let mut expected = Vec::new();
    for _ in 0..10 {
        let job = start_agent_runtime_job_in_session(
            &runtime,
            client,
            "repo",
            Some(&session.session_id),
            &auth,
        )
        .await;
        assert_eq!(mark_next_agent_job_running(&runtime, client).await, job);
        expected.push(job);
    }
    let mut excluded = Vec::new();
    // Newer caller-visible Jobs from another Project must not consume the bound.
    for _ in 0..10 {
        excluded.push(
            start_agent_runtime_job_in_session(&runtime, "attention-other", "repo", None, &auth)
                .await,
        );
    }
    excluded.push(
        start_agent_runtime_job_in_session(&runtime, "attention-foreign", "repo", None, &foreign)
            .await,
    );
    let project = runtime
        .resolve_project_input_for_auth(&id, Some(&auth))
        .await
        .unwrap();
    let stop = runtime
        .stop_job_model_facing(
            id.clone(),
            expected[0].clone(),
            Some(session.session_id.clone()),
            true,
            Some(&auth),
        )
        .await;
    assert!(stop.success, "{:?}", stop.error);
    for disconnected in [false, true] {
        if disconnected {
            runtime
                .runner_registry
                .reconcile_disconnect(client, "inst")
                .await;
        }
        let mut result = ToolResult::ok(json!({"main": "read completed"}));
        runtime
            .add_requested_context_projection(
                &mut result,
                &["jobs.attention".into()],
                Some(&project),
                Some(&auth),
                ContextMaterialCapabilities::default(),
            )
            .await;
        let material = context_material(&result, "jobs.attention");
        assert_eq!(material["status"], "available");
        let jobs = &material["projection"];
        assert_eq!(
            *jobs,
            runtime
                .active_jobs_summary(Some(&id), None, Some(&auth), 8)
                .await
        );
        assert_eq!(jobs["active_count"], 10);
        assert_eq!(jobs["recent_limit"], 8);
        assert_eq!(jobs["recent"].as_array().unwrap().len(), 8);
        assert_eq!(jobs["truncated"], true);
        assert!(
            jobs.get("active_job").is_none(),
            "attention must not infer an exact Session continuation"
        );
        if !disconnected {
            assert_eq!(jobs["stop_requested_count"], 1);
            assert_eq!(jobs["running_count"], 9);
            assert_eq!(jobs["recovering_count"], 0);
        } else {
            assert!(jobs["recovering_count"].as_u64().unwrap() > 0);
        }
        for job in jobs["recent"].as_array().unwrap() {
            assert!(expected.iter().any(|id| job["job_id"] == *id));
            let keys = job
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>();
            assert!(keys.iter().all(|key| [
                "job_id",
                "kind",
                "status",
                "project",
                "started_at",
                "created_at",
                "executor"
            ]
            .contains(key)));
        }
        let serialized = jobs.to_string();
        for id in &excluded {
            assert!(!serialized.contains(id));
        }
        for forbidden in [
            "stdout",
            "stderr",
            "command",
            "command_preview",
            "observation_token",
            "session_id",
            "runner_instance_id",
            "client_id",
            "hostname",
            "credentials",
            "cwd",
            "/tmp/",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "leaked {forbidden}: {serialized}"
            );
        }
        assert!(
            serde_json::to_vec(&result.output["context_projection"])
                .unwrap()
                .len()
                <= MAX_CONTEXT_PROJECTION_BYTES
        );
    }
}

async fn stop_with_attention(
    runtime: &ToolRuntime,
    project: &str,
    job_id: &str,
    confirm: bool,
    auth: &crate::auth::AuthContext,
) -> Option<ToolResult> {
    Box::pin(runtime.call_tool_with_invocation_metadata(
        ToolCallRequest {
            tool_name: "stop_job".into(),
            arguments: json!({"project": project, "job_id": job_id, "confirm": confirm}),
        },
        ToolCallContext {
            transport: ToolTransport::Mcp,
            session_id: None,
            auth: Some(auth),
            window: None,
            record_oauth_scope_denials: true,
            host_file_import_trust: HostFileImportTrust::Untrusted,
        },
        ToolInvocationMetadata {
            context_request: vec!["jobs.attention".into()],
            ..Default::default()
        },
        ToolProtocolCapabilities {
            context_sidecar: true,
            ..Default::default()
        },
    ))
    .await
    .result
}

#[tokio::test]
async fn jobs_attention_is_post_stop_effect_and_cannot_authorize_control() {
    let runtime = ToolRuntime::new_for_tests();
    let auth = shared_key_auth_context("attention-control-owner");
    let client = "attention-control";
    register_job_agent_for_auth(&runtime, client, "repo", &auth).await;
    let project = format!("agent:{client}:repo");
    let job_id = start_agent_runtime_job_in_session(&runtime, client, "repo", None, &auth).await;
    assert_eq!(mark_next_agent_job_running(&runtime, client).await, job_id);
    let mut observer = auth.clone();
    observer
        .scopes
        .retain(|scope| scope != crate::auth::SCOPE_JOB_RUN);
    assert!(observer.has_scope(crate::auth::SCOPE_RUNTIME_READ));
    let denied = stop_with_attention(&runtime, &project, &job_id, true, &observer).await;
    assert!(
        denied.is_none(),
        "kernel must deny missing stop scope before dispatch"
    );
    let unconfirmed = stop_with_attention(&runtime, &project, &job_id, false, &auth)
        .await
        .unwrap();
    assert!(!unconfirmed.success);
    assert_eq!(
        context_material(&unconfirmed, "jobs.attention")["projection"]["stop_requested_count"],
        0
    );
    assert!(probe_agent_request_for_instance(&runtime, client, "inst")
        .await
        .is_none());
    let stopped = stop_with_attention(&runtime, &project, &job_id, true, &auth)
        .await
        .unwrap();
    assert!(stopped.success, "{:?}", stopped.error);
    assert_eq!(stopped.output["stop_request_accepted"], true);
    assert_eq!(
        context_material(&stopped, "jobs.attention")["projection"]["stop_requested_count"],
        1,
        "sidecar must observe the committed main mutation, not the previous state"
    );
    assert!(
        stopped.output.get("permission").is_none(),
        "model projection must not grow a new permission receipt"
    );
    let request = wait_for_runner_request_for_instance(&runtime, client, "inst").await;
    assert_eq!(request.kind, "stop_job");
    assert_eq!(request.job_id.as_deref(), Some(job_id.as_str()));
    let replay = stop_with_attention(&runtime, &project, &job_id, true, &auth)
        .await
        .unwrap();
    assert!(replay.success, "{:?}", replay.error);
    assert_eq!(replay.output["already_stop_requested"], true);
    assert_eq!(replay.output["stop_request_accepted"], false);
    assert!(probe_agent_request_for_instance(&runtime, client, "inst")
        .await
        .is_none());
    for result in [&stopped, &replay] {
        for forbidden in ["stdout", "stderr", "command_preview", "runner_instance_id"] {
            assert!(!result.output.to_string().contains(forbidden));
        }
    }
}

#[tokio::test]
async fn jobs_attention_byte_budget_is_nonfatal_with_large_instructions() {
    let root = tempfile::tempdir().unwrap();
    init_git_repo(root.path());
    std::fs::write(
        root.path().join("AGENTS.md"),
        "Project guidance.\n".repeat(MAX_CONTEXT_PROJECTION_BYTES),
    )
    .unwrap();
    let runtime = ToolRuntime::new_for_tests();
    let client = "attention-budget";
    let project = register_runner_project_at_path(&runtime, client, "repo", root.path()).await;
    let result = dispatch_with_context_and_local_agent(
        &runtime,
        client,
        ToolCall::GitStatus {
            project,
            session_id: None,
        },
        vec![
            "jobs.attention".into(),
            "project.instructions".into(),
            "webcodex.workflow".into(),
        ],
    )
    .await;
    assert!(result.success, "{:?}", result.error);
    assert_eq!(
        context_material(&result, "jobs.attention")["status"],
        "available"
    );
    assert!(
        serde_json::to_vec(&result.output["context_projection"])
            .unwrap()
            .len()
            <= MAX_CONTEXT_PROJECTION_BYTES
    );
}
