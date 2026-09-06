use super::*;

fn request_body() -> serde_json::Value {
    serde_json::json!({
        "client_name": "Notion Custom MCP",
        "redirect_uris": ["https://notion.example/oauth/callback"],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    })
}

#[tokio::test]
async fn oauth_dynamic_registration_is_disabled_by_default() {
    let config = test_config(oauth2_enabled());
    let (_tmp, db) = test_db();
    let service = Service::new(build_router(config, db));
    let resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::NOT_FOUND));
}

#[tokio::test]
async fn oauth_dynamic_registration_requires_an_enabled_managed_user() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    let service = Service::new(build_router(config, db));
    let resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::SERVICE_UNAVAILABLE));
}

#[tokio::test]
async fn oauth_dynamic_registration_returns_confidential_least_privilege_client() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    let owner = seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    assert_eq!(
        resp.headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert!(body["client_id"]
        .as_str()
        .unwrap()
        .starts_with("wc_client_"));
    let secret = body["client_secret"].as_str().unwrap();
    assert!(secret.starts_with("wc_csec_"));
    assert_eq!(body["client_secret_expires_at"], 0);
    assert_eq!(body["application_type"], "web");
    assert_eq!(body["token_endpoint_auth_method"], "client_secret_post");
    assert_eq!(
        body["redirect_uris"],
        serde_json::json!(["https://notion.example/oauth/callback"])
    );
    assert_eq!(
        body["scope"],
        "runtime:read runner:manage session:collaborate communication:read communication:manage project:read project:write memory:read memory:manage job:run job:detach computer:read computer:control computer:launch computer:display_read computer:pointer_control computer:clipboard_read computer:clipboard_write mcp:local plugin:inspect plugin:invoke plugin:manage ssh:local coding_agent:run account:manage offline_access"
    );
    assert!(body["scope"].as_str().unwrap().contains("account:manage"));
    assert!(!body["scope"].as_str().unwrap().contains("admin"));

    let clients = db.list_oauth_clients().unwrap();
    assert_eq!(clients.len(), 1);
    let stored = &clients[0];
    assert_eq!(stored.owner_user_id.as_deref(), Some(owner.id.as_str()));
    assert_eq!(stored.client_secret_hash, crate::auth::hash_token(secret));
    assert!(!stored.client_secret_hash.contains(secret));
    assert_eq!(
        stored.redirect_uris,
        "https://notion.example/oauth/callback"
    );
    assert_eq!(
        stored.allowed_scopes,
        "runtime:read runner:manage session:collaborate communication:read communication:manage project:read project:write memory:read memory:manage job:run job:detach computer:read computer:control computer:launch computer:display_read computer:pointer_control computer:clipboard_read computer:clipboard_write mcp:local plugin:inspect plugin:invoke plugin:manage ssh:local coding_agent:run account:manage offline_access"
    );
}

#[tokio::test]
async fn oauth_dynamic_registration_normalizes_redirect_uri_name_and_scope_subset() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut body = request_body();
    body["client_name"] = serde_json::json!("  Notion Custom MCP  ");
    body["redirect_uris"] = serde_json::json!(["  https://notion.example/oauth/callback  "]);
    body["application_type"] = serde_json::json!("web");
    body["scope"] = serde_json::json!("runtime:read project:read");

    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let response: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(response["client_name"], "Notion Custom MCP");
    assert_eq!(response["application_type"], "web");
    assert_eq!(
        response["redirect_uris"],
        serde_json::json!(["https://notion.example/oauth/callback"])
    );
    assert_eq!(response["scope"], "runtime:read project:read");

    let clients = db.list_oauth_clients().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].name, "[dynamic] Notion Custom MCP");
    assert_eq!(
        clients[0].redirect_uris,
        "https://notion.example/oauth/callback"
    );
    assert_eq!(clients[0].allowed_scopes, "runtime:read project:read");
}

#[tokio::test]
async fn oauth_dynamic_registration_rejects_invalid_or_normalized_duplicate_redirect_uris() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db));

    for redirect_uris in [
        serde_json::json!(["http://notion.example/oauth/callback"]),
        serde_json::json!([
            " https://notion.example/oauth/callback ",
            "https://notion.example/oauth/callback"
        ]),
    ] {
        let mut body = request_body();
        body["redirect_uris"] = redirect_uris;
        let mut resp = TestClient::post("http://localhost/oauth/register")
            .json(&body)
            .send(&service)
            .await;
        assert_eq!(resp.status_code, Some(StatusCode::BAD_REQUEST));
        let error: serde_json::Value = resp.take_json().await.unwrap();
        assert_eq!(error["error"], "invalid_redirect_uri");
    }
}

#[tokio::test]
async fn oauth_dynamic_registration_rejects_disabled_owner() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    let owner = seed_user(&db, "notion-owner");
    db.set_user_disabled(&owner.id, true, chrono::Utc::now().timestamp())
        .unwrap();
    let service = Service::new(build_router(config, db));
    let resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::SERVICE_UNAVAILABLE));
}

#[tokio::test]
async fn oauth_dynamic_registration_requires_json_and_limits_request_size() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    let service = Service::new(build_router(config, db));

    let wrong_type = TestClient::post("http://localhost/oauth/register")
        .add_header("content-type", "text/plain", true)
        .body(request_body().to_string())
        .send(&service)
        .await;
    assert_eq!(
        wrong_type.status_code,
        Some(StatusCode::UNSUPPORTED_MEDIA_TYPE)
    );

    let oversized = TestClient::post("http://localhost/oauth/register")
        .add_header("content-type", "application/json", true)
        .body("x".repeat(16 * 1024 + 1))
        .send(&service)
        .await;
    assert_eq!(oversized.status_code, Some(StatusCode::PAYLOAD_TOO_LARGE));
}

