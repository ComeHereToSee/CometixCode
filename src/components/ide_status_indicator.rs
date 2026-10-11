//! Maps to: CC `components/IdeStatusIndicator.tsx`.
//!
//! Official `IdeStatusIndicator` reads IDE connection status from
//! `useIdeConnectionStatus(mcpClients)` and renders only an active IDE
//! selection/file indicator. Selection ownership lives in
//! [`crate::hooks::use_ide_selection`] (CC `useIdeSelection`); this component
//! receives the already-known status + selection and preserves render copy.

use crate::hooks::notifs::ide_status_indicator::IdeConnectionStatus;
use iocraft::prelude::*;

pub use crate::hooks::use_ide_selection::IdeSelection;

/// Maps to: CC `useIdeConnectionStatus.ts:15` / `utils/ide.ts:1251`
/// `getConnectedIdeClient` — IDE MCP client is connected. CC keys strictly on
/// `client.name === 'ide'` (case-sensitive exact match); `ideName` config
/// metadata never widens the gate.
pub fn ide_client_connected(mcp: &crate::state::app_state_store::McpState) -> bool {
    use crate::services::mcp::types::McpServerConnectionType;
    mcp.clients.iter().any(|server| {
        server.connection_type() == McpServerConnectionType::Connected
            && crate::hooks::use_ide_selection::is_ide_mcp_server_name(server.name())
    })
}

#[derive(Default, Props)]
pub struct IdeStatusIndicatorProps {
    pub ide_status: Option<IdeConnectionStatus>,
    pub ide_selection: Option<IdeSelection>,
}

/// Maps to: CC `components/IdeStatusIndicator.tsx#IdeStatusIndicator` render
/// decision (`connected` plus selected text or file path).
pub fn ide_status_indicator_text(
    ide_status: Option<IdeConnectionStatus>,
    ide_selection: Option<&IdeSelection>,
) -> Option<String> {
    let selection = ide_selection?;
    let should_show_ide_selection = ide_status == Some(IdeConnectionStatus::Connected)
        && (selection
            .file_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
            || (selection
                .text
                .as_deref()
                .is_some_and(|text| !text.is_empty())
                && selection.line_count > 0));

    if ide_status.is_none() || !should_show_ide_selection {
        return None;
    }

    if selection
        .text
        .as_deref()
        .is_some_and(|text| !text.is_empty())
        && selection.line_count > 0
    {
        let line_label = if selection.line_count == 1 {
            "line"
        } else {
            "lines"
        };
        return Some(format!("⧉ {} {line_label} selected", selection.line_count));
    }

    selection
        .file_path
        .as_deref()
        .and_then(ide_selection_basename)
        .map(|basename| format!("⧉ In {basename}"))
}

/// Maps to: CC footer height contribution of `<IdeStatusIndicator />`.
pub fn ide_hint_row_count(
    ide_status: Option<IdeConnectionStatus>,
    ide_selection: Option<&IdeSelection>,
) -> usize {
    usize::from(ide_status_indicator_text(ide_status, ide_selection).is_some())
}

