//! Project-to-Project artifact transfer protocol tests.

use super::super::sessions::SessionTransport;
use super::support::*;
use crate::auth::{AuthContext, AuthKind};
use crate::runner_protocol::RunnerCapabilities;
use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn transfer_caps(read: bool, write: bool) -> RunnerCapabilities {
    RunnerCapabilities {
        file_read: read,
        file_write: write,
        artifact_export_chunk_read: read,
        artifact_export_streaming_metadata: read,
        ..Default::default()
    }
}

fn transfer_auth(username: &str) -> AuthContext {
    AuthContext {
        kind: AuthKind::OAuth2Token,
        user_id: Some(format!("user-{username}")),
        username: Some(username.to_string()),
        api_key_id: Some(format!("oauth-{username}")),
        role: Some("user".to_string()),
        scopes: vec![
            crate::auth::SCOPE_PROJECT_READ.to_string(),
            crate::auth::SCOPE_PROJECT_WRITE.to_string(),
        ],
        is_bootstrap: false,
        token_kind: Some("oauth2".to_string()),
        allowed_client_id: Some("transfer-test-client".to_string()),
        shared_key_hash: None,
        project_grant_id: None,
    }
}

async fn complete_source_metadata(
    runtime: &super::super::ToolRuntime,
    client_id: &str,
    path: &str,
    bytes: &[u8],
    mime_type: Option<&str>,
) {
    let request = wait_for_patch_agent_request(runtime, client_id).await;
    assert_eq!(request.kind, "file_read_project_artifact_metadata");
    let payload: Value = serde_json::from_str(request.content.as_deref().unwrap()).unwrap();
    assert_eq!(payload["path"], path);
    assert_eq!(
        payload["max_bytes"],
        super::super::MAX_PROJECT_ARTIFACT_EXPORT_BYTES
    );
    complete_patch_agent_request(
        runtime,
        client_id,
        &request.request_id,
        0,
        &json!({
            "path": path,
            "exists": true,
            "missing": false,
            "bytes": bytes.len(),
            "sha256": sha256_hex(bytes),
            "mime_type": mime_type,
        })
        .to_string(),
        "",
    )
    .await;
}

async fn complete_destination_begin(
    runtime: &super::super::ToolRuntime,
    client_id: &str,
    destination_path: &str,
    bytes: &[u8],
    expected_mime: &str,
    overwrite: bool,
    upload_id: &str,
) {
    let request = wait_for_patch_agent_request(runtime, client_id).await;
    assert_eq!(request.kind, "file_artifact_upload_begin");
    let payload: Value = serde_json::from_str(request.content.as_deref().unwrap()).unwrap();
    assert_eq!(payload["path"], destination_path);
    assert_eq!(payload["expected_bytes"], bytes.len());
    assert_eq!(payload["expected_sha256"], sha256_hex(bytes));
    assert_eq!(payload["mime_type"], expected_mime);
    assert_eq!(payload["overwrite"], overwrite);
    complete_patch_agent_request(
        runtime,
        client_id,
        &request.request_id,
        0,
        &json!({
            "path": destination_path,
            "upload_id": upload_id,
            "received_bytes": 0,
            "next_offset": 0,
            "expected_bytes": bytes.len(),
            "expected_sha256": sha256_hex(bytes),
            "mime_type": expected_mime,
            "committed": false,
        })
        .to_string(),
        "",
    )
    .await;
}

