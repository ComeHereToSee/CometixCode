//! MCP elicitation service boundary.
//! Maps to: CC `services/mcp/elicitationHandler.ts`.
//!
//! This module owns the data model of MCP elicitation requests, the handlers
//! `registerElicitationHandler` puts on a connection, and the elicitation
//! hooks. The terminal UI lives in `components/mcp/elicitation_dialog.rs`;
//! the SDK side (dispatch to a connection's handler, the default handler, the
//! URL-mode refusal) and the tool call's -32042 retry stay in
//! `services/mcp/client.rs`, as in CC. The bodies of a queued event's
//! callbacks (`respond`, and the retry's `onWaitingDismiss`) live here with
//! the event, which is plain state; the hook results are reduced here too,
//! though CC reduces them in `utils/hooks.ts`.

use super::client::McpClient;
use crate::services::hooks::{HookBlockingError, HookElicitationResponse, HookResult};
use rmcp::model::ElicitRequestParams as RmcpElicitRequestParams;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::{Mutex, oneshot};

/// Maps to: CC `ElicitationWaitingState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElicitationWaitingState {
    pub action_label: String,
    pub show_cancel: bool,
}

/// Maps to: CC `onWaitingDismiss` action union.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElicitationWaitingDismissAction {
    Dismiss,
    Retry,
    Cancel,
}

/// Maps to: CC `ElicitRequestParams` mode split.
#[derive(Clone, Debug, PartialEq)]
pub enum ElicitationRequestParams {
    Form {
        message: String,
        requested_schema: Value,
    },
    Url {
        message: String,
        url: String,
        elicitation_id: Option<String>,
    },
}

/// Maps to: CC `getElicitationMode(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElicitationMode {
    Form,
    Url,
}

impl ElicitationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Form => "form",
            Self::Url => "url",
        }
    }
}

pub fn get_elicitation_mode(params: &ElicitationRequestParams) -> ElicitationMode {
    match params {
        ElicitationRequestParams::Form { .. } => ElicitationMode::Form,
        ElicitationRequestParams::Url { .. } => ElicitationMode::Url,
    }
}

/// Maps to: CC `ElicitResult['action']`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

impl ElicitationAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Decline => "decline",
            Self::Cancel => "cancel",
        }
    }
}

impl From<&str> for ElicitationAction {
    fn from(value: &str) -> Self {
        match value {
            "accept" => Self::Accept,
            "decline" => Self::Decline,
            _ => Self::Cancel,
        }
    }
}

/// Maps to: CC `ElicitResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct ElicitationResult {
    pub action: ElicitationAction,
    pub content: Option<Value>,
}

impl ElicitationResult {
    pub fn new(action: ElicitationAction) -> Self {
        Self {
            action,
            content: None,
        }
    }
}

/// Maps to: CC `ElicitationRequestEvent` minus the JS callback/signal fields.
#[derive(Clone, Debug, PartialEq)]
pub struct ElicitationRequestEvent {
    pub server_name: String,
    pub request_id: String,
    pub params: ElicitationRequestParams,
    pub waiting_state: Option<ElicitationWaitingState>,
    pub completed: bool,
}

impl ElicitationRequestEvent {
    pub fn new(
        server_name: impl Into<String>,
        request_id: impl Into<String>,
        params: ElicitationRequestParams,
    ) -> Self {
        let waiting_state = match &params {
            ElicitationRequestParams::Url {
                elicitation_id: Some(_),
                ..
            } => Some(ElicitationWaitingState {
                action_label: "Skip confirmation".to_string(),
                show_cancel: false,
            }),
            _ => None,
        };
        Self {
            server_name: server_name.into(),
            request_id: request_id.into(),
            params,
            waiting_state,
            completed: false,
        }
    }
}

