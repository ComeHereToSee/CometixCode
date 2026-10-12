//! Maps to: CC `utils/suggestions/slackChannelSuggestions.ts`.
//!
//! The MCP round-trip, result-envelope parsing and prefix cache stay in this
//! producer owner. `use_typeahead` only decides when a `#channel` token should
//! request it and how the returned rows are selected.

use crate::components::prompt_input::prompt_input_footer_suggestions::SuggestionItem;
use crate::services::mcp::types::{ConnectedMcpServer, McpServerConnection};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

const SEARCH_TOOL: &str = "slack_search_channels";
const MAX_RESULTS: usize = 10;

static CACHE: LazyLock<Mutex<HashMap<String, Vec<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static KNOWN_CHANNELS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static KNOWN_CHANNELS_VERSION: AtomicU64 = AtomicU64::new(0);

/// Maps to: CC `findSlackClient(clients)` (`slackChannelSuggestions.ts:23-27`):
/// the first connected record whose name contains `slack`, case-sensitively.
fn find_slack_client(clients: &[McpServerConnection]) -> Option<&ConnectedMcpServer> {
    clients
        .iter()
        .filter_map(McpServerConnection::as_connected)
        .find(|client| client.name.contains("slack"))
}

/// Maps to: CC `hasSlackMcpServer`.
pub fn has_slack_mcp_server(clients: &[McpServerConnection]) -> bool {
    find_slack_client(clients).is_some()
}

/// Maps to CC `getKnownChannelsVersion`.
pub fn get_known_channels_version() -> u64 {
    KNOWN_CHANNELS_VERSION.load(Ordering::Acquire)
}

/// Maps to CC `findSlackChannelPositions`. Ranges use UTF-8 byte offsets for
/// direct slicing by the Rust text renderer; only channels confirmed by an
/// MCP result are highlighted.
pub fn find_slack_channel_positions(text: &str) -> Vec<Range<usize>> {
    let Some(known) = KNOWN_CHANNELS.lock().ok() else {
        return Vec::new();
    };
    let mut positions = Vec::new();
    let mut line_offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let bytes = body.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let boundary = index == 0
                || body[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            if !boundary || bytes[index] != b'#' {
                index += 1;
                continue;
            }
            let start = index + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_lowercase()
                    || bytes[end].is_ascii_digit()
                    || bytes[end] == b'_'
                    || bytes[end] == b'-')
            {
                end += 1;
            }
            let channel = &body[start..end];
            let trailing_boundary =
                end == bytes.len() || body[end..].chars().next().is_some_and(char::is_whitespace);
            let starts_correctly = channel
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit());
            if starts_correctly
                && channel.len() <= 80
                && trailing_boundary
                && known.contains(channel)
            {
                positions.push(line_offset + index..line_offset + end);
            }
            index = end;
        }
        line_offset += line.len();
    }
    positions
}

fn unwrap_results(value: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.starts_with('{') {
        return value.to_string();
    }
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("results")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| value.to_string())
}

fn parse_channels(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .filter_map(|line| {
            let (head, value) = line.split_once(':')?;
            // The source regex is anchored and case-sensitive: `Name:` only.
            if head != "Name" {
                return None;
            }
            let value = value.trim_start();
            let channel = value.strip_prefix('#').unwrap_or(value).trim_end();
            let valid = !channel.is_empty()
                && channel.len() <= 80
                && channel
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
                && channel.chars().all(|ch| {
                    ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-'
                });
            if !valid || !seen.insert(channel.to_string()) {
                return None;
            }
            Some(channel.to_string())
        })
        .collect()
}

fn mcp_query_for(search_token: &str) -> String {
    let last_separator = search_token
        .rfind('-')
        .into_iter()
        .chain(search_token.rfind('_'))
        .max()
        .unwrap_or(0);
    if last_separator > 0 {
        search_token[..last_separator].to_string()
    } else {
        search_token.to_string()
    }
}

fn find_reusable_cache_entry(mcp_query: &str, search_token: &str) -> Option<Vec<String>> {
    let cache = CACHE.lock().ok()?;
    cache
        .iter()
        .filter(|(key, channels)| {
            mcp_query.starts_with(key.as_str())
                && channels
                    .iter()
                    .any(|channel| channel.starts_with(search_token))
        })
        .max_by_key(|(key, _)| key.len())
        .map(|(_, channels)| channels.clone())
}

/// Maps to: CC `fetchChannels(clients, query)`
/// (`slackChannelSuggestions.ts:29-65`): `slack_search_channels` straight
/// on the Slack client, with a five-second timeout; any failure is no
/// channels.
async fn fetch_channels(clients: &[McpServerConnection], query: &str) -> Vec<String> {
    let Some(slack_client) = find_slack_client(clients) else {
        return Vec::new();
    };
    let args = Map::from_iter([
        ("query".to_string(), json!(query)),
        ("limit".to_string(), json!(20)),
        (
            "channel_types".to_string(),
            json!("public_channel,private_channel"),
        ),
    ]);
    let params = rmcp::model::CallToolRequestParams::new(SEARCH_TOOL).with_arguments(args);
    let result = match slack_client
        .client
        .call_tool(params, std::time::Duration::from_millis(5000))
        .await
    {
        Ok(result) => result,
        Err(error) => {
            crate::utils::debug::log_for_debugging(&format!(
                "Failed to fetch Slack channels: {error}"
            ));
            return Vec::new();
        }
    };
    let raw_text = result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    parse_channels(&unwrap_results(&raw_text))
}