/// Maps to Node `path.basename(...)` in CC `IdeStatusIndicator.tsx` for the
/// display-only footer copy. Supports both separators because Cometix can read
/// Windows transcript/config paths on non-Windows hosts.
pub fn ide_selection_basename(path: &str) -> Option<&str> {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

/// Maps to: CC `components/IdeStatusIndicator.tsx#IdeStatusIndicator`.
#[component]
pub fn IdeStatusIndicator(
    props: &IdeStatusIndicatorProps,
    hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let theme = hooks.use_context::<crate::utils::theme::Theme>();
    let body = if let Some(text) =
        ide_status_indicator_text(props.ide_status, props.ide_selection.as_ref())
    {
        element! {
            Text(content: text, color: theme.ide, wrap: TextWrap::Truncate)
        }
        .into_any()
    } else {
        element! { View(width: 0u32, height: 0u32) }.into_any()
    };
    let children = vec![body];

    element! {
        Fragment { #(children) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::theme;

    #[test]
    fn ide_status_indicator_text_matches_selection_and_file_copy() {
        let selected = IdeSelection {
            text: Some("abc".to_string()),
            line_count: 3,
            file_path: Some("/tmp/demo.rs".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ide_status_indicator_text(Some(IdeConnectionStatus::Connected), Some(&selected)),
            Some("⧉ 3 lines selected".to_string())
        );

        let file = IdeSelection {
            file_path: Some("C:\\Users\\me\\main.rs".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ide_status_indicator_text(Some(IdeConnectionStatus::Connected), Some(&file)),
            Some("⧉ In main.rs".to_string())
        );
    }

    #[test]
    fn ide_status_indicator_text_matches_official_gates() {
        let selected = IdeSelection {
            text: Some("abc".to_string()),
            line_count: 1,
            file_path: None,
            ..Default::default()
        };
        assert!(ide_status_indicator_text(None, Some(&selected)).is_none());
        assert!(
            ide_status_indicator_text(Some(IdeConnectionStatus::Disconnected), Some(&selected))
                .is_none()
        );
        assert!(ide_status_indicator_text(Some(IdeConnectionStatus::Connected), None).is_none());
        assert!(
            ide_status_indicator_text(
                Some(IdeConnectionStatus::Connected),
                Some(&IdeSelection {
                    text: Some(String::new()),
                    line_count: 1,
                    file_path: None,
                    ..Default::default()
                }),
            )
            .is_none()
        );
    }

    #[test]
    fn ide_status_indicator_component_renders_ide_colored_text() {
        let theme = *theme::current();
        let canvas = element! {
            ContextProvider(value: Context::owned(theme)) {
                IdeStatusIndicator(
                    ide_status: Some(IdeConnectionStatus::Connected),
                    ide_selection: Some(IdeSelection {
                        text: Some("abc".to_string()),
                        line_count: 1,
                        file_path: None,
                        ..Default::default()
                    }),
                )
            }
        }
        .render(Some(80));
        let text = canvas.to_string();

        assert!(text.contains("⧉ 1 line selected"), "canvas=\n{text}");
        assert_eq!(
            canvas
                .resolved_text_style(0, 0)
                .and_then(|style| style.color),
            Some(theme.ide)
        );
    }

    /// Maps to: CC useIdeConnectionStatus.ts:14-31 — the connected gate finds
    /// `client.name === 'ide'` exactly; other names (even with an `ideName`
    /// config) and non-connected states never count.
    #[test]
    fn ide_client_connected_requires_official_exact_ide_name() {
        use crate::services::mcp::types::McpServerConnectionType;
        use crate::services::mcp::types::{McpServerConnection, ScopedMcpServerConfig};
        use crate::state::app_state_store::McpState;

        fn server(
            name: &str,
            status: McpServerConnectionType,
            ide_name: Option<&str>,
        ) -> McpServerConnection {
            let mut config = ScopedMcpServerConfig::stdio_for_test(name);
            config.ide_name = ide_name.map(str::to_string);
            match status {
                McpServerConnectionType::Connected => {
                    McpServerConnection::connected_for_test(name, config)
                }
                McpServerConnectionType::Pending => McpServerConnection::pending(name, config),
                other => unreachable!("fixture builds connected or pending, not {other:?}"),
            }
        }
        let state_for = |clients: Vec<McpServerConnection>| McpState {
            clients,
            ..McpState::default()
        };

        assert!(ide_client_connected(&state_for(vec![server(
            "ide",
            McpServerConnectionType::Connected,
            None
        )])));
        assert!(!ide_client_connected(&state_for(vec![server(
            "IDE",
            McpServerConnectionType::Connected,
            None
        )])));
        assert!(!ide_client_connected(&state_for(vec![server(
            "vscode",
            McpServerConnectionType::Connected,
            Some("VS Code")
        )])));
        assert!(!ide_client_connected(&state_for(vec![server(
            "ide",
            McpServerConnectionType::Pending,
            None
        )])));
    }
}