/// Maps to: CC `onWaitingDismiss` in `callMCPToolWithUrlElicitationRetry(...)`
/// (`client.ts:2984-2991`). The event is plain state, so the body of this
/// callback field lives here with it, as `respond`'s carrier does.
pub fn url_retry_waiting_dismiss_result(
    action: ElicitationWaitingDismissAction,
) -> ElicitationResult {
    match action {
        ElicitationWaitingDismissAction::Retry => ElicitationResult::new(ElicitationAction::Accept),
        ElicitationWaitingDismissAction::Dismiss | ElicitationWaitingDismissAction::Cancel => {
            ElicitationResult::new(ElicitationAction::Cancel)
        }
    }
}

/// Maps to: CC `findElicitationInQueue(...)`.
pub fn find_elicitation_in_queue(
    queue: &[ElicitationRequestEvent],
    server_name: &str,
    elicitation_id: &str,
) -> Option<usize> {
    queue.iter().position(|event| {
        event.server_name == server_name
            && matches!(
                &event.params,
                ElicitationRequestParams::Url {
                    elicitation_id: Some(id),
                    ..
                } if id == elicitation_id
            )
    })
}

/// Maps to: CC completion-notification `setAppState(...)` queue update.
pub fn mark_elicitation_complete(
    queue: &[ElicitationRequestEvent],
    server_name: &str,
    elicitation_id: &str,
) -> (Vec<ElicitationRequestEvent>, bool) {
    let Some(index) = find_elicitation_in_queue(queue, server_name, elicitation_id) else {
        return (queue.to_vec(), false);
    };
    let mut next = queue.to_vec();
    if let Some(event) = next.get_mut(index) {
        event.completed = true;
    }
    (next, true)
}

fn hook_response_to_result(response: &HookElicitationResponse) -> ElicitationResult {
    ElicitationResult {
        action: ElicitationAction::from(response.action.as_str()),
        content: response.content.clone(),
    }
}

/// Maps to: CC `executeElicitationHooks`'s reduction of the hook results
/// (`utils/hooks.ts:4509-4523`), which belongs to the hooks module there.
pub fn reduce_elicitation_hook_results(
    results: &[HookResult],
) -> (Option<ElicitationResult>, Option<HookBlockingError>) {
    let mut response = None;
    let mut blocking_error = None;
    for result in results {
        if let Some(error) = &result.blocking_error {
            blocking_error = Some(error.clone());
        }
        if let Some(hook_response) = &result.elicitation_response {
            response = Some(hook_response_to_result(hook_response));
        }
    }
    (response, blocking_error)
}

/// Maps to: CC `executeElicitationResultHooks`'s reduction of the hook
/// results (`utils/hooks.ts:4561-4575`), followed by `runElicitationResultHooks`
/// declining on a blocking error and applying the override
/// (`elicitationHandler.ts:282-295`).
pub fn reduce_elicitation_result_hook_results(
    original: &ElicitationResult,
    results: &[HookResult],
) -> (ElicitationResult, Option<HookBlockingError>) {
    let mut final_result = original.clone();
    let mut blocking_error = None;
    for result in results {
        if let Some(error) = &result.blocking_error {
            blocking_error = Some(error.clone());
        }
        if let Some(hook_response) = &result.elicitation_result_response {
            final_result = hook_response_to_result(hook_response);
            if final_result.content.is_none() {
                final_result.content = original.content.clone();
            }
        }
    }
    if blocking_error.is_some() {
        final_result = ElicitationResult::new(ElicitationAction::Decline);
    }
    (final_result, blocking_error)
}

/// CC SDK `ElicitationCompleteNotificationSchema.method`.
const ELICITATION_COMPLETE_NOTIFICATION_METHOD: &str = "notifications/elicitation/complete";

/// The `respond` of each queued [`ElicitationRequestEvent`], by server name
/// and request id: the event is plain state, so its callback is kept here.
pub(super) static PENDING_ELICITATION_RESPONSES: LazyLock<
    Mutex<HashMap<(String, String), oneshot::Sender<ElicitationResult>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Maps to: CC `ElicitationRequestEvent.respond(...)`, resolving the pending
/// `ElicitResult` promise its producer awaits.
pub async fn respond_to_mcp_elicitation(
    server_name: &str,
    request_id: &str,
    result: ElicitationResult,
) -> bool {
    let sender = PENDING_ELICITATION_RESPONSES
        .lock()
        .await
        .remove(&(server_name.to_string(), request_id.to_string()));
    sender.is_some_and(|sender| sender.send(result).is_ok())
}