async fn complete_one_transfer_chunk(
    runtime: &super::super::ToolRuntime,
    source_client: &str,
    destination_client: &str,
    source_path: &str,
    destination_path: &str,
    bytes: &[u8],
    offset: usize,
    upload_id: &str,
) -> usize {
    let source_request = wait_for_patch_agent_request(runtime, source_client).await;
    assert_eq!(
        source_request.kind,
        "file_read_project_artifact_export_chunk"
    );
    let source_payload: Value =
        serde_json::from_str(source_request.content.as_deref().unwrap()).unwrap();
    assert_eq!(source_payload["path"], source_path);
    assert_eq!(source_payload["expected_file_bytes"], bytes.len());
    assert_eq!(source_payload["expected_sha256"], sha256_hex(bytes));
    assert_eq!(source_payload["offset"], offset);
    let requested = source_payload["length"].as_u64().unwrap() as usize;
    let next = (offset + requested).min(bytes.len());
    let segment = &bytes[offset..next];
    let content_base64 = general_purpose::STANDARD.encode(segment);
    complete_patch_agent_request(
        runtime,
        source_client,
        &source_request.request_id,
        0,
        &json!({
            "path": source_path,
            "file_bytes": bytes.len(),
            "offset": offset,
            "bytes_returned": segment.len(),
            "content_base64": content_base64,
            "next_offset": next,
            "truncated": next < bytes.len(),
            "eof": next == bytes.len(),
        })
        .to_string(),
        "",
    )
    .await;

    let destination_request = wait_for_patch_agent_request(runtime, destination_client).await;
    assert_eq!(destination_request.kind, "file_artifact_upload_chunk");
    let destination_payload: Value =
        serde_json::from_str(destination_request.content.as_deref().unwrap()).unwrap();
    assert_eq!(destination_payload["path"], destination_path);
    assert_eq!(destination_payload["upload_id"], upload_id);
    assert_eq!(destination_payload["offset"], offset);
    let uploaded = general_purpose::STANDARD
        .decode(destination_payload["content_base64"].as_str().unwrap())
        .unwrap();
    assert_eq!(uploaded, segment);
    complete_patch_agent_request(
        runtime,
        destination_client,
        &destination_request.request_id,
        0,
        &json!({
            "path": destination_path,
            "upload_id": upload_id,
            "received_bytes": next,
            "next_offset": next,
            "expected_bytes": bytes.len(),
            "expected_sha256": sha256_hex(bytes),
            "committed": false,
        })
        .to_string(),
        "",
    )
    .await;
    next
}

async fn complete_destination_finish(
    runtime: &super::super::ToolRuntime,
    client_id: &str,
    destination_path: &str,
    bytes: &[u8],
    mime_type: &str,
    upload_id: &str,
) {
    let request = wait_for_patch_agent_request(runtime, client_id).await;
    assert_eq!(request.kind, "file_artifact_upload_finish");
    let payload: Value = serde_json::from_str(request.content.as_deref().unwrap()).unwrap();
    assert_eq!(payload["path"], destination_path);
    assert_eq!(payload["upload_id"], upload_id);
    complete_patch_agent_request(
        runtime,
        client_id,
        &request.request_id,
        0,
        &json!({
            "path": destination_path,
            "upload_id": upload_id,
            "bytes": bytes.len(),
            "received_bytes": bytes.len(),
            "expected_bytes": bytes.len(),
            "expected_sha256": sha256_hex(bytes),
            "sha256": sha256_hex(bytes),
            "mime_type": mime_type,
            "committed": true,
        })
        .to_string(),
        "",
    )
    .await;
}

async fn complete_destination_abort(
    runtime: &super::super::ToolRuntime,
    client_id: &str,
    destination_path: &str,
    upload_id: &str,
) {
    let request = wait_for_patch_agent_request(runtime, client_id).await;
    assert_eq!(request.kind, "file_artifact_upload_abort");
    let payload: Value = serde_json::from_str(request.content.as_deref().unwrap()).unwrap();
    assert_eq!(payload["path"], destination_path);
    assert_eq!(payload["upload_id"], upload_id);
    complete_patch_agent_request(
        runtime,
        client_id,
        &request.request_id,
        0,
        &json!({
            "path": destination_path,
            "upload_id": upload_id,
            "received_bytes": 0,
            "committed": false,
            "aborted": true,
            "final_file_exists": false,
        })
        .to_string(),
        "",
    )
    .await;
}

