//! Diagnostic tracking formatting helpers.
//!
//! Maps to: CC `services/diagnosticTracking.ts` `DiagnosticTrackingService`.
//! Owns the IDE/MCP baseline, right-file state, query reset, new-diagnostic
//! filtering, and shared UI/model summary formatting.

use crate::services::lsp::types::{Diagnostic, DiagnosticFile};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const MAX_DIAGNOSTICS_SUMMARY_CHARS: usize = 4000;

#[derive(Default)]
struct DiagnosticTrackerState {
    baseline: HashMap<String, Vec<Diagnostic>>,
    initialized: bool,
    /// CC `mcpClient`: the IDE client the first `handleQueryStart` found,
    /// kept for the session (CC's own TODO notes it can change).
    mcp_client: Option<crate::services::mcp::types::ConnectedMcpServer>,
    right_file_diagnostics_state: HashMap<String, Vec<Diagnostic>>,
    last_processed_timestamps: HashMap<String, i64>,
}

impl DiagnosticTrackerState {
    /// Maps to CC `DiagnosticTrackingService.reset()` (`:72-76`): the tracked
    /// files go; the service stays initialized with its client.
    fn reset(&mut self) {
        self.baseline.clear();
        self.right_file_diagnostics_state.clear();
        self.last_processed_timestamps.clear();
    }
}

static TRACKER: LazyLock<Mutex<DiagnosticTrackerState>> =
    LazyLock::new(|| Mutex::new(DiagnosticTrackerState::default()));
const TRUNCATION_MARKER: &str = "…[truncated]";

fn normalize_file_uri(file_uri: &str) -> String {
    let without_protocol = ["file://", "_claude_fs_right:", "_claude_fs_left:"]
        .into_iter()
        .find_map(|prefix| file_uri.strip_prefix(prefix))
        .unwrap_or(file_uri);
    let mut normalized = std::path::Path::new(without_protocol)
        .components()
        .fold(std::path::PathBuf::new(), |mut path, component| {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    path.pop();
                }
                other => path.push(other.as_os_str()),
            }
            path
        })
        .display()
        .to_string();
    if cfg!(windows) {
        normalized = normalized.replace('/', "\\").to_lowercase();
    }
    normalized
}

