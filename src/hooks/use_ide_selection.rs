//! Maps to: CC `hooks/useIdeSelection.ts`.
//!
//! Selection types, parsing, and the hook. Ownership matches CC: the REPL owns
//! the selection state (`REPL.tsx:1110-1113` `useState<IDESelection|undefined>`)
//! and [`use_ide_selection`] registers the `selection_changed` handler on the
//! IDE client itself (CC `client.setNotificationHandler(SelectionChangedSchema,
//! ...)`). No AppState field and no process-static writer are involved.

use iocraft::prelude::*;

/// Maps to: CC `hooks/useIdeSelection.ts#SelectionPoint`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionPoint {
    pub line: u32,
    pub character: u32,
}

/// Maps to: CC `hooks/useIdeSelection.ts#SelectionData`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionData {
    pub selection: Option<SelectionRange>,
    pub text: Option<String>,
    pub file_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: SelectionPoint,
    pub end: SelectionPoint,
}

/// Maps to: CC `hooks/useIdeSelection.ts#IDESelection`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdeSelection {
    pub file_path: Option<String>,
    pub text: Option<String>,
    pub line_count: u32,
    /// Maps to: CC `IDESelection.lineStart`.
    pub line_start: Option<u32>,
}

/// Maps to: CC notification method literal `selection_changed`.
pub const SELECTION_CHANGED_METHOD: &str = "selection_changed";

/// Maps to: CC `getConnectedIdeClient` / `useIdeConnectionStatus` name gate —
/// strictly `client.name === 'ide'` (utils/ide.ts:1251,
/// useIdeConnectionStatus.ts:15). Case-sensitive exact match; `ideName` in the
/// config never widens this gate.
pub fn is_ide_mcp_server_name(name: &str) -> bool {
    name == "ide"
}

/// Maps to: CC `useIdeSelection.ts:77-82` — the reset OBJECT passed to
/// `onSelect` when the IDE client identity changes. CC stores this empty
/// selection object (NOT `undefined`); only submit clears the state to
/// `undefined` (REPL.tsx:4525). Preserve the shape — do not collapse to `None`.
pub fn ide_selection_reset() -> IdeSelection {
    IdeSelection {
        line_count: 0,
        line_start: None,
        text: None,
        file_path: None,
    }
}

/// Maps to: CC `useIdeSelection` `selectionChangeHandler` line-count math.
pub fn ide_selection_from_selection_data(data: &SelectionData) -> Option<IdeSelection> {
    let range = data.selection?;
    let mut line_count = range
        .end
        .line
        .saturating_sub(range.start.line)
        .saturating_add(1);
    // If on the first character of the line, do not count the line as selected.
    if range.end.character == 0 {
        line_count = line_count.saturating_sub(1);
    }
    Some(IdeSelection {
        line_count,
        line_start: Some(range.start.line),
        text: data.text.clone(),
        file_path: data.file_path.clone(),
    })
}

/// Parse MCP `selection_changed` params (CC `SelectionChangedSchema.params`).
pub fn parse_selection_changed_params(params: Option<&serde_json::Value>) -> Option<SelectionData> {
    let obj = params?.as_object()?;
    let text = obj.get("text").and_then(|v| v.as_str()).map(str::to_string);
    let file_path = obj
        .get("filePath")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let selection = match obj.get("selection") {
        None | Some(serde_json::Value::Null) => None,
        Some(sel) => {
            let sel = sel.as_object()?;
            let start = parse_selection_point(sel.get("start")?)?;
            let end = parse_selection_point(sel.get("end")?)?;
            Some(SelectionRange { start, end })
        }
    };

    Some(SelectionData {
        selection,
        text,
        file_path,
    })
}

fn parse_selection_point(value: &serde_json::Value) -> Option<SelectionPoint> {
    let obj = value.as_object()?;
    let line = obj.get("line")?.as_u64()? as u32;
    let character = obj.get("character")?.as_u64()? as u32;
    Some(SelectionPoint { line, character })
}

/// Maps to: CC `useIdeSelection` handler → `onSelect(selection)`.
///
/// Returns `Some` when the notification carries a usable range (same gate as
/// CC `selectionChangeHandler`). Text-only / null-selection payloads are no-ops
/// (useIdeSelection.ts:131-138 routes them to a `data.selection?.start` gate
/// that never fires — effectively dead in CC too).
pub fn ide_selection_from_notification_params(
    params: Option<&serde_json::Value>,
) -> Option<IdeSelection> {
    let data = parse_selection_changed_params(params)?;
    ide_selection_from_selection_data(&data)
}