#[tokio::test]
async fn transfer_project_artifact_streams_markdown_across_runners() {
    let runtime = runtime_with_agent_project("transfer-runtime");
    register_agent(
        &runtime,
        "transfer-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "transfer-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let source_project = agent_test_project_id("transfer-source");
    let destination_project = agent_test_project_id("transfer-destination");
    let source_path = "paper/README.md";
    let destination_path = "artifacts/README.md";
    let bytes: Vec<u8> = (0..(super::super::INTERNAL_ARTIFACT_TRANSFER_CHUNK_BYTES + 17))
        .map(|index| b'a' + (index % 23) as u8)
        .collect();
    let expected_sha = sha256_hex(&bytes);
    let upload_id = "wc_upload_transfer_markdown";

    let auth = transfer_auth("alice");
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let source_project = source_project.clone();
        let destination_project = destination_project.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    source_project,
                    source_path.to_string(),
                    destination_project,
                    destination_path.to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });

    complete_source_metadata(
        &runtime,
        "transfer-source",
        source_path,
        &bytes,
        Some("text/markdown"),
    )
    .await;
    complete_destination_begin(
        &runtime,
        "transfer-destination",
        destination_path,
        &bytes,
        "text/markdown",
        false,
        upload_id,
    )
    .await;
    let mut offset = 0;
    let mut chunk_count = 0;
    while offset < bytes.len() {
        offset = complete_one_transfer_chunk(
            &runtime,
            "transfer-source",
            "transfer-destination",
            source_path,
            destination_path,
            &bytes,
            offset,
            upload_id,
        )
        .await;
        chunk_count += 1;
    }
    assert_eq!(
        chunk_count, 2,
        "1 MiB internal streaming should require two chunks"
    );

    complete_destination_finish(
        &runtime,
        "transfer-destination",
        destination_path,
        &bytes,
        "text/markdown",
        upload_id,
    )
    .await;

    let result = task.await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.output["source_project"], source_project);
    assert_eq!(result.output["destination_project"], destination_project);
    assert_eq!(result.output["bytes"], bytes.len());
    assert_eq!(result.output["sha256"], expected_sha);
    assert_eq!(result.output["mime_type"], "text/markdown");
    let serialized = serde_json::to_string(&result.output).unwrap();
    assert!(!serialized.contains("content_base64"));
}

#[tokio::test]
async fn transfer_project_artifact_unknown_binary_preserves_generic_mime_and_overwrite_true() {
    let runtime = runtime_with_agent_project("transfer-binary");
    register_agent(
        &runtime,
        "binary-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "binary-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let source_project = agent_test_project_id("binary-source");
    let destination_project = agent_test_project_id("binary-destination");
    let bytes = b"custom-binary-payload".to_vec();
    let source_path = "data/data.customblob";
    let destination_path = "artifacts/data.customblob";
    let upload_id = "wc_upload_transfer_binary";
    let auth = transfer_auth("alice");

    let task = tokio::spawn({
        let runtime = runtime.clone();
        let source_project = source_project.clone();
        let destination_project = destination_project.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    source_project,
                    source_path.to_string(),
                    destination_project,
                    destination_path.to_string(),
                    Some(true),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });

    complete_source_metadata(&runtime, "binary-source", source_path, &bytes, None).await;
    complete_destination_begin(
        &runtime,
        "binary-destination",
        destination_path,
        &bytes,
        "application/octet-stream",
        true,
        upload_id,
    )
    .await;
    complete_one_transfer_chunk(
        &runtime,
        "binary-source",
        "binary-destination",
        source_path,
        destination_path,
        &bytes,
        0,
        upload_id,
    )
    .await;
    complete_destination_finish(
        &runtime,
        "binary-destination",
        destination_path,
        &bytes,
        "application/octet-stream",
        upload_id,
    )
    .await;

    let result = task.await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.output["mime_type"], "application/octet-stream");
}

#[tokio::test]
async fn transfer_project_artifact_same_project_uses_existing_protocol() {
    let runtime = runtime_with_agent_project("same-transfer");
    register_agent(
        &runtime,
        "same-transfer",
        Some("alice"),
        transfer_caps(true, true),
    )
    .await;
    let project = agent_test_project_id("same-transfer");
    let bytes = b"same-project".to_vec();
    let source_path = "source.bin";
    let destination_path = "copy.bin";
    let upload_id = "wc_upload_same_project";
    let auth = transfer_auth("alice");

    let task = tokio::spawn({
        let runtime = runtime.clone();
        let project = project.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    project.clone(),
                    source_path.to_string(),
                    project,
                    destination_path.to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });

    complete_source_metadata(&runtime, "same-transfer", source_path, &bytes, None).await;
    complete_destination_begin(
        &runtime,
        "same-transfer",
        destination_path,
        &bytes,
        "application/octet-stream",
        false,
        upload_id,
    )
    .await;
    complete_one_transfer_chunk(
        &runtime,
        "same-transfer",
        "same-transfer",
        source_path,
        destination_path,
        &bytes,
        0,
        upload_id,
    )
    .await;
    complete_destination_finish(
        &runtime,
        "same-transfer",
        destination_path,
        &bytes,
        "application/octet-stream",
        upload_id,
    )
    .await;
    assert!(task.await.unwrap().success);
}