/// Maps to CC `DiagnosticTrackingService.parseDiagnosticResult(result)`
/// (`:285-294`): the first text block's JSON. Text that is not a diagnostic
/// file list fails, as CC's `jsonParse` throws into the caller's `catch`.
fn parse_diagnostic_result(result: &serde_json::Value) -> serde_json::Result<Vec<DiagnosticFile>> {
    let text = result.as_array().and_then(|blocks| {
        blocks
            .iter()
            .find(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
            .and_then(|block| block.get("text").and_then(serde_json::Value::as_str))
    });
    match text {
        Some(text) => serde_json::from_str(text),
        None => Ok(Vec::new()),
    }
}

fn lock_tracker() -> std::sync::MutexGuard<'static, DiagnosticTrackerState> {
    TRACKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// CC's guard at the top of each RPC method: the kept client, while the
/// service is initialized and the client connected.
fn initialized_client() -> Option<crate::services::mcp::types::ConnectedMcpServer> {
    let tracker = lock_tracker();
    tracker
        .initialized
        .then(|| tracker.mcp_client.clone())
        .flatten()
}

/// Maps to CC `DiagnosticTrackingService.initialize(mcpClient)` (`:51-59`):
/// the first client is kept; later calls change nothing.
fn initialize(
    tracker: &mut DiagnosticTrackerState,
    mcp_client: crate::services::mcp::types::ConnectedMcpServer,
) {
    if tracker.initialized {
        return;
    }
    tracker.mcp_client = Some(mcp_client);
    tracker.initialized = true;
}

/// Maps to CC `DiagnosticTrackingService.handleQueryStart(clients)`
/// (`:330-343`): the first query that finds a connected IDE client keeps it;
/// every later one resets the tracked files.
pub fn handle_query_start(clients: &[crate::services::mcp::types::McpServerConnection]) {
    let mut tracker = lock_tracker();
    if !tracker.initialized {
        if let Some(client) = crate::utils::ide::get_connected_ide_client(clients) {
            initialize(&mut tracker, client.clone());
        }
    } else {
        tracker.reset();
    }
}

/// Maps to CC `DiagnosticTrackingService.shutdown()` (`:61-66`).
pub fn shutdown() {
    let mut tracker = lock_tracker();
    tracker.initialized = false;
    tracker.reset();
}

/// Maps to CC `DiagnosticTrackingService.beforeFileEdited(filePath)`
/// (`:135-182`): the file's diagnostics from the kept IDE client become its
/// baseline; any failure leaves the baseline as it was.
pub async fn before_file_edited(file_path: &str) {
    let Some(client) = initialized_client() else {
        return;
    };
    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut arguments = serde_json::Map::new();
    arguments.insert(
        "uri".to_string(),
        serde_json::Value::String(format!("file://{file_path}")),
    );
    let Ok(result) =
        crate::services::mcp::client::call_ide_rpc("getDiagnostics", arguments, &client).await
    else {
        return;
    };
    let Ok(files) = parse_diagnostic_result(&result) else {
        return;
    };
    let expected = normalize_file_uri(file_path);
    let diagnostics = match files.first() {
        Some(file) if normalize_file_uri(&file.uri) == expected => file.diagnostics.clone(),
        Some(file) => {
            crate::utils::debug::log_for_debugging(&format!(
                "Diagnostics file path mismatch: expected {file_path}, got {})",
                file.uri
            ));
            return;
        }
        None => Vec::new(),
    };
    let mut tracker = lock_tracker();
    tracker.baseline.insert(expected.clone(), diagnostics);
    tracker
        .last_processed_timestamps
        .insert(expected, timestamp);
}

fn diagnostic_arrays_equal(left: &[Diagnostic], right: &[Diagnostic]) -> bool {
    left.len() == right.len()
        && left.iter().all(|diagnostic| right.contains(diagnostic))
        && right.iter().all(|diagnostic| left.contains(diagnostic))
}

fn collect_new_diagnostics(
    tracker: &mut DiagnosticTrackerState,
    files: &[DiagnosticFile],
) -> Vec<DiagnosticFile> {
    let right_files = files
        .iter()
        .filter(|file| file.uri.starts_with("_claude_fs_right:"))
        .filter(|file| {
            tracker
                .baseline
                .contains_key(&normalize_file_uri(&file.uri))
        })
        .map(|file| (normalize_file_uri(&file.uri), file))
        .collect::<HashMap<_, _>>();
    let tracked_file_uris = files
        .iter()
        .filter(|file| file.uri.starts_with("file://"))
        .filter(|file| {
            tracker
                .baseline
                .contains_key(&normalize_file_uri(&file.uri))
        })
        .collect::<Vec<_>>();
    let mut new_files = Vec::new();
    for file in tracked_file_uris {
        let normalized = normalize_file_uri(&file.uri);
        let baseline = tracker
            .baseline
            .get(&normalized)
            .cloned()
            .unwrap_or_default();
        let selected = if let Some(right) = right_files.get(&normalized) {
            let changed = tracker
                .right_file_diagnostics_state
                .get(&normalized)
                .is_none_or(|previous| !diagnostic_arrays_equal(previous, &right.diagnostics));
            tracker
                .right_file_diagnostics_state
                .insert(normalized.clone(), right.diagnostics.clone());
            if changed { *right } else { file }
        } else {
            file
        };
        let diagnostics = selected
            .diagnostics
            .iter()
            .filter(|diagnostic| !baseline.contains(diagnostic))
            .cloned()
            .collect::<Vec<_>>();
        if !diagnostics.is_empty() {
            new_files.push(DiagnosticFile {
                uri: file.uri.clone(),
                diagnostics,
            });
        }
        tracker
            .baseline
            .insert(normalized, selected.diagnostics.clone());
    }
    new_files
}

/// Maps to CC `DiagnosticTrackingService.getNewDiagnostics()` (`:188-283`):
/// every diagnostic from the kept IDE client, diffed against the baselines;
/// any failure is no new diagnostics.
pub async fn get_new_diagnostics() -> Vec<DiagnosticFile> {
    let Some(client) = initialized_client() else {
        return Vec::new();
    };
    let Ok(Ok(files)) = crate::services::mcp::client::call_ide_rpc(
        "getDiagnostics",
        serde_json::Map::new(),
        &client,
    )
    .await
    .map(|result| parse_diagnostic_result(&result)) else {
        return Vec::new();
    };
    collect_new_diagnostics(&mut lock_tracker(), &files)
}

/// Maps to: CC `DiagnosticTrackingService.getSeveritySymbol(...)`.
pub fn get_severity_symbol(severity: &str) -> &'static str {
    let figures = crate::constants::figures::figures();
    match severity {
        "Error" => figures.cross,
        "Warning" => figures.warning,
        "Info" => figures.info,
        "Hint" => figures.star,
        _ => figures.bullet,
    }
}