/// CC `ElicitationRequestEvent.params`, which is the SDK request params.
pub(super) fn rmcp_elicitation_params_to_event_params(
    params: &RmcpElicitRequestParams,
) -> ElicitationRequestParams {
    match params {
        RmcpElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } => ElicitationRequestParams::Form {
            message: message.clone(),
            requested_schema: serde_json::to_value(requested_schema)
                .unwrap_or_else(|_| Value::Object(Map::new())),
        },
        RmcpElicitRequestParams::UrlElicitationParams {
            message,
            url,
            elicitation_id,
            ..
        } => ElicitationRequestParams::Url {
            message: message.clone(),
            url: url.clone(),
            elicitation_id: Some(elicitation_id.clone()),
        },
        _ => ElicitationRequestParams::Form {
            // Safe fallback for future rmcp variants; current MCP variants
            // map above and match CC `getElicitationMode(...)`.
            message: "Unsupported MCP elicitation request".to_string(),
            requested_schema: Value::Object(Map::new()),
        },
    }
}

/// Maps to: CC `registerElicitationHandler(client, serverName,
/// setAppState)` (`elicitationHandler.ts:68-212`), which
/// `onConnectionAttempt` calls in place of the default handler
/// `connectToServer` registered. The request handler lets the elicitation
/// hooks answer first, else queues the request for the dialog and waits
/// for its answer or the request's cancellation, then runs the result
/// hooks. The completion handler marks the queued request complete.
pub(super) fn register_elicitation_handler(
    client: &McpClient,
    server_name: &str,
    writer: &crate::state::app_state_store::McpWriter,
) {
    use futures::FutureExt as _;
    {
        let (server_name, writer) = (server_name.to_owned(), writer.clone());
        client.set_elicitation_handler(move |request, request_id, cancelled| {
            queue_elicitation_request(
                server_name.clone(),
                request,
                request_id,
                cancelled,
                writer.clone(),
            )
            .boxed()
        });
    }
    // `:175-207`. rmcp has no typed variant for
    // `notifications/elicitation/complete`, so it arrives as a custom
    // notification. A payload without a string `elicitationId` fails the
    // SDK schema and never reaches CC's handler; it is dropped the same
    // way.
    let (server_name, writer) = (server_name.to_owned(), writer.clone());
    client.set_notification_handler(ELICITATION_COMPLETE_NOTIFICATION_METHOD, move |params| {
        if let Some(elicitation_id) = params
            .as_ref()
            .and_then(|params| params.get("elicitationId"))
            .and_then(Value::as_str)
        {
            crate::utils::log::log_mcp_debug(
                &server_name,
                &format!("Received elicitation completion notification: {elicitation_id}"),
            );
            spawn_elicitation_notification_hooks(
                elicitation_complete_notification_message(&server_name, elicitation_id),
                "elicitation_complete",
            );
            writer.mark_elicitation_complete(&server_name, elicitation_id);
        }
        futures::future::ready(()).boxed()
    });
}