#[tokio::test]
async fn transfer_project_artifact_overwrite_false_is_definite_begin_failure() {
    let runtime = runtime_with_agent_project("overwrite-transfer");
    register_agent(
        &runtime,
        "overwrite-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "overwrite-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let source_project = agent_test_project_id("overwrite-source");
    let destination_project = agent_test_project_id("overwrite-destination");
    let bytes = b"payload".to_vec();
    let auth = transfer_auth("alice");
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    source_project,
                    "source.bin".to_string(),
                    destination_project,
                    "existing.bin".to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });
    complete_source_metadata(&runtime, "overwrite-source", "source.bin", &bytes, None).await;
    let request = wait_for_patch_agent_request(&runtime, "overwrite-destination").await;
    assert_eq!(request.kind, "file_artifact_upload_begin");
    complete_patch_agent_request(
        &runtime,
        "overwrite-destination",
        &request.request_id,
        0,
        r#"{"path":"existing.bin","error":"file exists and overwrite is false","failure_kind":"policy_rejected"}"#,
        "",
    )
    .await;
    let result = task.await.unwrap();
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "destination_begin_failed");
    assert_eq!(result.output["outcome_unknown"], false);
}

#[tokio::test]
async fn transfer_project_artifact_source_missing_stops_before_destination_write() {
    let runtime = runtime_with_agent_project("missing-transfer");
    register_agent(
        &runtime,
        "missing-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "missing-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let auth = transfer_auth("alice");
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    agent_test_project_id("missing-source"),
                    "missing.bin".to_string(),
                    agent_test_project_id("missing-destination"),
                    "copy.bin".to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });
    let request = wait_for_patch_agent_request(&runtime, "missing-source").await;
    assert_eq!(request.kind, "file_read_project_artifact_metadata");
    complete_patch_agent_request(
        &runtime,
        "missing-source",
        &request.request_id,
        0,
        r#"{"path":"missing.bin","error":"stat failed: not found"}"#,
        "",
    )
    .await;
    let result = task.await.unwrap();
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "source_read_failed");
    assert!(
        probe_agent_request_for_instance(&runtime, "missing-destination", "inst")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn transfer_project_artifact_source_snapshot_change_aborts_destination_upload() {
    let runtime = runtime_with_agent_project("snapshot-transfer");
    register_agent(
        &runtime,
        "snapshot-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "snapshot-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let source_project = agent_test_project_id("snapshot-source");
    let destination_project = agent_test_project_id("snapshot-destination");
    let bytes = b"original snapshot".to_vec();
    let upload_id = "wc_upload_snapshot_changed";
    let auth = transfer_auth("alice");
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    source_project,
                    "source.bin".to_string(),
                    destination_project,
                    "copy.bin".to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });
    complete_source_metadata(&runtime, "snapshot-source", "source.bin", &bytes, None).await;
    complete_destination_begin(
        &runtime,
        "snapshot-destination",
        "copy.bin",
        &bytes,
        "application/octet-stream",
        false,
        upload_id,
    )
    .await;
    let request = wait_for_patch_agent_request(&runtime, "snapshot-source").await;
    assert_eq!(request.kind, "file_read_project_artifact_export_chunk");
    complete_patch_agent_request(
        &runtime,
        "snapshot-source",
        &request.request_id,
        0,
        &json!({
            "path": "source.bin",
            "error": "artifact snapshot changed",
            "error_kind": "snapshot_changed",
            "expected_sha256": sha256_hex(&bytes),
            "actual_sha256": "f".repeat(64),
        })
        .to_string(),
        "",
    )
    .await;
    complete_destination_abort(&runtime, "snapshot-destination", "copy.bin", upload_id).await;
    let result = task.await.unwrap();
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "source_snapshot_changed");
    assert_eq!(result.output["destination_upload_aborted"], true);
}

