use std::sync::Arc;

use minicode_permissions::{PermissionDecision, PermissionPromptHandler, PermissionPromptResult};
use tokio::sync::{mpsc, oneshot};

use crate::state::TurnEvent;

/// 构造将权限请求转发到 UI 的回调处理器。
pub(crate) fn build_prompt_handler(
    tx: mpsc::UnboundedSender<TurnEvent>,
) -> PermissionPromptHandler {
    Arc::new(move |request| {
        let event_tx = tx.clone();
        Box::pin(async move {
            let (decision_tx, decision_rx) = oneshot::channel();
            if event_tx
                .send(TurnEvent::Approval {
                    request,
                    responder: decision_tx,
                })
                .is_err()
            {
                return PermissionPromptResult {
                    decision: PermissionDecision::DenyOnce,
                    feedback: None,
                };
            }
            match decision_rx.await {
                Ok(v) => v,
                Err(_) => PermissionPromptResult {
                    decision: PermissionDecision::DenyOnce,
                    feedback: None,
                },
            }
        })
    })
}