/// The request handler of [`register_elicitation_handler`]
/// (`elicitationHandler.ts:77-171`).
async fn queue_elicitation_request(
    server_name: String,
    request: RmcpElicitRequestParams,
    request_id: String,
    mut cancelled: futures::future::BoxFuture<'static, ()>,
    writer: crate::state::app_state_store::McpWriter,
) -> ElicitationResult {
    use futures::FutureExt as _;
    crate::utils::log::log_mcp_debug(
        &server_name,
        &format!(
            "Received elicitation request: {}",
            elicitation_request_json(&request)
        ),
    );
    if let Some(hook_response) = run_elicitation_hooks(&server_name, &request).await {
        crate::utils::log::log_mcp_debug(
            &server_name,
            &format!(
                "Elicitation resolved by hook: {}",
                elicitation_result_json(&hook_response)
            ),
        );
        return hook_response;
    }

    let params = rmcp_elicitation_params_to_event_params(&request);
    // `:83` and `:109-112`.
    let mode = get_elicitation_mode(&params);
    let elicitation_id = match &params {
        ElicitationRequestParams::Url { elicitation_id, .. } => elicitation_id.clone(),
        ElicitationRequestParams::Form { .. } => None,
    };

    // `:119-122`: a request already cancelled is not queued.
    if (&mut cancelled).now_or_never().is_some() {
        return ElicitationResult::new(ElicitationAction::Cancel);
    }
    let (sender, receiver) = oneshot::channel::<ElicitationResult>();
    PENDING_ELICITATION_RESPONSES
        .lock()
        .await
        .insert((server_name.clone(), request_id.clone()), sender);
    writer.push_elicitation_event(ElicitationRequestEvent::new(
        server_name.clone(),
        request_id.clone(),
        params,
    ));
    let raw_result = tokio::select! {
        _ = cancelled => {
            // `:115-117`, and the dialog that cancels on the abort
            // (`ElicitationDialog.tsx:193-209`): the request leaves the
            // queue as it is answered with a cancel.
            PENDING_ELICITATION_RESPONSES
                .lock()
                .await
                .remove(&(server_name.clone(), request_id.clone()));
            writer.remove_elicitation_event(&server_name, &request_id);
            ElicitationResult::new(ElicitationAction::Cancel)
        }
        response = receiver => {
            response.unwrap_or_else(|_| ElicitationResult::new(ElicitationAction::Cancel))
        }
    };
    crate::utils::log::log_mcp_debug(
        &server_name,
        &format!(
            "Elicitation response: {}",
            elicitation_result_json(&raw_result)
        ),
    );
    run_elicitation_result_hooks(
        &server_name,
        raw_result,
        Some(mode),
        elicitation_id.as_deref(),
    )
    .await
}

/// CC `jsonStringify(request)` of an `elicitation/create` request; the
/// default handler in client.rs logs the same.
pub(super) fn elicitation_request_json(request: &RmcpElicitRequestParams) -> Value {
    serde_json::json!({ "method": "elicitation/create", "params": request })
}

/// CC `jsonStringify(result)` of an `ElicitResult`.
fn elicitation_result_json(result: &ElicitationResult) -> Value {
    let mut json = serde_json::json!({ "action": result.action.as_str() });
    if let Some(content) = &result.content {
        json["content"] = content.clone();
    }
    json
}

/// The hook inputs CC reads off the request params inline: message, schema,
/// mode, URL and elicitation id (`elicitationHandler.ts:220-234`).
fn rmcp_elicitation_hook_details(
    params: &RmcpElicitRequestParams,
) -> (
    String,
    Option<Value>,
    &'static str,
    Option<String>,
    Option<String>,
) {
    match params {
        RmcpElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } => (
            message.clone(),
            Some(
                serde_json::to_value(requested_schema)
                    .unwrap_or_else(|_| Value::Object(Map::new())),
            ),
            "form",
            None,
            None,
        ),
        RmcpElicitRequestParams::UrlElicitationParams {
            message,
            url,
            elicitation_id,
            ..
        } => (
            message.clone(),
            None,
            "url",
            Some(url.clone()),
            Some(elicitation_id.clone()),
        ),
        _ => (
            "Unsupported MCP elicitation request".to_string(),
            None,
            "form",
            None,
            None,
        ),
    }
}

/// Stands in for the hook lookup `executeHooksOutsideREPL` does
/// (`utils/hooks.ts:3003-3046`), with a narrower source: settings only, no
/// plugin or session hooks, and a `permission_mode` CC does not send.
fn load_mcp_elicitation_hooks_config_and_env() -> Option<(
    crate::services::hooks::RegisteredHooks,
    crate::services::hooks::HookContext,
    Vec<(String, String)>,
)> {
    let loaded_hooks =
        crate::utils::hooks::hooks_config_snapshot::load_hooks_config_from_settings_sources();
    let config = loaded_hooks.config;
    if config.is_empty() {
        return None;
    }
    let cwd = std::env::current_dir()
        .unwrap_or_default()
        .display()
        .to_string();
    let hook_context = crate::services::hooks::HookContext {
        cwd,
        permission_mode: loaded_hooks.merged_settings.default_permission_mode,
        ..Default::default()
    };
    Some((config, hook_context, Vec::new()))
}