#[tokio::test]
async fn transfer_project_artifact_destination_sha_failure_aborts_known_upload() {
    let runtime = runtime_with_agent_project("sha-transfer");
    register_agent(
        &runtime,
        "sha-source",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "sha-destination",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let bytes = b"sha-protected".to_vec();
    let upload_id = "wc_upload_sha_mismatch";
    let auth = transfer_auth("alice");
    let task = tokio::spawn({
        let runtime = runtime.clone();
        let auth = auth.clone();
        async move {
            runtime
                .transfer_project_artifact(
                    agent_test_project_id("sha-source"),
                    "source.bin".to_string(),
                    agent_test_project_id("sha-destination"),
                    "copy.bin".to_string(),
                    Some(false),
                    Some(&auth),
                    SessionTransport::Api,
                )
                .await
        }
    });
    complete_source_metadata(&runtime, "sha-source", "source.bin", &bytes, None).await;
    complete_destination_begin(
        &runtime,
        "sha-destination",
        "copy.bin",
        &bytes,
        "application/octet-stream",
        false,
        upload_id,
    )
    .await;
    complete_one_transfer_chunk(
        &runtime,
        "sha-source",
        "sha-destination",
        "source.bin",
        "copy.bin",
        &bytes,
        0,
        upload_id,
    )
    .await;
    let finish = wait_for_patch_agent_request(&runtime, "sha-destination").await;
    assert_eq!(finish.kind, "file_artifact_upload_finish");
    complete_patch_agent_request(
        &runtime,
        "sha-destination",
        &finish.request_id,
        0,
        &json!({
            "path": "copy.bin",
            "upload_id": upload_id,
            "received_bytes": bytes.len(),
            "expected_bytes": bytes.len(),
            "expected_sha256": sha256_hex(&bytes),
            "sha256": "0".repeat(64),
            "committed": false,
            "error": "uploaded sha256 does not match expected_sha256",
        })
        .to_string(),
        "",
    )
    .await;
    complete_destination_abort(&runtime, "sha-destination", "copy.bin", upload_id).await;
    let result = task.await.unwrap();
    assert!(!result.success);
    assert_eq!(result.output["error_kind"], "destination_finish_failed");
    assert_eq!(result.output["destination_upload_aborted"], true);
}

#[tokio::test]
async fn transfer_project_artifact_independently_authorizes_source_and_destination() {
    let runtime = runtime_with_agent_project("auth-transfer");
    register_agent(
        &runtime,
        "auth-source-alice",
        Some("alice"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "auth-destination-bob",
        Some("bob"),
        transfer_caps(false, true),
    )
    .await;
    let bob = transfer_auth("bob");
    let source_denied = runtime
        .transfer_project_artifact(
            agent_test_project_id("auth-source-alice"),
            "source.bin".to_string(),
            agent_test_project_id("auth-destination-bob"),
            "copy.bin".to_string(),
            Some(false),
            Some(&bob),
            SessionTransport::Api,
        )
        .await;
    assert!(!source_denied.success);

    let runtime = runtime_with_agent_project("auth-transfer-2");
    register_agent(
        &runtime,
        "auth-source-bob",
        Some("bob"),
        transfer_caps(true, false),
    )
    .await;
    register_agent(
        &runtime,
        "auth-destination-alice",
        Some("alice"),
        transfer_caps(false, true),
    )
    .await;
    let destination_denied = runtime
        .transfer_project_artifact(
            agent_test_project_id("auth-source-bob"),
            "source.bin".to_string(),
            agent_test_project_id("auth-destination-alice"),
            "copy.bin".to_string(),
            Some(false),
            Some(&bob),
            SessionTransport::Api,
        )
        .await;
    assert!(!destination_denied.success);
    assert!(
        probe_agent_request_for_instance(&runtime, "auth-source-bob", "inst")
            .await
            .is_none()
    );
}
