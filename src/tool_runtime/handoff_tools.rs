//! Runtime dispatch adapter for explicit handoff recovery.

use super::{ToolCall, ToolResult, ToolRuntime};
use crate::auth::AuthContext;

impl ToolRuntime {
    pub(crate) async fn dispatch_handoff_tool(
        &self,
        call: ToolCall,
        auth: Option<&AuthContext>,
    ) -> ToolResult {
        match call {
            ToolCall::SessionHandoffSummary {
                session_id,
                project,
                include_workspace,
                include_checkpoints,
                include_validation,
                diagnostic,
                limit,
            } => {
                self.session_handoff_summary(
                    session_id,
                    project,
                    include_workspace,
                    include_checkpoints,
                    include_validation,
                    diagnostic,
                    limit,
                    auth,
                )
                .await
            }
            ToolCall::SessionHandoffState {
                project,
                session_id,
            } => {
                self.session_handoff_summary(
                    session_id,
                    Some(project),
                    None,
                    None,
                    None,
                    false,
                    None,
                    auth,
                )
                .await
            }
            _ => unreachable!("non-handoff tool routed to handoff dispatcher"),
        }
    }
}