/// Maps to: CC `hooks/useIdeSelection.ts:59-150`
/// `useIdeSelection(mcpClients, onSelect)`: when the connected IDE client
/// changes, the selection resets, and the new client gets a
/// `selection_changed` handler that delivers to `on_select` while it is still
/// the current one.
///
/// CC's effect reruns on every `mcpClients` change but acts only when
/// `getConnectedIdeClient` returns a different client; keyed on that
/// client's connection, this effect runs exactly then. CC compares the
/// connected record by reference, and a record is rebuilt on every
/// `updateServer`, the IDE's own `list_changed` included; records here are
/// values, so the comparison is the connection: a refreshed record of the
/// same connection is no change. The current client is remembered by its
/// connection id, so the handler, stored on the client, holds no handle to
/// it.
pub fn use_ide_selection(
    hooks: &mut Hooks<'_, '_>,
    mcp_clients: &[crate::services::mcp::types::McpServerConnection],
    on_select: async_channel::Sender<IdeSelection>,
) {
    let current_ide: CurrentIdeConnection = hooks.use_const(Default::default);
    let ide_client = crate::utils::ide::get_connected_ide_client(mcp_clients)
        .map(|client| client.client.clone());
    let ide_connection = ide_client
        .as_ref()
        .map(crate::services::mcp::client::McpClient::connection_id);
    hooks.use_effect(
        move || run_ide_selection_effect(&current_ide, ide_client, &on_select),
        ide_connection,
    );
}

/// The connection id of the IDE client [`use_ide_selection`] last saw
/// (CC `currentIDERef`).
pub(crate) type CurrentIdeConnection = std::sync::Arc<std::sync::Mutex<Option<u64>>>;

/// The body of [`use_ide_selection`]'s effect (CC `useIdeSelection.ts:66-149`)
/// for the connected IDE client now.
pub(crate) fn run_ide_selection_effect(
    current_ide: &CurrentIdeConnection,
    ide_client: Option<crate::services::mcp::client::McpClient>,
    on_select: &async_channel::Sender<IdeSelection>,
) {
    let ide_connection = ide_client
        .as_ref()
        .map(crate::services::mcp::client::McpClient::connection_id);
    {
        let mut current = current_ide
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // CC `:73-83`: undefined and null both mean no IDE, so a run without
        // one after none is no change.
        if *current != ide_connection {
            *current = ide_connection;
            let _ = on_select.try_send(ide_selection_reset());
        }
    }
    let (Some(ide_client), Some(registered)) = (ide_client, ide_connection) else {
        return;
    };
    let current_ide = current_ide.clone();
    let on_select = on_select.clone();
    ide_client.set_notification_handler(SELECTION_CHANGED_METHOD, move |params| {
        let Some(selection) = ide_selection_from_notification_params(params.as_ref()) else {
            return;
        };
        // CC `:115-117`: a handler of a client no longer current is ignored.
        // The check holds the lock through the send, as the effect holds it
        // through its reset: CC runs both on one thread, so a selection of a
        // client that just stopped being current never lands after the
        // reset.
        let current = current_ide
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *current == Some(registered) {
            let _ = on_select.try_send(selection);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_count_matches_official_end_character_zero_rule() {
        let data = SelectionData {
            selection: Some(SelectionRange {
                start: SelectionPoint {
                    line: 10,
                    character: 0,
                },
                end: SelectionPoint {
                    line: 12,
                    character: 0,
                },
            }),
            text: Some("abc".into()),
            file_path: Some("/tmp/a.rs".into()),
        };
        let sel = ide_selection_from_selection_data(&data).unwrap();
        assert_eq!(sel.line_count, 2);
        assert_eq!(sel.line_start, Some(10));
        assert_eq!(sel.text.as_deref(), Some("abc"));
    }

    #[test]
    fn parse_selection_changed_params_reads_official_shape() {
        let params = serde_json::json!({
            "selection": {
                "start": { "line": 1, "character": 0 },
                "end": { "line": 1, "character": 4 }
            },
            "text": "code",
            "filePath": "/repo/main.rs"
        });
        let sel = ide_selection_from_notification_params(Some(&params)).unwrap();
        assert_eq!(sel.line_count, 1);
        assert_eq!(sel.file_path.as_deref(), Some("/repo/main.rs"));
        assert_eq!(sel.text.as_deref(), Some("code"));
    }

    #[test]
    fn null_selection_payload_is_noop_like_official_handler() {
        let params = serde_json::json!({
            "selection": null,
            "text": "",
            "filePath": "/repo/main.rs"
        });
        assert!(ide_selection_from_notification_params(Some(&params)).is_none());
    }

    /// Maps to: CC useIdeSelection.ts:77-82 — the identity-change reset is the
    /// empty selection OBJECT (stored as `Some`), never `undefined`/`None`, and
    /// it must hide the footer row without conflating with the submit-time
    /// `setIDESelection(undefined)` clear (REPL.tsx:4525).
    #[test]
    fn identity_change_reset_preserves_official_empty_object_shape() {
        let reset = ide_selection_reset();
        assert_eq!(
            reset,
            IdeSelection {
                line_count: 0,
                line_start: None,
                text: None,
                file_path: None,
            }
        );
        assert!(
            crate::components::ide_status_indicator::ide_status_indicator_text(
                Some(crate::hooks::notifs::ide_status_indicator::IdeConnectionStatus::Connected),
                Some(&reset),
            )
            .is_none(),
            "the CC reset object renders no ⧉ row"
        );
    }

    /// Maps to: CC utils/ide.ts:1251 + useIdeConnectionStatus.ts:15 — the IDE
    /// gate is strictly `client.name === 'ide'` (case-sensitive).
    #[test]
    fn ide_server_name_gate_is_official_exact_match() {
        assert!(is_ide_mcp_server_name("ide"));
        assert!(!is_ide_mcp_server_name("IDE"));
        assert!(!is_ide_mcp_server_name("Ide"));
        assert!(!is_ide_mcp_server_name("vscode"));
    }
}
