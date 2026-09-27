use super::{parse_json_body, render_result, require_runtime};
use crate::action_audit::ActionAudit;
use salvo::prelude::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ResolveOrRegisterProjectRequest {
    client_id: String,
    path: String,
}

/// `POST /api/projects/resolve-or-register` — hidden operator path bootstrap.
/// The request carries only an exact Runner identity and path, then delegates to
/// the same ModelHidden ToolRuntime convergence primitive used by workflow
/// bootstrap. The Server never writes Runner project TOML here.
#[handler]
pub async fn projects_resolve_or_register(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) {
    let audit = ActionAudit::start(
        req,
        depot,
        "/api/projects/resolve-or-register",
        "resolveOrRegisterProject",
    );
    let Some(runtime) = require_runtime(depot, res) else {
        return;
    };
    let Some(body) = parse_json_body::<ResolveOrRegisterProjectRequest>(req, res).await else {
        return;
    };
    let auth = depot.obtain::<crate::auth::AuthContext>().ok().cloned();
    let result = runtime
        .resolve_or_register_project(body.client_id, body.path, auth.as_ref())
        .await;
    render_result(res, &audit, "resolve_or_register_project", None, result);
}