/// Maps to: CC `runElicitationHooks` (`elicitationHandler.ts:214-257`).
pub(super) async fn run_elicitation_hooks(
    server_name: &str,
    request: &RmcpElicitRequestParams,
) -> Option<ElicitationResult> {
    let (config, hook_context, base_env) = load_mcp_elicitation_hooks_config_and_env()?;
    let (message, requested_schema, mode, url, elicitation_id) =
        rmcp_elicitation_hook_details(request);
    let results = crate::services::hooks::elicitation::execute_elicitation_hooks(
        &config,
        &hook_context,
        server_name,
        &message,
        requested_schema,
        Some(mode),
        url.as_deref(),
        elicitation_id.as_deref(),
        base_env,
    )
    .await;
    let (response, blocking_error) = reduce_elicitation_hook_results(&results);
    if blocking_error.is_some() {
        Some(ElicitationResult::new(ElicitationAction::Decline))
    } else {
        response
    }
}

fn elicitation_complete_notification_message(server_name: &str, elicitation_id: &str) -> String {
    // Maps to: CC `registerElicitationHandler(...)` completion
    // notification hook message.
    format!("MCP server \"{server_name}\" confirmed elicitation {elicitation_id} complete")
}

fn elicitation_response_notification_message(server_name: &str, action: &str) -> String {
    // Maps to: CC `runElicitationResultHooks(...)` response
    // notification hook message.
    format!("Elicitation response for server \"{server_name}\": {action}")
}

/// CC `void executeNotificationHooks({message, notificationType})`: detached
/// for the life of the process. A -32042 retry runs this on a query's private
/// runtime, which the turn's end would tear down (PORTING.md A2/A3).
fn spawn_elicitation_notification_hooks(message: String, notification_type: &'static str) {
    let Some((config, _, base_env)) = load_mcp_elicitation_hooks_config_and_env() else {
        return;
    };
    let runtime = crate::utils::process_runtime::runtime_handle_for_detached_work()
        .expect("elicitation notification hooks require the process runtime");
    runtime.spawn(async move {
        crate::services::hooks::lifecycle::execute_notification_hooks(
            &config,
            &message,
            notification_type,
            None,
            base_env,
        )
        .await;
    });
}

