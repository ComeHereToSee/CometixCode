//! Maps to CC `commands/ide/ide.tsx`.

use crate::commands::Command;
use crate::components::design_system::dialog::Dialog;
use crate::state::app_state_store::McpState;
use crate::tool::ToolUseContext;
use crate::utils::process_user_input::ProcessUserInputBaseResult;
use crate::utils::process_user_input::process_slash_command::{
    LocalCommandUi, SlashCommandAction, SlashCommandInvocation,
};
use crate::utils::theme::Theme;
use iocraft::prelude::*;

/// Source-shaped local JSX dispatch. Detection and the status/open flow are
/// rendered by the REPL-owned panel using the live MCP client snapshot.
pub fn dispatch(
    command: &Command,
    args: &str,
    _uuid: Option<String>,
    _context: &ToolUseContext,
) -> ProcessUserInputBaseResult {
    ProcessUserInputBaseResult {
        messages: Vec::new(),
        should_query: false,
        allowed_tools: None,
        local_action: Some(SlashCommandAction::OpenLocalCommandUi {
            command: LocalCommandUi::Ide {
                args: args.to_string(),
            },
            invocation: SlashCommandInvocation::new(command.name.as_ref(), args),
        }),
        query_source: crate::constants::query_source::QuerySource::Prompt,
    }
}

#[derive(Default, Props)]
pub struct IdeCommandProps<'a> {
    pub args: String,
    pub mcp_state: McpState,
    pub on_done: HandlerMut<'a, String>,
}

fn connected_ide_labels(mcp_state: &McpState) -> Vec<String> {
    mcp_state
        .clients
        .iter()
        .filter(|server| crate::hooks::use_ide_selection::is_ide_mcp_server_name(server.name()))
        .map(|server| {
            let name = server
                .config()
                .ide_name
                .clone()
                .unwrap_or_else(|| "IDE".to_string());
            format!("{name} ({})", server.connection_type().as_str())
        })
        .collect()
}

fn status_message(args: &str, mcp_state: &McpState) -> String {
    let clients = connected_ide_labels(mcp_state);
    let has_open = args.trim() == "open";
    if clients.is_empty() {
        if has_open {
            "No IDEs with Claude Code extension detected.".to_string()
        } else {
            "No connected IDE detected. Start an IDE with the Claude Code extension and run /ide again."
                .to_string()
        }
    } else if has_open {
        format!(
            "Connected IDE: {}\nOpening the project is not available in this build.",
            clients.join(", ")
        )
    } else {
        format!("Connected IDEs:\n{}", clients.join("\n"))
    }
}

/// Maps to CC `commands/ide/ide.tsx` status surface. The live MCP manager is
/// the Rust source of truth for an already connected IDE; process/lockfile
/// discovery and extension installation remain explicit utility seams.
#[component]
pub fn IdeCommand<'a>(
    props: &mut IdeCommandProps<'a>,
    mut hooks: Hooks,
) -> impl Into<AnyElement<'static>> {
    let theme = hooks.use_context::<Theme>();
    let mut pending = hooks.use_state(|| Option::<String>::None);
    let summary = status_message(&props.args, &props.mcp_state);
    let summary_for_event = summary.clone();
    hooks.use_terminal_events({
        let mut pending = pending;
        move |event| {
            let TerminalEvent::Key(KeyEvent { code, kind, .. }) = event else {
                return;
            };
            if kind == KeyEventKind::Release {
                return;
            }
            match code {
                KeyCode::Enter => pending.set(Some(summary_for_event.clone())),
                KeyCode::Esc => pending.set(Some("IDE dialog dismissed".to_string())),
                _ => {}
            }
        }
    });
    let output = pending.read().clone();
    if let Some(output) = output {
        pending.set(None);
        (props.on_done)(output);
    }
    element! {
        Dialog(
            title: "IDE integrations".to_string(),
            color: Some(theme.ide),
            on_cancel: move |_| pending.set(Some("IDE dialog dismissed".to_string())),
        ) {
            Text(content: summary, wrap: TextWrap::Wrap)
            Text(
                content: "Press Enter to continue · Esc to cancel".to_string(),
                dim: true,
                wrap: TextWrap::NoWrap,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_message_matches_no_connection_source_branches() {
        let state = McpState::default();
        assert_eq!(
            status_message("", &state),
            "No connected IDE detected. Start an IDE with the Claude Code extension and run /ide again."
        );
        assert_eq!(
            status_message("open", &state),
            "No IDEs with Claude Code extension detected."
        );
    }

    #[test]
    fn ide_labels_take_the_configured_ide_name_or_fall_back_to_ide() {
        use crate::services::mcp::types::{McpServerConnection, ScopedMcpServerConfig};
        let unnamed = McpState {
            clients: vec![McpServerConnection::pending(
                "ide",
                ScopedMcpServerConfig::stdio_for_test("ide"),
            )],
            ..McpState::default()
        };
        assert_eq!(connected_ide_labels(&unnamed), ["IDE (pending)"]);

        let mut config = ScopedMcpServerConfig::stdio_for_test("ide");
        config.ide_name = Some("Cursor".to_string());
        let named = McpState {
            clients: vec![McpServerConnection::connected_for_test("ide", config)],
            ..McpState::default()
        };
        assert_eq!(connected_ide_labels(&named), ["Cursor (connected)"]);
    }
}
