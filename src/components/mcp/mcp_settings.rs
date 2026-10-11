//! Maps to: CC `components/mcp/MCPSettings.tsx`.
//!
//! This component owns the official MCP settings view-state boundary and builds
//! `ServerInfo` from already-known runtime/config snapshots. It does not start
//! clients, open OAuth flows, list tools/resources, or write config.

use super::mcp_agent_server_menu::MCPAgentServerMenu;
use super::mcp_list_panel::MCPListPanel;
use super::mcp_remote_server_menu::MCPRemoteServerMenu;
use super::mcp_stdio_server_menu::MCPStdioServerMenu;
use super::mcp_tool_detail_view::MCPToolDetailView;
use super::mcp_tool_list_view::MCPToolListView;
use super::types::{
    MCPViewState, McpToolInfo, McpToolParameterInfo, ServerInfo, mcp_client_state_from_parts,
};
use crate::services::mcp::config::all_configured_mcp_servers_readonly;
use crate::services::mcp::types::McpServerConnection;
use crate::services::mcp::types::{
    ConfigScope, McpServerConnectionType, ScopedMcpServerConfig, Transport,
};
use crate::utils::config::{GlobalConfig, ProjectConfig, normalize_project_path};
use crate::utils::status::StartupDiagnosticsSnapshot;
use iocraft::prelude::*;
use std::collections::BTreeMap;

fn parameter_infos_from_schema(schema: &serde_json::Value) -> Vec<McpToolParameterInfo> {
    let required = schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_default();
    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    properties
        .iter()
        .map(|(name, value)| McpToolParameterInfo {
            name: name.clone(),
            type_name: value
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown")
                .to_string(),
            description: value
                .get("description")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            required: required.contains(name),
        })
        .collect()
}

/// An `mcp.tools` entry as CC's tool views read it, off the tool itself:
/// `userFacingName()` (`MCPToolListView.tsx:36-42`, `MCPToolDetailView.tsx`),
/// i.e. the FULL `` `${client.name} - ${displayName} (MCP)` `` string that
/// `extractMcpToolDisplayName` then parses; `description()`; and the three
/// annotation methods.
impl From<&crate::types::tools::Tool> for McpToolInfo {
    fn from(tool: &crate::types::tools::Tool) -> Self {
        Self {
            name: tool.name.clone(),
            user_facing_name: tool.mcp_user_facing_name(),
            description: Some(tool.mcp_description.clone()),
            is_read_only: tool.mcp_annotations.read_only_hint,
            is_destructive: tool.mcp_annotations.destructive_hint,
            is_open_world: tool.mcp_annotations.open_world_hint,
            parameters: parameter_infos_from_schema(&tool.input_schema),
        }
    }
}

pub const NO_MCP_SERVERS_CONFIGURED: &str = "No MCP servers configured. Please run /doctor if this is unexpected. Otherwise, run `claude mcp --help` or visit https://code.claude.com/docs/en/mcp to learn more.";

#[derive(Default, Props)]
pub struct MCPSettingsProps {
    pub on_complete: Handler<String>,
}

fn project_config_for_current_cwd(global_config: &GlobalConfig) -> ProjectConfig {
    let cwd = std::env::current_dir().unwrap_or_default();
    let key = normalize_project_path(&cwd.to_string_lossy());
    global_config
        .projects
        .get(&key)
        .cloned()
        .unwrap_or_default()
}

pub fn server_infos_from_client_snapshots(
    global_config: &GlobalConfig,
    project_config: &ProjectConfig,
    runtime_clients: &[crate::services::mcp::types::McpClientSnapshot],
) -> Vec<ServerInfo> {
    server_infos_from_server_snapshots(global_config, project_config, &[], runtime_clients)
}

