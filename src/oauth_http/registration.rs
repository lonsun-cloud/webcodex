use salvo::http::header::CONTENT_TYPE;
use salvo::prelude::*;
use serde::Deserialize;

use crate::auth::{generate_oauth_client_id, generate_oauth_client_secret, hash_token};
use crate::models::OAuthClientRecord;

use super::{apply_oauth_no_store_headers, validate_redirect_uri};

const MAX_REGISTRATION_BODY_BYTES: usize = 16 * 1024;
const MAX_ACTIVE_DYNAMIC_CLIENTS: usize = 256;
const DYNAMIC_CLIENT_NAME_PREFIX: &str = "[dynamic] ";
/// Default scopes granted when a dynamic client omits `scope`. An explicit
/// `scope` request may use anything advertised as `scopes_supported`.
const DYNAMIC_CLIENT_SCOPES: &[&str] = &[
    "runtime:read",
    "project:read",
    "project:write",
    "job:run",
    "computer:read",
    "computer:control",
    "offline_access",
];

#[derive(Debug, Deserialize)]
struct RegistrationRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    application_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

fn registration_error(
    res: &mut Response,
    status: StatusCode,
    error: &'static str,
    description: impl Into<String>,
) {
    apply_oauth_no_store_headers(res);
    res.status_code(status);
    res.render(Json(serde_json::json!({
        "error": error,
        "error_description": description.into(),
    })));
}

fn validate_exact_values(
    values: Option<Vec<String>>,
    defaults: &[&str],
    allowed: &[&str],
) -> Result<Vec<String>, &'static str> {
    let values = values.unwrap_or_else(|| defaults.iter().map(|value| (*value).into()).collect());
    if values.is_empty()
        || values
            .iter()
            .any(|value| !allowed.contains(&value.as_str()))
        || values
            .iter()
            .enumerate()
            .any(|(index, value)| values[..index].contains(value))
    {
        return Err("registration metadata contains an unsupported or duplicate value");
    }
    Ok(values)
}

fn normalize_scopes(requested: Option<String>) -> Result<Vec<String>, &'static str> {
    let Some(requested) = requested else {
        return Ok(DYNAMIC_CLIENT_SCOPES
            .iter()
            .map(|scope| (*scope).to_string())
            .collect());
    };
    let mut scopes = Vec::new();
    for scope in requested.split_ascii_whitespace() {
        let is_supported = super::scope_registry::oauth_scopes_supported().contains(&scope)
            || scope == super::scope_registry::OAUTH_OFFLINE_ACCESS_SCOPE;
        if !is_supported || scopes.iter().any(|item| item == scope) {
            return Err("requested scope is not available to dynamically registered clients");
        }
        scopes.push(scope.to_string());
    }
    if scopes.is_empty() {
        return Err("scope must contain at least one supported value");
    }
    Ok(scopes)
}

