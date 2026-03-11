use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rmcp::{
    model::{CallToolRequestParams, CallToolResult, Content, ListToolsResult},
    ErrorData as McpError,
};
use serde_json::Value;
use tokio::sync::Mutex;

use super::child::ChildHandle;
use super::relay::build_jsonrpc_request;

pub async fn proxy_call_tool(
    debug_child: &Arc<Mutex<Option<ChildHandle>>>,
    next_id: &AtomicU64,
    proxy_mode: &Arc<AtomicBool>,
    request: CallToolRequestParams,
) -> Result<CallToolResult, McpError> {
    let params = serde_json::json!({
        "name": request.name,
        "arguments": request.arguments.unwrap_or_default(),
    });
    let raw = relay_to_debug_child(debug_child, next_id, proxy_mode, "tools/call", params).await?;
    extract_call_result(&raw)
}

pub async fn proxy_list_tools(
    debug_child: &Arc<Mutex<Option<ChildHandle>>>,
    next_id: &AtomicU64,
    proxy_mode: &Arc<AtomicBool>,
) -> Result<ListToolsResult, McpError> {
    let raw = relay_to_debug_child(
        debug_child,
        next_id,
        proxy_mode,
        "tools/list",
        serde_json::json!({}),
    )
    .await?;
    extract_list_result(&raw)
}

async fn relay_to_debug_child(
    debug_child: &Arc<Mutex<Option<ChildHandle>>>,
    next_id: &AtomicU64,
    proxy_mode: &Arc<AtomicBool>,
    method: &str,
    params: Value,
) -> Result<String, McpError> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let req_json = build_jsonrpc_request(id, method, params);

    let guard = debug_child.lock().await;
    let Some(child) = guard.as_ref() else {
        revert_proxy_mode(proxy_mode);
        return Err(McpError::internal_error(
            "debug child is gone; reverted to normal mode",
            None,
        ));
    };

    match child.relay(&req_json).await {
        Ok(raw) => Ok(raw),
        Err(e) => {
            let crashed = !child.is_alive().await;
            drop(guard);
            if crashed {
                kill_and_clear_debug_child(debug_child).await;
                revert_proxy_mode(proxy_mode);
                tracing::warn!("debug child crashed; reverted to normal mode: {e}");
                return Err(McpError::internal_error(
                    format!("debug child crashed; reverted to normal mode. Error: {e}"),
                    None,
                ));
            }
            Err(McpError::internal_error(
                format!("relay to debug child failed: {e}"),
                None,
            ))
        }
    }
}

fn revert_proxy_mode(proxy_mode: &Arc<AtomicBool>) {
    proxy_mode.store(false, Ordering::Relaxed);
}

async fn kill_and_clear_debug_child(debug_child: &Arc<Mutex<Option<ChildHandle>>>) {
    let mut guard = debug_child.lock().await;
    if let Some(old) = guard.take() {
        old.kill().await;
    }
}

fn extract_call_result(raw: &str) -> Result<CallToolResult, McpError> {
    let val = parse_response(raw)?;

    if let Some(err) = val.get("error") {
        let msg = err["message"]
            .as_str()
            .unwrap_or("unknown error from debug child");
        return Ok(CallToolResult::error(vec![Content::text(msg)]));
    }

    let result = val
        .get("result")
        .ok_or_else(|| McpError::internal_error("missing result field in child response", None))?;

    serde_json::from_value(result.clone()).map_err(|e| {
        McpError::internal_error(
            format!("failed to deserialize CallToolResult from child: {e}"),
            None,
        )
    })
}

fn extract_list_result(raw: &str) -> Result<ListToolsResult, McpError> {
    let val = parse_response(raw)?;

    let result = val
        .get("result")
        .ok_or_else(|| McpError::internal_error("missing result field in child response", None))?;

    serde_json::from_value(result.clone()).map_err(|e| {
        McpError::internal_error(
            format!("failed to deserialize ListToolsResult from child: {e}"),
            None,
        )
    })
}

fn parse_response(raw: &str) -> Result<Value, McpError> {
    serde_json::from_str(raw).map_err(|e| {
        McpError::internal_error(
            format!("failed to parse debug child response as JSON: {e}"),
            None,
        )
    })
}