fn remote_server_authentication_state_for_settings(
    name: &str,
    config: &ScopedMcpServerConfig,
    client_type: McpServerConnectionType,
    tools_count: usize,
) -> Option<bool> {
    // Maps to: CC `components/mcp/MCPSettings.tsx#prepareServers`
    // `isAuthenticated` derivation for SSE/HTTP/claudeai-proxy clients.
    match config.transport {
        Transport::Sse | Transport::Http => {
            let has_oauth_tokens =
                crate::services::mcp::auth::ClaudeAuthProvider::new(name, config)
                    .tokens()
                    .ok()
                    .flatten()
                    .is_some();
            let has_session_auth = client_type == McpServerConnectionType::Connected
                && crate::utils::session_ingress_auth::get_session_ingress_auth_token().is_some();
            let has_tools_and_connected =
                client_type == McpServerConnectionType::Connected && tools_count > 0;
            Some(has_oauth_tokens || has_session_auth || has_tools_and_connected)
        }
        Transport::ClaudeAiProxy => Some(false),
        _ => None,
    }
}

pub fn server_infos_from_server_snapshots(
    global_config: &GlobalConfig,
    project_config: &ProjectConfig,
    runtime_servers: &[McpServerConnection],
    fallback_clients: &[crate::services::mcp::types::McpClientSnapshot],
) -> Vec<ServerInfo> {
    server_infos_from_mcp_state(
        global_config,
        project_config,
        &crate::state::app_state_store::McpState {
            clients: runtime_servers.to_vec(),
            ..Default::default()
        },
        fallback_clients,
    )
}

/// Maps to CC `MCPSettings.tsx` consuming the already-flattened
/// `mcp.tools`/`mcp.commands` arrays.
pub fn server_infos_from_mcp_state(
    global_config: &GlobalConfig,
    project_config: &ProjectConfig,
    mcp: &crate::state::app_state_store::McpState,
    fallback_clients: &[crate::services::mcp::types::McpClientSnapshot],
) -> Vec<ServerInfo> {
    server_infos_from_capability_arrays(
        global_config,
        project_config,
        &mcp.clients,
        &mcp.tools,
        &mcp.commands,
        &mcp.resources,
        fallback_clients,
    )
}

/// Maps to: CC `components/mcp/MCPSettings.tsx:56-117` nested
/// `prepareServers`, which projects existing MCP client state without
/// precomputing the remote menu's Claude.ai authentication URL.
fn server_infos_from_capability_arrays(
    global_config: &GlobalConfig,
    project_config: &ProjectConfig,
    runtime_servers: &[McpServerConnection],
    runtime_tools: &[crate::types::tools::Tool],
    runtime_commands: &[crate::commands::Command],
    runtime_resources: &BTreeMap<String, Vec<crate::services::mcp::types::ServerResource>>,
    fallback_clients: &[crate::services::mcp::types::McpClientSnapshot],
) -> Vec<ServerInfo> {
    let mut configs = all_configured_mcp_servers_readonly(global_config, project_config);
    let runtime_by_name = runtime_servers
        .iter()
        .map(|server| (server.name().to_owned(), server))
        .collect::<BTreeMap<_, _>>();
    let client_statuses = fallback_clients
        .iter()
        .map(|client| (client.name.clone(), client.status))
        .collect::<BTreeMap<_, _>>();

    for server in runtime_servers {
        configs
            .entry(server.name().to_owned())
            .or_insert_with(|| server.config().clone());
    }
    for client in fallback_clients {
        configs
            .entry(client.name.clone())
            .or_insert_with(|| ScopedMcpServerConfig {
                name: None,
                scope: ConfigScope::Project,
                transport: Transport::Stdio,
                command: None,
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
                url: None,
                headers: std::collections::BTreeMap::new(),
                headers_helper: None,
                oauth: None,
                ide_running_in_windows: None,
                ide_name: None,
                auth_token: None,
                id: None,
                plugin_source: None,
            });
    }

    let mut servers = configs
        .into_iter()
        .filter(|(name, _)| name != "ide")
        .map(|(name, config)| {
            let runtime = runtime_by_name.get(&name).copied();
            let config = runtime.map_or(config, |server| server.config().clone());
            let status = runtime
                .map(|server| server.connection_type())
                .or_else(|| client_statuses.get(&name).copied())
                .unwrap_or(McpServerConnectionType::Pending);
            let tools = crate::services::mcp::utils::filter_tools_by_server(runtime_tools, &name)
                .iter()
                .map(McpToolInfo::from)
                .collect::<Vec<_>>();
            let client_type = status;
            let resources_count = runtime_resources.get(&name).map_or(0, Vec::len);
            let client = mcp_client_state_from_parts(
                client_type,
                tools.clone(),
                resources_count,
                match runtime {
                    Some(McpServerConnection::Failed(failed)) => failed.error.clone(),
                    _ => None,
                },
                None,
            );
            let (reconnect_attempt, max_reconnect_attempts) = match runtime {
                Some(McpServerConnection::Pending(pending)) => {
                    (pending.reconnect_attempt, pending.max_reconnect_attempts)
                }
                _ => (None, None),
            };
            let is_authenticated = remote_server_authentication_state_for_settings(
                &name,
                &config,
                client_type,
                tools.len(),
            );
            // CC's server menus count `filterMcpPromptsByServer(mcp.commands,
            // name)` (`MCPRemoteServerMenu.tsx:277`, `MCPStdioServerMenu.tsx:75`).
            let prompts_count =
                crate::services::mcp::utils::filter_mcp_prompts_by_server(runtime_commands, &name)
                    .len();
            ServerInfo {
                name,
                client,
                client_type,
                scope: config.scope,
                transport: config.transport,
                is_authenticated,
                config,
                reconnect_attempt,
                max_reconnect_attempts,
                tools,
                prompts_count,
                resources_count,
            }
        })
        .collect::<Vec<_>>();
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    servers
}