/// Maps to: CC `DiagnosticTrackingService.formatDiagnosticsSummary(...)`.
pub fn format_diagnostics_summary(files: &[DiagnosticFile]) -> String {
    let result = files
        .iter()
        .map(|file| {
            let filename = file.uri.rsplit('/').next().unwrap_or(&file.uri);
            let diagnostics = file
                .diagnostics
                .iter()
                .map(format_single_diagnostic)
                .collect::<Vec<_>>()
                .join("\n");
            format!("{filename}:\n{diagnostics}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if result.chars().count() > MAX_DIAGNOSTICS_SUMMARY_CHARS {
        let take = MAX_DIAGNOSTICS_SUMMARY_CHARS.saturating_sub(TRUNCATION_MARKER.chars().count());
        format!(
            "{}{}",
            result.chars().take(take).collect::<String>(),
            TRUNCATION_MARKER
        )
    } else {
        result
    }
}

/// Maps to: CC `components/DiagnosticsDisplay.tsx` normal-mode summary text.
pub fn format_diagnostics_display_summary(files: &[DiagnosticFile]) -> String {
    let total_issues = files
        .iter()
        .map(|file| file.diagnostics.len())
        .sum::<usize>();
    let file_count = files.len();
    let issue_word = if total_issues == 1 { "issue" } else { "issues" };
    let file_word = if file_count == 1 { "file" } else { "files" };
    format!("Found {total_issues} new diagnostic {issue_word} in {file_count} {file_word}")
}

/// Maps to each diagnostic row in CC `components/DiagnosticsDisplay.tsx`.
pub fn format_single_diagnostic(diagnostic: &Diagnostic) -> String {
    let code = diagnostic
        .code
        .as_ref()
        .map(|code| format!(" [{code}]"))
        .unwrap_or_default();
    let source = diagnostic
        .source
        .as_ref()
        .map(|source| format!(" ({source})"))
        .unwrap_or_default();
    format!(
        "  {} [Line {}:{}] {}{}{}",
        get_severity_symbol(&diagnostic.severity),
        diagnostic.range.start.line + 1,
        diagnostic.range.start.character + 1,
        diagnostic.message,
        code,
        source
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::lsp::types::{DiagnosticPosition, DiagnosticRange};

    fn diag(message: &str, severity: &str, line: u32) -> Diagnostic {
        Diagnostic {
            message: message.to_string(),
            severity: severity.to_string(),
            range: DiagnosticRange {
                start: DiagnosticPosition { line, character: 2 },
                end: DiagnosticPosition { line, character: 8 },
            },
            source: Some("rust-analyzer".to_string()),
            code: Some("E0425".to_string()),
        }
    }

    /// CC `parseDiagnosticResult` (`:285-294`): the first text block of the
    /// `callIdeRpc` content is parsed; with no text block there is nothing,
    /// and text that is not JSON throws.
    #[test]
    fn diagnostic_result_parser_reads_first_text_block() {
        let value = serde_json::json!([
            { "type": "image", "data": "", "mimeType": "image/png" },
            {
                "type": "text",
                "text": serde_json::to_string(&vec![DiagnosticFile {
                    uri: "file:///tmp/a.rs".to_string(),
                    diagnostics: vec![diag("broken", "Error", 0)],
                }]).unwrap()
            },
            { "type": "text", "text": "ignored" },
        ]);
        let parsed = parse_diagnostic_result(&value).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].diagnostics[0].message, "broken");

        assert!(
            parse_diagnostic_result(&serde_json::json!([]))
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_diagnostic_result(&serde_json::json!([{ "type": "text", "text": "ok:s1" }]))
                .is_err()
        );
    }

    #[test]
    fn new_diagnostics_are_diffed_against_baseline_and_then_advanced() {
        let old = diag("old", "Warning", 0);
        let new = diag("new", "Error", 1);
        let mut tracker = DiagnosticTrackerState::default();
        tracker
            .baseline
            .insert("/tmp/a.rs".to_string(), vec![old.clone()]);
        let files = vec![DiagnosticFile {
            uri: "file:///tmp/a.rs".to_string(),
            diagnostics: vec![old, new.clone()],
        }];
        let first = collect_new_diagnostics(&mut tracker, &files);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].diagnostics, vec![new]);
        assert!(collect_new_diagnostics(&mut tracker, &files).is_empty());
    }

    #[test]
    fn right_file_diagnostics_follow_official_change_selection_state() {
        let baseline = diag("baseline", "Warning", 0);
        let disk_new = diag("disk", "Error", 1);
        let right_new = diag("right", "Hint", 2);
        let right_changed = diag("right changed", "Info", 3);
        let mut tracker = DiagnosticTrackerState::default();
        tracker
            .baseline
            .insert("/tmp/a.rs".to_string(), vec![baseline.clone()]);

        let files = |right: Diagnostic| {
            vec![
                DiagnosticFile {
                    uri: "file:///tmp/a.rs".to_string(),
                    diagnostics: vec![baseline.clone(), disk_new.clone()],
                },
                DiagnosticFile {
                    uri: "_claude_fs_right:/tmp/a.rs".to_string(),
                    diagnostics: vec![baseline.clone(), right],
                },
            ]
        };

        let first = collect_new_diagnostics(&mut tracker, &files(right_new.clone()));
        assert_eq!(first[0].uri, "file:///tmp/a.rs");
        assert_eq!(first[0].diagnostics, vec![right_new.clone()]);

        // An unchanged right-side snapshot is ignored in favor of file://.
        let second = collect_new_diagnostics(&mut tracker, &files(right_new));
        assert_eq!(second[0].diagnostics, vec![disk_new.clone()]);

        // A newly changed right-side snapshot takes priority again.
        let third = collect_new_diagnostics(&mut tracker, &files(right_changed.clone()));
        assert_eq!(third[0].diagnostics, vec![right_changed]);
    }

    #[test]
    fn right_only_diagnostics_are_not_emitted_without_file_uri_peer() {
        let mut tracker = DiagnosticTrackerState::default();
        tracker
            .baseline
            .insert("/tmp/a.rs".to_string(), vec![diag("old", "Warning", 0)]);
        let files = vec![DiagnosticFile {
            uri: "_claude_fs_right:/tmp/a.rs".to_string(),
            diagnostics: vec![diag("new", "Error", 1)],
        }];
        assert!(collect_new_diagnostics(&mut tracker, &files).is_empty());
    }

    #[test]
    fn diagnostic_summary_matches_official_shape() {
        let files = vec![DiagnosticFile {
            uri: "/tmp/project/src/main.rs".to_string(),
            diagnostics: vec![diag("cannot find value", "Error", 4)],
        }];

        let summary = format_diagnostics_summary(&files);
        assert!(summary.starts_with("main.rs:\n"));
        assert!(summary.contains("[Line 5:3] cannot find value [E0425] (rust-analyzer)"));
    }

    #[test]
    fn diagnostic_display_summary_pluralizes_like_official() {
        let one = vec![DiagnosticFile {
            uri: "/a.rs".to_string(),
            diagnostics: vec![diag("broken", "Warning", 0)],
        }];
        assert_eq!(
            format_diagnostics_display_summary(&one),
            "Found 1 new diagnostic issue in 1 file"
        );

        let many = vec![
            DiagnosticFile {
                uri: "/a.rs".to_string(),
                diagnostics: vec![diag("a", "Error", 0), diag("b", "Hint", 1)],
            },
            DiagnosticFile {
                uri: "/b.rs".to_string(),
                diagnostics: vec![diag("c", "Info", 2)],
            },
        ];
        assert_eq!(
            format_diagnostics_display_summary(&many),
            "Found 3 new diagnostic issues in 2 files"
        );
    }

    #[test]
    fn diagnostic_summary_truncates_to_official_budget() {
        let files = vec![DiagnosticFile {
            uri: "/tmp/huge.rs".to_string(),
            diagnostics: vec![diag(&"x".repeat(5000), "Error", 0)],
        }];

        let summary = format_diagnostics_summary(&files);
        assert!(summary.ends_with(TRUNCATION_MARKER));
        assert_eq!(summary.chars().count(), MAX_DIAGNOSTICS_SUMMARY_CHARS);
    }
    /// CC `handleQueryStart` (`:330-343`) keeps the IDE client the first
    /// query finds, and `beforeFileEdited` / `getNewDiagnostics` call
    /// `getDiagnostics` on it through `callIdeRpc`. Before that, and after
    /// `shutdown`, nothing is asked; a later query start resets the
    /// baselines.
    #[test]
    fn diagnostics_go_through_the_ide_client_kept_at_query_start() {
        use crate::services::mcp::types::McpServerConnection;
        let _guard = crate::utils::test_env::TEST_ENV_LOCK.lock().unwrap();
        crate::utils::process_runtime::initialize_test_process_runtime();

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use crate::services::mcp::client::session_http_fixture::{
                    SESSION_NOT_FOUND, SessionHttpFixture,
                };
                let fixture = SessionHttpFixture::spawn(|_, _| false, SESSION_NOT_FOUND).await;
                let config = fixture.config();
                let ide = crate::services::mcp::client::connect_to_server("ide", &config).await;
                let reply = |diagnostics: Vec<Diagnostic>| {
                    let files = vec![DiagnosticFile {
                        uri: "file:///tmp/edited.rs".into(),
                        diagnostics,
                    }];
                    fixture.reply(
                        "getDiagnostics",
                        serde_json::json!({ "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&files).unwrap(),
                        }] }),
                    );
                };
                let calls = || {
                    fixture
                        .tool_calls()
                        .into_iter()
                        .map(|call| (call["name"].clone(), call["arguments"].clone()))
                        .collect::<Vec<_>>()
                };
                let old = diag("old", "Warning", 0);
                let new = diag("new", "Error", 1);
                reply(vec![old.clone()]);
                shutdown();

                // Not initialized, then a query start with no connected IDE:
                // nothing is asked.
                before_file_edited("/tmp/edited.rs").await;
                assert!(get_new_diagnostics().await.is_empty());
                handle_query_start(&[McpServerConnection::failed("ide", config.clone(), None)]);
                before_file_edited("/tmp/edited.rs").await;
                assert!(calls().is_empty());

                handle_query_start(std::slice::from_ref(&ide));
                before_file_edited("/tmp/edited.rs").await;
                reply(vec![old.clone(), new.clone()]);
                assert_eq!(
                    get_new_diagnostics().await,
                    [DiagnosticFile {
                        uri: "file:///tmp/edited.rs".into(),
                        diagnostics: vec![new.clone()],
                    }]
                );
                assert_eq!(
                    calls(),
                    [
                        (
                            serde_json::json!("getDiagnostics"),
                            serde_json::json!({ "uri": "file:///tmp/edited.rs" })
                        ),
                        (serde_json::json!("getDiagnostics"), serde_json::json!({})),
                    ]
                );

                // A later query start resets the baselines, and keeps the
                // first IDE client although another one is connected now
                // (CC `:56` TODO): the call still reaches the fixture.
                let other_ide = McpServerConnection::connected_for_test(
                    "ide",
                    crate::services::mcp::types::ScopedMcpServerConfig::stdio_for_test("other"),
                );
                handle_query_start(&[other_ide]);
                assert!(get_new_diagnostics().await.is_empty());
                assert_eq!(calls().len(), 3);

                shutdown();
                assert!(get_new_diagnostics().await.is_empty());
                assert_eq!(calls().len(), 3);

                crate::services::mcp::client::clear_server_cache("ide", None).await;
            });
    }

    #[tokio::test]
    async fn hook_query_preserves_parent_diagnostics_matches_official_generator_entry() {
        // execAgentHook.ts:177-189 calls query directly; only REPL.tsx:3565-3570
        // invokes handleQueryStart. Retain baseline so the next diagnostic is emitted.
        #[derive(Clone)]
        struct FailedModel;
        impl crate::query::deps::QueryDeps for FailedModel {}
        let old = diag("old", "Warning", 0);
        TRACKER
            .lock()
            .unwrap()
            .baseline
            .insert("/tmp/hook-parent.rs".into(), vec![old.clone()]);
        let handle = crate::query::spawn_query_generator(
            crate::query::QueryParams {
                turn_id: "hook-diagnostics".into(),
                input: "verify".into(),
                messages: Vec::new(),
                model_messages: vec![crate::types::message::Message::User(
                    crate::utils::messages::create_user_message("verify".into()),
                )],
                system_prompt: Vec::new(),
                user_context: Default::default(),
                system_context: Default::default(),
                query_source: crate::constants::query_source::QuerySource::HookAgent,
                token_budget: None,
                task_budget: None,
                max_turns: None,
                tool_use_context: crate::tool::ToolUseContext {
                    agent_id: Some("hook-agent-diagnostics".into()),
                    ..Default::default()
                },
            },
            FailedModel,
        );
        let control = handle.resume.as_ref().unwrap().clone();
        let _guard = crate::query::QueryGeneratorGuard(control.clone());
        while let Ok(event) = handle.events.recv().await {
            if matches!(event, crate::query::QueryEvent::Terminal(_)) {
                break;
            }
            control.advance();
        }
        let new = diag("new", "Error", 1);
        let result = collect_new_diagnostics(
            &mut TRACKER.lock().unwrap(),
            &[DiagnosticFile {
                uri: "file:///tmp/hook-parent.rs".into(),
                diagnostics: vec![old, new.clone()],
            }],
        );
        assert_eq!(
            result.len(),
            1,
            "hook query must not clear the parent's baseline"
        );
        assert_eq!(result[0].diagnostics, vec![new]);
    }
}