/// Maps to: CC `runElicitationResultHooks(serverName, result, signal, mode?,
/// elicitationId?)` (`elicitationHandler.ts:264-313`), minus the signal.
pub(super) async fn run_elicitation_result_hooks(
    server_name: &str,
    result: ElicitationResult,
    mode: Option<ElicitationMode>,
    elicitation_id: Option<&str>,
) -> ElicitationResult {
    let Some((config, hook_context, base_env)) = load_mcp_elicitation_hooks_config_and_env() else {
        return result;
    };
    let results = crate::services::hooks::elicitation::execute_elicitation_result_hooks(
        &config,
        &hook_context,
        server_name,
        result.action.as_str(),
        result.content.clone(),
        mode.map(ElicitationMode::as_str),
        elicitation_id,
        base_env,
    )
    .await;
    let final_result = reduce_elicitation_result_hook_results(&result, &results).0;
    spawn_elicitation_notification_hooks(
        elicitation_response_notification_message(server_name, final_result.action.as_str()),
        "elicitation_response",
    );
    final_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::hooks::{HookBlockingError, HookElicitationResponse, HookResult};

    #[test]
    fn find_and_mark_elicitation_completion_matches_official_queue_lookup() {
        let form = ElicitationRequestEvent::new(
            "docs",
            "1",
            ElicitationRequestParams::Form {
                message: "form".to_string(),
                requested_schema: serde_json::json!({"type":"object"}),
            },
        );
        let url = ElicitationRequestEvent::new(
            "docs",
            "2",
            ElicitationRequestParams::Url {
                message: "open".to_string(),
                url: "https://example.com/auth".to_string(),
                elicitation_id: Some("elicit-1".to_string()),
            },
        );
        let queue = vec![form, url];

        assert_eq!(
            find_elicitation_in_queue(&queue, "docs", "elicit-1"),
            Some(1)
        );
        assert_eq!(find_elicitation_in_queue(&queue, "other", "elicit-1"), None);

        let (updated, found) = mark_elicitation_complete(&queue, "docs", "elicit-1");
        assert!(found);
        assert!(!queue[1].completed);
        assert!(updated[1].completed);
    }

    #[test]
    fn url_event_with_elicitation_id_gets_official_waiting_state_default() {
        let event = ElicitationRequestEvent::new(
            "docs",
            "2",
            ElicitationRequestParams::Url {
                message: "open".to_string(),
                url: "https://example.com/auth".to_string(),
                elicitation_id: Some("elicit-1".to_string()),
            },
        );
        assert_eq!(get_elicitation_mode(&event.params), ElicitationMode::Url);
        assert_eq!(
            event.waiting_state,
            Some(ElicitationWaitingState {
                action_label: "Skip confirmation".to_string(),
                show_cancel: false,
            })
        );
    }

    #[test]
    fn url_retry_waiting_dismiss_result_matches_official_retry_flow() {
        assert_eq!(
            url_retry_waiting_dismiss_result(ElicitationWaitingDismissAction::Retry),
            ElicitationResult::new(ElicitationAction::Accept)
        );
        assert_eq!(
            url_retry_waiting_dismiss_result(ElicitationWaitingDismissAction::Cancel),
            ElicitationResult::new(ElicitationAction::Cancel)
        );
    }

    #[test]
    fn reduce_elicitation_hook_results_keeps_last_response_and_blocking_error() {
        let mut first = HookResult::default();
        first.elicitation_response = Some(HookElicitationResponse {
            action: "accept".to_string(),
            content: Some(serde_json::json!({"a": 1})),
        });
        let mut second = HookResult::default();
        second.elicitation_response = Some(HookElicitationResponse {
            action: "decline".to_string(),
            content: None,
        });
        second.blocking_error = Some(HookBlockingError {
            blocking_error: "blocked".to_string(),
            command: "hook".to_string(),
        });

        let (response, blocking) = reduce_elicitation_hook_results(&[first, second]);
        assert_eq!(
            response,
            Some(ElicitationResult::new(ElicitationAction::Decline))
        );
        assert_eq!(blocking.unwrap().blocking_error, "blocked");
    }

    #[test]
    fn reduce_elicitation_result_hook_results_preserves_original_content_when_override_omits_it() {
        let original = ElicitationResult {
            action: ElicitationAction::Accept,
            content: Some(serde_json::json!({"email":"user@example.com"})),
        };
        let mut hook = HookResult::default();
        hook.elicitation_result_response = Some(HookElicitationResponse {
            action: "accept".to_string(),
            content: None,
        });

        let (result, blocking) = reduce_elicitation_result_hook_results(&original, &[hook]);
        assert!(blocking.is_none());
        assert_eq!(result, original);
    }

    #[test]
    fn reduce_elicitation_result_hook_results_declines_when_blocking() {
        let original = ElicitationResult {
            action: ElicitationAction::Accept,
            content: Some(serde_json::json!({"email":"user@example.com"})),
        };
        let mut hook = HookResult::default();
        hook.blocking_error = Some(HookBlockingError {
            blocking_error: "no".to_string(),
            command: "hook".to_string(),
        });

        let (result, blocking) = reduce_elicitation_result_hook_results(&original, &[hook]);
        assert!(blocking.is_some());
        assert_eq!(result, ElicitationResult::new(ElicitationAction::Decline));
    }

    #[test]
    fn elicitation_notification_messages_match_official_hooks() {
        assert_eq!(
            elicitation_complete_notification_message("docs", "elicit-1"),
            "MCP server \"docs\" confirmed elicitation elicit-1 complete"
        );
        assert_eq!(
            elicitation_response_notification_message("docs", "decline"),
            "Elicitation response for server \"docs\": decline"
        );
    }
}