/// Maps to CC `MCPSettings.tsx` server-tools/detail render paths, where the
/// selected server name is kept in view state but tool data is read from live
/// `mcp.tools` every render.
fn latest_server_for_view_state(servers: &[ServerInfo], server: ServerInfo) -> ServerInfo {
    servers
        .iter()
        .find(|candidate| candidate.name == server.name)
        .cloned()
        .unwrap_or(server)
}

fn latest_tool_for_detail(
    servers: &[ServerInfo],
    server: ServerInfo,
    tool_index: usize,
) -> (ServerInfo, Option<McpToolInfo>) {
    let server = latest_server_for_view_state(servers, server);
    let tool = server.tools.get(tool_index).cloned();
    (server, tool)
}

#[component]
pub fn MCPSettings(props: &MCPSettingsProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let mut view_state = hooks.use_state(MCPViewState::default);
    let mut did_complete_empty = hooks.use_state(|| false);
    let runtime_mcp =
        crate::state::app_state::use_app_state(&mut hooks, |state| (*state.mcp).clone());
    let servers = if let Some(ctx) = hooks.try_use_context::<StartupDiagnosticsSnapshot>() {
        // Maps to: CC `getGlobalConfig()` cached direct reads; the snapshot
        // carries only the startup fallback client list.
        let global_config = crate::utils::config::load_global_config();
        let project_config = project_config_for_current_cwd(&global_config);
        // Without a store the state is empty: no clients, and empty
        // `mcp.tools` / `.commands` / `.resources`, so every configured
        // server shows with no capabilities.
        server_infos_from_mcp_state(
            &global_config,
            &project_config,
            &runtime_mcp,
            &ctx.mcp_clients,
        )
    } else {
        // Maps to: CC `MCPSettings.tsx` reading `useAppState(s => s.mcp)`
        // directly.
        server_infos_from_mcp_state(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &runtime_mcp,
            &[],
        )
    };
    // Maps to CC `MCPSettings.tsx` `extractAgentMcpServers(agentDefinitions.allAgents)`.
    let agent_servers = std::env::current_dir()
        .ok()
        .map(|cwd| {
            let definitions =
                crate::tools::agent_tool::load_agents_dir::get_agent_definitions_with_overrides(
                    &cwd,
                );
            crate::services::mcp::utils::extract_agent_mcp_servers(&definitions.all_agents)
        })
        .unwrap_or_default();

    let no_servers = servers.is_empty() && agent_servers.is_empty();
    hooks.use_effect(
        {
            let mut did_complete_empty = did_complete_empty;
            let on_complete = props.on_complete.clone();
            move || {
                // Maps to: CC `MCPSettings.tsx` `useEffect` no-server branch,
                // which calls `onComplete(NO_MCP_SERVERS_CONFIGURED)` instead
                // of rendering a no-client management panel.
                if no_servers && !did_complete_empty.get() {
                    did_complete_empty.set(true);
                    on_complete(NO_MCP_SERVERS_CONFIGURED.to_string());
                } else if !no_servers && did_complete_empty.get() {
                    did_complete_empty.set(false);
                }
            }
        },
        no_servers,
    );
    if no_servers {
        return element! { View {} }.into_any();
    }

    let current_view_state = { view_state.read().clone() };
    match current_view_state {
        MCPViewState::List { default_tab } => {
            let mut select_state = view_state;
            let mut select_agent_state = view_state;
            element! {
                MCPListPanel(
                    servers: servers,
                    agent_servers: agent_servers,
                    default_tab: default_tab,
                    on_select_server: move |server: ServerInfo| {
                        select_state.set(MCPViewState::ServerMenu { server });
                    },
                    on_select_agent_server: move |agent_server| {
                        select_agent_state.set(MCPViewState::AgentServerMenu { agent_server });
                    },
                    on_complete: props.on_complete.clone(),
                )
            }
            .into_any()
        }
        MCPViewState::ServerMenu { server } => {
            // Keep the selected identity in view state, but source capability
            // counts/tool rows from the live flat MCP arrays every render.
            let server = latest_server_for_view_state(&servers, server);
            let default_tab = if server.transport == Transport::ClaudeAiProxy {
                "claude.ai".to_string()
            } else {
                "Claude Code".to_string()
            };
            if server.transport == Transport::Stdio {
                let mut cancel_state = view_state;
                let mut tools_state = view_state;
                let server_for_tools = server.clone();
                let default_tab_for_cancel = default_tab.clone();
                element! {
                    MCPStdioServerMenu(
                        server: Some(server),
                        on_view_tools: move |_| {
                            tools_state.set(MCPViewState::ServerTools { server: server_for_tools.clone() });
                        },
                        on_cancel: move |_| {
                            cancel_state.set(MCPViewState::List { default_tab: Some(default_tab_for_cancel.clone()) });
                        },
                        on_complete: props.on_complete.clone(),
                    )
                }
                .into_any()
            } else {
                let mut cancel_state = view_state;
                let mut tools_state = view_state;
                let server_for_tools = server.clone();
                let default_tab_for_cancel = default_tab.clone();
                element! {
                    MCPRemoteServerMenu(
                        server: Some(server),
                        on_view_tools: move |_| {
                            tools_state.set(MCPViewState::ServerTools { server: server_for_tools.clone() });
                        },
                        on_cancel: move |_| {
                            cancel_state.set(MCPViewState::List { default_tab: Some(default_tab_for_cancel.clone()) });
                        },
                        on_complete: props.on_complete.clone(),
                    )
                }
                .into_any()
            }
        }
        MCPViewState::ServerTools { server } => {
            // Maps to CC `MCPToolListView.tsx`: the tool list is derived from
            // live `mcp.tools` each render, not the stale server snapshot kept
            // inside view state.
            let server = latest_server_for_view_state(&servers, server);
            let mut back_state = view_state;
            let mut detail_state = view_state;
            let server_for_detail = server.clone();
            let server_for_back = server.clone();
            element! {
                MCPToolListView(
                    server: Some(server),
                    on_select_tool: move |tool_index: usize| {
                        detail_state.set(MCPViewState::ServerToolDetail {
                            server: server_for_detail.clone(),
                            tool_index,
                        });
                    },
                    on_back: move |_| {
                        back_state.set(MCPViewState::ServerMenu { server: server_for_back.clone() });
                    },
                )
            }
            .into_any()
        }
        MCPViewState::ServerToolDetail { server, tool_index } => {
            // Maps to CC `MCPSettings.tsx` `server-tool-detail`: re-read the
            // selected tool from live MCP tools by index and bounce back to
            // `server-tools` if it disappeared after a list_changed refresh.
            let (server, tool) = latest_tool_for_detail(&servers, server, tool_index);
            if tool.is_none() {
                view_state.set(MCPViewState::ServerTools { server });
                return element! { View {} }.into_any();
            }
            let mut back_state = view_state;
            let server_for_back = server.clone();
            element! {
                MCPToolDetailView(
                    tool: tool,
                    server: Some(server),
                    on_back: move |_| {
                        back_state.set(MCPViewState::ServerTools { server: server_for_back.clone() });
                    },
                )
            }
            .into_any()
        }
        MCPViewState::AgentServerMenu { agent_server } => {
            let mut cancel_state = view_state;
            element! {
                MCPAgentServerMenu(
                    agent_server: Some(agent_server),
                    on_cancel: move |_| {
                        cancel_state.set(MCPViewState::List { default_tab: Some("Agents".to_string()) });
                    },
                    on_complete: props.on_complete.clone(),
                )
            }
            .into_any()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::mcp::types::{McpClientSnapshot, McpServerConnection, ServerResource};
    use crate::utils::test_env::TEST_ENV_LOCK;
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn mcp_settings_no_servers_completes_instead_of_rendering_no_client_panel() {
        let results = Arc::new(Mutex::new(Vec::<String>::new()));
        let results_for_handler = Arc::clone(&results);
        let last = Arc::new(Mutex::new(String::new()));
        let last_for_loop = Arc::clone(&last);

        futures::executor::block_on(async move {
            let mut app = element! {
                // Reads `state.mcp`. Default state IS the fixture here — this
                // test asserts the empty-server path completes instead of
                // rendering a panel, and default AppState has no MCP clients.
                crate::state::app_state::AppStateProvider(
                    children: crate::state::app_state::ProviderChildren::new(move || {
                        let results_for_handler = Arc::clone(&results_for_handler);
                        element! {
                            MCPSettings(
                                on_complete: move |result: String| {
                                    results_for_handler.lock().expect("results mutex").push(result);
                                }
                            )
                        }.into_any()
                    }),
                )
            };
            let mut render_loop = Box::pin(
                app.mock_terminal_render_loop(MockTerminalConfig::default().with_size(120, 24)),
            );
            for _ in 0..4 {
                let next = crate::utils::race(render_loop.next(), async {
                    futures_timer::Delay::new(Duration::from_millis(100)).await;
                    None
                })
                .await;
                let Some(canvas) = next else {
                    break;
                };
                *last_for_loop.lock().expect("last canvas mutex") = canvas.to_string();
            }
        });

        let text = last.lock().expect("last canvas mutex").clone();
        assert_eq!(
            results.lock().expect("results mutex").as_slice(),
            &[NO_MCP_SERVERS_CONFIGURED.to_string()]
        );
        assert!(!text.contains("Manage MCP servers"), "canvas=\n{text}");
        assert!(
            !text.contains("No MCP servers configured"),
            "canvas=\n{text}"
        );
    }

    #[test]
    fn mcp_settings_consumes_flat_tools_and_commands_instead_of_nested_capability_lists() {
        let config = ScopedMcpServerConfig {
            name: None,
            scope: ConfigScope::Project,
            transport: Transport::Stdio,
            command: Some("docs-server".to_string()),
            args: Vec::new(),
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            headers_helper: None,
            oauth: None,
            ide_running_in_windows: None,
            ide_name: None,
            auth_token: None,
            id: None,
            plugin_source: None,
        };
        let server = McpServerConnection::connected_for_test("docs", config);
        let state = crate::state::app_state_store::McpState {
            plugin_reconnect_key: 0,
            clients: vec![server],
            // The flat `mcp.tools` entry, as `fetchToolsForClient` makes it.
            tools: vec![crate::services::mcp::client::mcp_tool_for_test(
                "docs",
                serde_json::json!({
                    "name": "search",
                    "description": "Search docs",
                    "inputSchema": {"type": "object"},
                    "annotations": {"title": "Search Docs", "readOnlyHint": true}
                }),
            )],
            commands: vec![crate::commands::Command::from_mcp_prompt(
                crate::services::mcp::client::McpPromptCommandSnapshot {
                    name: "mcp__docs__summarize".to_string(),
                    description: "Summarize docs".to_string(),
                    has_user_specified_description: true,
                    user_facing_name: "docs:summarize (MCP)".to_string(),
                    arg_names: Vec::new(),
                    source: "mcp",
                },
            )],
            resources: BTreeMap::from([(
                "docs".to_string(),
                vec![ServerResource {
                    server: "docs".to_string(),
                    uri: "docs://guide".to_string(),
                    name: "guide".to_string(),
                    description: None,
                    mime_type: Some("text/plain".to_string()),
                }],
            )]),
        };

        let infos = server_infos_from_mcp_state(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &state,
            &[],
        );
        let docs = infos.iter().find(|server| server.name == "docs").unwrap();
        assert_eq!(docs.tools.len(), 1);
        assert_eq!(docs.tools[0].name, "mcp__docs__search");
        assert_eq!(
            docs.tools[0].user_facing_name.as_deref(),
            Some("docs - Search Docs (MCP)")
        );
        assert_eq!(docs.tools[0].description.as_deref(), Some("Search docs"));
        assert!(docs.tools[0].is_read_only);
        assert_eq!(docs.prompts_count, 1);
        assert_eq!(docs.resources_count, 1);
    }

    #[test]
    fn mcp_settings_builds_server_infos_from_config_and_runtime_without_clients() {
        let _env_guard = TEST_ENV_LOCK.lock().unwrap();
        let temp_dir =
            std::env::temp_dir().join(format!("cometix-mcp-settings-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let previous_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let global = GlobalConfig {
            mcp_servers: Some(serde_json::json!({"github":{"command":"gh-mcp"}})),
            ..GlobalConfig::default()
        };
        let project = ProjectConfig {
            mcp_servers: Some(
                serde_json::json!({"filesystem":{"type":"http","url":"https://example.com/mcp"}}),
            ),
            ..ProjectConfig::default()
        };
        let infos = server_infos_from_client_snapshots(
            &global,
            &project,
            &[
                McpClientSnapshot {
                    name: "github".to_string(),
                    status: McpServerConnectionType::Disabled,
                    reconnect_attempt: None,
                    max_reconnect_attempts: None,
                    ide_name: None,
                    server_version: None,
                    error: None,
                },
                McpClientSnapshot {
                    name: "ide".to_string(),
                    status: McpServerConnectionType::Connected,
                    reconnect_attempt: None,
                    max_reconnect_attempts: None,
                    ide_name: Some("IDE".to_string()),
                    server_version: None,
                    error: None,
                },
            ],
        );

        assert_eq!(infos.len(), 2);
        assert!(infos.iter().any(|server| {
            server.name == "github"
                && server.official_client_type() == McpServerConnectionType::Disabled
        }));
        assert!(
            infos.iter().any(|server| {
                server.name == "filesystem" && server.scope == ConfigScope::Local
            })
        );
        assert!(!infos.iter().any(|server| server.name == "ide"));

        std::env::set_current_dir(previous_cwd).unwrap();
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn mcp_settings_needs_auth_projection_matches_official_without_precomputed_url() {
        let config = ScopedMcpServerConfig {
            name: None,
            scope: ConfigScope::Dynamic,
            transport: Transport::ClaudeAiProxy,
            command: None,
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
            url: None,
            headers: std::collections::BTreeMap::new(),
            headers_helper: None,
            oauth: None,
            ide_running_in_windows: None,
            ide_name: None,
            auth_token: None,
            id: Some("mcprs_abc".to_string()),
            plugin_source: None,
        };
        let runtime_servers = vec![McpServerConnection::needs_auth("claudeai", config)];
        let infos = server_infos_from_server_snapshots(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &runtime_servers,
            &[],
        );
        match &infos[0].client {
            super::super::types::MCPClientState::NeedsAuth { auth_url } => {
                assert!(auth_url.is_none());
            }
            other => panic!("expected needs-auth state, got {other:?}"),
        }
    }

    #[test]
    fn mcp_settings_projects_live_runtime_tools_prompts_and_resources_into_server_info() {
        let global = GlobalConfig {
            mcp_servers: Some(serde_json::json!({"github":{"command":"gh-mcp"}})),
            ..GlobalConfig::default()
        };
        let project = ProjectConfig::default();
        let mut server = McpServerConnection::connected_for_test(
            "github",
            ScopedMcpServerConfig {
                name: None,
                ide_running_in_windows: None,
                scope: ConfigScope::Dynamic,
                transport: Transport::Stdio,
                command: Some("runtime-gh-mcp".to_string()),
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
                url: None,
                headers: std::collections::BTreeMap::new(),
                headers_helper: None,
                oauth: None,
                ide_name: None,
                auth_token: None,
                id: None,
                plugin_source: None,
            },
        );
        if let McpServerConnection::Connected(connected) = &mut server {
            connected.capabilities.resources = Some(Default::default());
            connected.server_info = Some(rmcp::model::Implementation::new("github", "1.2.3"));
        }
        let runtime_servers = vec![server];
        let state = crate::state::app_state_store::McpState {
            clients: runtime_servers,
            tools: vec![crate::services::mcp::client::mcp_tool_for_test(
                "github",
                serde_json::json!({
                    "name": "create_issue",
                    "description": "Create an issue",
                    "inputSchema": {
                        "type": "object",
                        "required": ["title"],
                        "properties": {
                            "title": { "type": "string", "description": "Issue title" }
                        }
                    },
                    "annotations": {"title": "Create issue", "openWorldHint": true}
                }),
            )],
            commands: vec![crate::services::mcp::client::mcp_command_for_test(
                "github",
                "triage",
                Some("Triage issue"),
                &["issue"],
            )],
            resources: BTreeMap::from([(
                "github".to_string(),
                vec![ServerResource {
                    server: "github".to_string(),
                    uri: "repo://issues".to_string(),
                    name: "issues".to_string(),
                    description: None,
                    mime_type: None,
                }],
            )]),
            ..Default::default()
        };

        let infos = server_infos_from_mcp_state(&global, &project, &state, &[]);
        let github = infos
            .iter()
            .find(|server| server.name == "github")
            .expect("github");
        assert_eq!(
            github.official_client_type(),
            McpServerConnectionType::Connected
        );
        assert_eq!(github.scope, ConfigScope::Dynamic);
        assert_eq!(github.config.command.as_deref(), Some("runtime-gh-mcp"));
        assert_eq!(github.prompts_count, 1);
        assert_eq!(github.resources_count, 1);
        assert_eq!(github.tools.len(), 1);
        // Re-derived 2026-08-26 (#128): this asserted the bare annotation title.
        // CC's dynamic MCP tool exposes
        // `` `${client.name} - ${tool.annotations?.title || tool.name} (MCP)` ``
        // (`services/mcp/client.ts:1972-1976`), and `MCPToolListView.tsx:37-42`
        // runs `extractMcpToolDisplayName` over exactly that string to recover
        // `Create issue`. With the bare title there was nothing for
        // `mcp_string_utils::extract_mcp_tool_display_name` to strip — the
        // sibling list/detail tests already encode the full CC shape
        // (`mcp_tool_list_view.rs:205`, `mcp_tool_detail_view.rs:180`).
        assert_eq!(
            github.tools[0].user_facing_name.as_deref(),
            Some("github - Create issue (MCP)")
        );
        assert_eq!(
            crate::services::mcp::mcp_string_utils::extract_mcp_tool_display_name(
                github.tools[0].user_facing_name.as_deref().unwrap()
            ),
            "Create issue"
        );
        assert_eq!(github.tools[0].parameters[0].name, "title");
        assert!(github.tools[0].parameters[0].required);
        assert_eq!(github.is_authenticated, None);
        assert_eq!(
            github.client.client_type(),
            McpServerConnectionType::Connected
        );

        let runtime_only_infos = server_infos_from_mcp_state(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &state,
            &[],
        );
        assert_eq!(runtime_only_infos.len(), 1);
        assert_eq!(runtime_only_infos[0].name, "github");
        assert_eq!(
            runtime_only_infos[0].official_client_type(),
            McpServerConnectionType::Connected
        );
        assert_eq!(runtime_only_infos[0].scope, ConfigScope::Dynamic);
    }

    #[test]
    fn mcp_settings_tool_views_use_latest_live_server_snapshot() {
        fn info(name: &str, tool_name: &str) -> ServerInfo {
            let tools = vec![McpToolInfo {
                name: tool_name.to_string(),
                user_facing_name: None,
                description: None,
                is_read_only: true,
                is_destructive: false,
                is_open_world: false,
                parameters: Vec::new(),
            }];
            ServerInfo {
                name: name.to_string(),
                client: mcp_client_state_from_parts(
                    McpServerConnectionType::Connected,
                    tools.clone(),
                    0,
                    None,
                    None,
                ),
                client_type: McpServerConnectionType::Connected,
                scope: ConfigScope::Project,
                transport: Transport::Http,
                is_authenticated: Some(true),
                config: ScopedMcpServerConfig {
                    name: None,
                    scope: ConfigScope::Project,
                    transport: Transport::Http,
                    command: None,
                    args: Vec::new(),
                    env: std::collections::BTreeMap::new(),
                    url: Some("https://example.test/mcp".to_string()),
                    headers: std::collections::BTreeMap::new(),
                    headers_helper: None,
                    oauth: None,
                    ide_running_in_windows: None,
                    ide_name: None,
                    auth_token: None,
                    id: None,
                    plugin_source: None,
                },
                reconnect_attempt: None,
                max_reconnect_attempts: None,
                tools,
                prompts_count: 0,
                resources_count: 0,
            }
        }

        let stale = info("docs", "mcp__docs__old");
        let live = info("docs", "mcp__docs__new");
        let latest = latest_server_for_view_state(&[live.clone()], stale.clone());
        assert_eq!(latest.tools[0].name, "mcp__docs__new");

        let (latest, tool) = latest_tool_for_detail(&[live], stale, 0);
        assert_eq!(latest.name, "docs");
        assert_eq!(tool.unwrap().name, "mcp__docs__new");

        let no_tool = latest_tool_for_detail(&[], info("docs", "mcp__docs__old"), 1).1;
        assert!(no_tool.is_none());
    }

    #[test]
    fn mcp_settings_projects_pending_reconnect_attempts_into_server_info() {
        let runtime_servers = vec![McpServerConnection::Pending(
            crate::services::mcp::types::PendingMcpServer {
                name: "docs".to_string(),
                config: ScopedMcpServerConfig {
                    name: None,
                    ide_running_in_windows: None,
                    scope: ConfigScope::User,
                    transport: Transport::Http,
                    command: None,
                    args: Vec::new(),
                    env: std::collections::BTreeMap::new(),
                    url: Some("https://example.test/mcp".to_string()),
                    headers: std::collections::BTreeMap::new(),
                    headers_helper: None,
                    oauth: None,
                    ide_name: None,
                    auth_token: None,
                    id: None,
                    plugin_source: None,
                },
                reconnect_attempt: Some(2),
                max_reconnect_attempts: Some(5),
            },
        )];

        let infos = server_infos_from_server_snapshots(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &runtime_servers,
            &[],
        );
        let docs = infos.iter().find(|server| server.name == "docs").unwrap();
        assert_eq!(
            docs.official_client_type(),
            McpServerConnectionType::Pending
        );
        assert_eq!(docs.reconnect_attempt, Some(2));
        assert_eq!(docs.max_reconnect_attempts, Some(5));
    }

    #[test]
    fn mcp_settings_marks_connected_http_server_with_tools_authenticated_like_official() {
        let runtime_servers = vec![McpServerConnection::connected_for_test(
            "docs",
            ScopedMcpServerConfig {
                name: None,
                ide_running_in_windows: None,
                scope: ConfigScope::User,
                transport: Transport::Http,
                command: None,
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
                url: Some("https://example.test/mcp".to_string()),
                headers: std::collections::BTreeMap::new(),
                headers_helper: None,
                oauth: None,
                ide_name: None,
                auth_token: None,
                id: None,
                plugin_source: None,
            },
        )];
        // CC `MCPSettings.tsx:80`: connected with tools in `mcp.tools`.
        let state = crate::state::app_state_store::McpState {
            clients: runtime_servers,
            tools: vec![crate::services::mcp::client::mcp_tool_for_test(
                "docs",
                serde_json::json!({"name": "lookup", "inputSchema": {"type": "object"}}),
            )],
            ..Default::default()
        };

        let infos = server_infos_from_mcp_state(
            &GlobalConfig::default(),
            &ProjectConfig::default(),
            &state,
            &[],
        );
        let docs = infos.iter().find(|server| server.name == "docs").unwrap();
        assert_eq!(docs.transport, Transport::Http);
        assert_eq!(docs.is_authenticated, Some(true));
    }
}