/// RFC 7591 dynamic client registration for MCP clients such as Notion.
///
/// WebCodex deliberately registers confidential clients only. If a client asks
/// for `none`, the response reports the effective `client_secret_post` method
/// and returns a one-time secret. The secret is stored only as a hash.
#[handler]
pub(crate) async fn oauth_register(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let Some(config) = crate::auth::get_config(depot) else {
        registration_error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "OAuth configuration is unavailable",
        );
        return;
    };
    if !config.oauth2.enabled || !config.oauth2.dynamic_client_registration_enabled {
        registration_error(
            res,
            StatusCode::NOT_FOUND,
            "invalid_client_metadata",
            "dynamic client registration is not enabled",
        );
        return;
    }

    let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        registration_error(
            res,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_client_metadata",
            "Content-Type must be application/json",
        );
        return;
    }
    if req
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_REGISTRATION_BODY_BYTES)
    {
        registration_error(
            res,
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid_client_metadata",
            "registration request is too large",
        );
        return;
    }
    let payload = match req.payload().await {
        Ok(payload) if payload.len() <= MAX_REGISTRATION_BODY_BYTES => payload,
        Ok(_) => {
            registration_error(
                res,
                StatusCode::PAYLOAD_TOO_LARGE,
                "invalid_client_metadata",
                "registration request is too large",
            );
            return;
        }
        Err(_) => {
            registration_error(
                res,
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "registration request body could not be read",
            );
            return;
        }
    };
    let registration: RegistrationRequest = match serde_json::from_slice(payload) {
        Ok(registration) => registration,
        Err(_) => {
            registration_error(
                res,
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "registration request must be valid JSON",
            );
            return;
        }
    };

    if registration.redirect_uris.is_empty() || registration.redirect_uris.len() > 16 {
        registration_error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris must contain unique absolute HTTPS or loopback callback URLs",
        );
        return;
    }
    let mut redirect_uris = Vec::with_capacity(registration.redirect_uris.len());
    for uri in &registration.redirect_uris {
        let uri = uri.trim().to_string();
        if uri.len() > 2048 || validate_redirect_uri(&uri).is_err() || redirect_uris.contains(&uri)
        {
            registration_error(
                res,
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "redirect_uris must contain unique absolute HTTPS or loopback callback URLs",
            );
            return;
        }
        redirect_uris.push(uri);
    }

    let grant_types = match validate_exact_values(
        registration.grant_types,
        &["authorization_code", "refresh_token"],
        &["authorization_code", "refresh_token"],
    ) {
        Ok(values) if values.iter().any(|value| value == "authorization_code") => values,
        _ => {
            registration_error(
                res,
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "grant_types must include authorization_code and may include refresh_token",
            );
            return;
        }
    };
    let response_types =
        match validate_exact_values(registration.response_types, &["code"], &["code"]) {
            Ok(values) => values,
            Err(description) => {
                registration_error(
                    res,
                    StatusCode::BAD_REQUEST,
                    "invalid_client_metadata",
                    description,
                );
                return;
            }
        };
    if !matches!(
        registration.token_endpoint_auth_method.as_deref(),
        None | Some("none") | Some("client_secret_post")
    ) {
        registration_error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "token_endpoint_auth_method must be none or client_secret_post",
        );
        return;
    }
    let application_type = registration.application_type.as_deref().unwrap_or("web");
    if !matches!(application_type, "web" | "native") {
        registration_error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "application_type must be web or native",
        );
        return;
    }
    let scopes = match normalize_scopes(registration.scope) {
        Ok(scopes) => scopes,
        Err(description) => {
            registration_error(
                res,
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                description,
            );
            return;
        }
    };

    let Some(db) = crate::auth::get_db(depot) else {
        registration_error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "OAuth storage is unavailable",
        );
        return;
    };
    let users = match db.list_users() {
        Ok(users) => users,
        Err(_) => {
            registration_error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "OAuth storage is unavailable",
            );
            return;
        }
    };
    let Some(owner) = users.into_iter().find(|user| !user.is_disabled()) else {
        registration_error(
            res,
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "no enabled managed user is available for dynamic clients",
        );
        return;
    };
    let active_dynamic_clients = match db.list_oauth_clients() {
        Ok(clients) => clients
            .iter()
            .filter(|client| {
                client.revoked_at.is_none() && client.name.starts_with(DYNAMIC_CLIENT_NAME_PREFIX)
            })
            .count(),
        Err(_) => {
            registration_error(
                res,
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "OAuth storage is unavailable",
            );
            return;
        }
    };
    if active_dynamic_clients >= MAX_ACTIVE_DYNAMIC_CLIENTS {
        registration_error(
            res,
            StatusCode::TOO_MANY_REQUESTS,
            "temporarily_unavailable",
            "dynamic client registration limit reached",
        );
        return;
    }

    let requested_name = registration
        .client_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("MCP client");
    if requested_name.len() > 128 {
        registration_error(
            res,
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "client_name is too long",
        );
        return;
    }

    let client_id = generate_oauth_client_id();
    let client_secret = generate_oauth_client_secret();
    let issued_at = chrono::Utc::now().timestamp();
    let refresh_token_mode = if grant_types.iter().any(|value| value == "refresh_token")
        && config
            .oauth2
            .dynamic_client_uses_confidential_reuse(&redirect_uris)
    {
        crate::OAuthRefreshTokenMode::ConfidentialReuse
    } else {
        crate::OAuthRefreshTokenMode::Rotating
    };
    let record = OAuthClientRecord {
        id: uuid::Uuid::new_v4().to_string(),
        client_id: client_id.clone(),
        client_secret_hash: hash_token(&client_secret),
        name: format!("{DYNAMIC_CLIENT_NAME_PREFIX}{requested_name}"),
        redirect_uris: redirect_uris.join("\n"),
        allowed_scopes: scopes.join(" "),
        owner_user_id: Some(owner.id),
        owner_project_grant_id: None,
        owner_shared_key_hash: None,
        created_at: issued_at,
        revoked_at: None,
    };
    if db
        .insert_oauth_client_with_refresh_token_mode(&record, refresh_token_mode)
        .is_err()
    {
        registration_error(
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "client registration could not be stored",
        );
        return;
    }

    apply_oauth_no_store_headers(res);
    res.status_code(StatusCode::CREATED);
    res.render(Json(serde_json::json!({
        "client_id": client_id,
        "client_secret": client_secret,
        "client_id_issued_at": issued_at,
        "client_secret_expires_at": 0,
        "client_name": requested_name,
        "redirect_uris": redirect_uris,
        "grant_types": grant_types,
        "response_types": response_types,
        "application_type": application_type,
        "token_endpoint_auth_method": "client_secret_post",
        "scope": scopes.join(" "),
    })));
}