#[tokio::test]
async fn oauth_dynamic_registration_rejects_privileged_scope() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db));
    let mut body = request_body();
    body["scope"] = serde_json::json!("runtime:read admin");
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::BAD_REQUEST));
    let error: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(error["error"], "invalid_client_metadata");
}

#[tokio::test]
async fn oauth_dynamic_registration_accepts_full_advertised_scope_set() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db));
    let mut body = request_body();
    body["scope"] = serde_json::json!(
        "runtime:read session:collaborate communication:read communication:manage project:read project:write memory:read memory:manage job:run job:detach computer:read computer:control computer:launch computer:display_read computer:pointer_control computer:clipboard_read computer:clipboard_write mcp:local coding_agent:run account:manage offline_access"
    );
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let response: serde_json::Value = resp.take_json().await.unwrap();
    let scope = response["scope"].as_str().unwrap();
    assert!(scope.contains("account:manage"));
    assert!(scope.contains("offline_access"));
}

#[tokio::test]
async fn oauth_dynamic_registration_rejects_unsupported_metadata() {
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db));

    for (field, value) in [
        ("grant_types", serde_json::json!(["client_credentials"])),
        ("response_types", serde_json::json!(["token"])),
        (
            "token_endpoint_auth_method",
            serde_json::json!("client_secret_basic"),
        ),
    ] {
        let mut body = request_body();
        body[field] = value;
        let resp = TestClient::post("http://localhost/oauth/register")
            .json(&body)
            .send(&service)
            .await;
        assert_eq!(resp.status_code, Some(StatusCode::BAD_REQUEST), "{field}");
    }
}

// -----------------------------------------------------------------------
// Refresh-token mode assignment (rotating default / confidential reuse)
// -----------------------------------------------------------------------

const TRUSTED_REUSE_REDIRECT: &str = "https://notion.example/oauth/callback";

fn oauth2_enabled_dcr_trusting_reuse_redirect() -> OAuth2Config {
    OAuth2Config {
        confidential_reuse_redirect_uris: vec![TRUSTED_REUSE_REDIRECT.to_string()],
        ..oauth2_enabled_dcr()
    }
}

fn stored_refresh_token_mode(
    db: &crate::Database,
    client_id: &str,
) -> crate::OAuthRefreshTokenMode {
    db.get_oauth_client_refresh_token_mode(client_id)
        .unwrap()
        .expect("registered client should have a refresh token mode")
}

#[tokio::test]
async fn oauth_dynamic_registration_defaults_to_rotating_refresh_mode() {
    // Without a confidential-reuse allow-list, even a client that could
    // refresh registers as rotating.
    let config = test_config(oauth2_enabled_dcr());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::Rotating
    );
}

#[tokio::test]
async fn oauth_dynamic_registration_trusted_redirect_gets_confidential_reuse() {
    let config = test_config(oauth2_enabled_dcr_trusting_reuse_redirect());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&request_body())
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::ConfidentialReuse
    );
    // The response stays a client_secret_post confidential client and never
    // exposes the server-internal refresh token mode.
    assert_eq!(body["token_endpoint_auth_method"], "client_secret_post");
    assert!(body.get("refresh_token_mode").is_none());
    assert!(body.get("refresh_mode").is_none());
}

#[tokio::test]
async fn oauth_dynamic_registration_client_name_does_not_grant_trust() {
    // The allow-list trusts a different URI than the one registered here;
    // claiming a well-known client name must not change the outcome.
    let config = test_config(OAuth2Config {
        confidential_reuse_redirect_uris: vec![
            "https://notion.example/trusted-callback".to_string()
        ],
        ..oauth2_enabled_dcr()
    });
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut body = request_body();
    body["client_name"] = serde_json::json!("Notion");
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::Rotating
    );
}

#[tokio::test]
async fn oauth_dynamic_registration_matches_redirect_uri_not_client_name() {
    let config = test_config(oauth2_enabled_dcr_trusting_reuse_redirect());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut body = request_body();
    body["client_name"] = serde_json::json!("Definitely Not Notion");
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::ConfidentialReuse
    );
}

#[tokio::test]
async fn oauth_dynamic_registration_with_extra_redirect_uri_stays_rotating() {
    let config = test_config(oauth2_enabled_dcr_trusting_reuse_redirect());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut body = request_body();
    body["redirect_uris"] = serde_json::json!([
        TRUSTED_REUSE_REDIRECT,
        "https://attacker.example/oauth/callback"
    ]);
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::Rotating
    );
}

#[tokio::test]
async fn oauth_dynamic_registration_without_refresh_token_grant_stays_rotating() {
    let config = test_config(oauth2_enabled_dcr_trusting_reuse_redirect());
    let (_tmp, db) = test_db();
    seed_user(&db, "notion-owner");
    let service = Service::new(build_router(config, db.clone()));
    let mut body = request_body();
    body["grant_types"] = serde_json::json!(["authorization_code"]);
    let mut resp = TestClient::post("http://localhost/oauth/register")
        .json(&body)
        .send(&service)
        .await;
    assert_eq!(resp.status_code, Some(StatusCode::CREATED));
    let body: serde_json::Value = resp.take_json().await.unwrap();
    assert_eq!(
        stored_refresh_token_mode(&db, body["client_id"].as_str().unwrap()),
        crate::OAuthRefreshTokenMode::Rotating
    );
}
