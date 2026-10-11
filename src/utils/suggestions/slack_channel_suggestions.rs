//! Maps to: CC `utils/suggestions/slackChannelSuggestions.ts`.
//!
//! The MCP round-trip, result-envelope parsing and prefix cache stay in this
//! producer owner. `use_typeahead` only decides when a `#channel` token should
//! request it and how the returned rows are selected.

use crate::components::prompt_input::prompt_input_footer_suggestions::SuggestionItem;
use crate::services::mcp::types::{McpServerConnectionType, McpServerSnapshot};
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

fn find_slack_server(clients: &[McpServerSnapshot]) -> Option<String> {
    clients
        .iter()
        .find(|server| {
            server.connection_type() == McpServerConnectionType::Connected
                // Source uses `client.name.includes('slack')` without a
                // case-fold. Preserve that protocol-facing gate exactly.
                && server.name().contains("slack")
        })
        .map(|server| server.name().to_owned())
}

/// Maps to: CC `hasSlackMcpServer`.
pub fn has_slack_mcp_server(clients: &[McpServerSnapshot]) -> bool {
    find_slack_server(clients).is_some()
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

async fn fetch_channels(server_name: &str, query: &str) -> Vec<String> {
    let args = Map::from_iter([
        ("query".to_string(), json!(query)),
        ("limit".to_string(), json!(20)),
        (
            "channel_types".to_string(),
            json!("public_channel,private_channel"),
        ),
    ]);
    let Ok(result) =
        crate::services::mcp::client::call_mcp_tool(server_name, SEARCH_TOOL, args).await
    else {
        return Vec::new();
    };
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            (entry.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| entry.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n");
    parse_channels(&unwrap_results(&content))
}

/// Maps to: CC `getSlackChannelSuggestions`.
pub async fn get_slack_channel_suggestions(
    clients: &[McpServerSnapshot],
    search_token: &str,
) -> Vec<SuggestionItem> {
    let token = search_token.to_ascii_lowercase();
    if token.is_empty() {
        return Vec::new();
    }
    let Some(server_name) = find_slack_server(clients) else {
        return Vec::new();
    };
    let mcp_query = mcp_query_for(&token);
    let cached = CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&mcp_query).cloned())
        .or_else(|| find_reusable_cache_entry(&mcp_query, &token));
    let channels = if let Some(cached) = cached {
        cached
    } else {
        let channels = fetch_channels(&server_name, &mcp_query).await;
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

    #[test]
    fn unwrap_results_accepts_slack_json_envelope() {
        assert_eq!(unwrap_results(r#"{"results":"Name: #ops"}"#), "Name: #ops");
        assert_eq!(unwrap_results("Name: #ops"), "Name: #ops");
    }
}