/// Maps to: CC `getSlackChannelSuggestions`.
pub async fn get_slack_channel_suggestions(
    clients: &[McpServerConnection],
    search_token: &str,
) -> Vec<SuggestionItem> {
    let token = search_token.to_ascii_lowercase();
    if token.is_empty() {
        return Vec::new();
    }
    let mcp_query = mcp_query_for(&token);
    let cached = CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&mcp_query).cloned())
        .or_else(|| find_reusable_cache_entry(&mcp_query, &token));
    let channels = if let Some(cached) = cached {
        cached
    } else {
        let channels = fetch_channels(clients, &mcp_query).await;
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(mcp_query, channels.clone());
            if cache.len() > 50 {
                let first = cache.keys().next().cloned();
                if let Some(first) = first {
                    cache.remove(&first);
                }
            }
        }
        if let Ok(mut known) = KNOWN_CHANNELS.lock() {
            let previous_len = known.len();
            known.extend(channels.iter().cloned());
            if known.len() != previous_len {
                KNOWN_CHANNELS_VERSION.fetch_add(1, Ordering::AcqRel);
            }
        }
        channels
    };
    let mut channels = channels
        .into_iter()
        .filter(|channel| channel.starts_with(&token))
        .collect::<Vec<_>>();
    channels.sort();
    channels
        .into_iter()
        .take(MAX_RESULTS)
        .map(|channel| SuggestionItem {
            id: format!("slack-channel-{channel}"),
            display_text: format!("#{channel}"),
            tag: None,
            command_text: format!("#{channel}"),
            description: String::new(),
            metadata: Some(json!({"type": "slack-channel", "channel": channel})),
            color: None,
        })
        .collect()
}

pub fn clear_slack_channel_cache() {
    if let Ok(mut cache) = CACHE.lock() {
        cache.clear();
    }
    if let Ok(mut known) = KNOWN_CHANNELS.lock() {
        known.clear();
    }
    KNOWN_CHANNELS_VERSION.store(0, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_channels_matches_official_name_lines_and_filters_invalid_values() {
        assert_eq!(
            parse_channels("Name: #general\nName: team_dev\nName: #general\nName: #bad channel\n"),
            vec!["general", "team_dev"]
        );
    }

    #[test]
    fn compound_search_uses_complete_prefix_and_highlight_only_known_channels() {
        assert_eq!(mcp_query_for("claude-code-team-en"), "claude-code-team");
        assert_eq!(mcp_query_for("general"), "general");
        clear_slack_channel_cache();
        KNOWN_CHANNELS.lock().unwrap().insert("general".to_string());
        assert_eq!(
            find_slack_channel_positions("#general #unknown\n#general"),
            vec![0..8, 18..26]
        );
        clear_slack_channel_cache();
    }

    /// CC `fetchChannels` (`:29-65`): `slack_search_channels` goes straight
    /// to the first connected record whose name contains `slack`; with none,
    /// the query is cached as no channels.
    #[test]
    fn channel_suggestions_call_the_slack_client_among_the_records() {
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
                fixture.reply(
                    SEARCH_TOOL,
                    json!({ "content": [{
                        "type": "text",
                        "text": "Name: #general\nName: #gen-ai\nName: #random",
                    }] }),
                );
                let slack = crate::services::mcp::client::connect_to_server(
                    "team-slack",
                    &fixture.config(),
                )
                .await;
                clear_slack_channel_cache();

                let clients = [
                    McpServerConnection::failed(
                        "slack-down",
                        crate::services::mcp::types::ScopedMcpServerConfig::stdio_for_test("x"),
                        None,
                    ),
                    slack,
                ];
                let ids = get_slack_channel_suggestions(&clients, "gen")
                    .await
                    .into_iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>();
                assert_eq!(ids, ["slack-channel-gen-ai", "slack-channel-general"]);
                let calls = fixture.tool_calls();
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0]["name"], SEARCH_TOOL);
                assert_eq!(
                    calls[0]["arguments"],
                    json!({
                        "query": "gen",
                        "limit": 20,
                        "channel_types": "public_channel,private_channel",
                    })
                );

                assert!(get_slack_channel_suggestions(&[], "zz").await.is_empty());
                assert_eq!(CACHE.lock().unwrap().get("zz"), Some(&Vec::new()));

                clear_slack_channel_cache();
                crate::services::mcp::client::clear_server_cache("team-slack", None).await;
            });
    }

    #[test]
    fn unwrap_results_accepts_slack_json_envelope() {
        assert_eq!(unwrap_results(r#"{"results":"Name: #ops"}"#), "Name: #ops");
        assert_eq!(unwrap_results("Name: #ops"), "Name: #ops");
    }
}
