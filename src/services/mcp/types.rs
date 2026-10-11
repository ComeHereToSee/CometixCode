//! Maps to: CC `services/mcp/types.ts`.

use crate::utils::zod::{self, Schema};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Maps to: CC `ConfigScope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConfigScope {
    Local,
    User,
    Project,
    Dynamic,
    Enterprise,
    ClaudeAi,
    Managed,
}

impl ConfigScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::User => "user",
            Self::Project => "project",
            Self::Dynamic => "dynamic",
            Self::Enterprise => "enterprise",
            Self::ClaudeAi => "claudeai",
            Self::Managed => "managed",
        }
    }
}

/// Maps to: CC `Transport` plus the `claudeai-proxy` config discriminator used
/// by `MCPSettings.tsx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Sse,
    SseIde,
    Http,
    Ws,
    WsIde,
    Sdk,
    ClaudeAiProxy,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse => "sse",
            Self::SseIde => "sse-ide",
            Self::Http => "http",
            Self::Ws => "ws",
            Self::WsIde => "ws-ide",
            Self::Sdk => "sdk",
            Self::ClaudeAiProxy => "claudeai-proxy",
        }
    }
}

/// Maps to: CC `McpServerConfig` transport discriminator.
pub fn transport_from_config(config: &crate::utils::config::McpServerConfig) -> Transport {
    match config.server_type.as_deref() {
        Some("stdio") => Transport::Stdio,
        Some("sse") => Transport::Sse,
        Some("sse-ide") => Transport::SseIde,
        Some("http") => Transport::Http,
        Some("ws") => Transport::Ws,
        Some("ws-ide") => Transport::WsIde,
        Some("sdk") => Transport::Sdk,
        Some("claudeai-proxy") => Transport::ClaudeAiProxy,
        // Older configs inferred remote streamable HTTP from URL presence.
        None if config.url.is_some() => Transport::Http,
        None => Transport::Stdio,
        Some(_) => Transport::Stdio,
    }
}

/// Maps to: CC `ScopedMcpServerConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedMcpServerConfig {
    pub scope: ConfigScope,
    pub transport: Transport,
    /// Maps to: CC `McpSdkServerConfigSchema.name`, preserved by the intersection.
    pub name: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub headers_helper: Option<String>,
    pub oauth: Option<serde_json::Value>,
    pub ide_name: Option<String>,
    /// Maps to: CC `McpSSEIDEServerConfigSchema` / `McpWebSocketIDEServerConfigSchema`.
    pub ide_running_in_windows: Option<bool>,
    pub auth_token: Option<String>,
    pub id: Option<String>,
    /// Maps to: CC `ScopedMcpServerConfig.pluginSource`.
    pub plugin_source: Option<String>,
}

impl ScopedMcpServerConfig {
    pub fn from_config(scope: ConfigScope, config: &crate::utils::config::McpServerConfig) -> Self {
        Self {
            scope,
            transport: transport_from_config(config),
            name: config.name.clone(),
            command: config.command.clone(),
            args: config.args.clone().unwrap_or_default(),
            env: config.env.clone().unwrap_or_default().into_iter().collect(),
            url: config.url.clone(),
            headers: config
                .headers
                .clone()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            headers_helper: config.headers_helper.clone(),
            oauth: config.oauth.clone(),
            ide_name: config.ide_name.clone(),
            ide_running_in_windows: config.ide_running_in_windows,
            auth_token: config.auth_token.clone(),
            id: config.id.clone(),
            plugin_source: None,
        }
    }
}

/// Maps to: CC `services/mcp/types.ts#ServerResource`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerResource {
    pub server: String,
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Maps to: CC `MCPServerConnection['type']` — the discriminant of the
/// `services/mcp/types.ts:221` union, hoisted into a standalone enum because
/// Rust models the union as one struct plus this tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpServerConnectionType {
    Connected,
    Failed,
    NeedsAuth,
    Pending,
    Disabled,
}

impl McpServerConnectionType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::NeedsAuth => "needs-auth",
            Self::Pending => "pending",
            Self::Disabled => "disabled",
        }
    }
}

/// Maps to: CC `MCPServerConnection` (`services/mcp/types.ts:221-226`) flattened
/// from a 5-way union into one struct plus the `status` tag. Each field is
/// carried by only some of the official variants — `reconnectAttempt` /
/// `maxReconnectAttempts` by `PendingMCPServer`, `error` by `FailedMCPServer`,
/// `serverInfo.version` by `ConnectedMCPServer` — so they are all `Option` here
/// where TypeScript got them for free from the union.
///
/// The name follows CC's wire key, not a CC type: `systemInit.ts:63-66` emits
/// `{ name: client.name, status: client.type }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpClientSnapshot {
    pub name: String,
    pub status: McpServerConnectionType,
    /// Maps to: CC `PendingMCPServer.reconnectAttempt`.
    pub reconnect_attempt: Option<u32>,
    /// Maps to: CC `PendingMCPServer.maxReconnectAttempts`.
    pub max_reconnect_attempts: Option<u32>,
    /// Already-known IDE display name from the MCP config. This mirrors
    /// official sse-ide/ws-ide `config.ideName`.
    pub ide_name: Option<String>,
    pub server_version: Option<String>,
    pub error: Option<String>,
}

/// Maps to: CC `MCPServerConnection` as stored in `AppState.mcp.clients`
/// (`state/AppStateStore.ts:174`), consumed by `components/mcp/MCPSettings.tsx`.
///
/// Like CC's, the record carries no tools, prompts or resources: those live in
/// `AppState.mcp.tools` / `.commands` / `.resources`, and views pick a
/// server's share out of them (`filterToolsByServer`,
/// `commandBelongsToServer`, `resources[name]`).
///
/// **Stores less**: `ConnectedMCPServer.capabilities` — the whole
/// `ServerCapabilities` object — is reduced here to the single
/// `supports_resources` bool. CC reads that object for at least five distinct
/// questions: `capabilities.resources` (client.ts:2169, the one kept here), the
/// three `X.listChanged` gates (useManageMCPConnections.ts:618/:667/:705), and
/// `capabilities.experimental['claude/channel'…]` (channelPermissions.ts:191-192).
///
/// The reduction is not a loss of information: `peer.peer_info()` carries the
/// full `ServerCapabilities` for any live connection, and the registry keeps the
/// peer. Consumers needing more than `supports_resources` read it live — the
/// listChanged gates via `context.peer` in the rmcp handlers, the channel gates
/// via `server_info_has_experimental_capability` (client.rs:1344).
///
/// The cost is that "does this server support X" now has two answer paths, and
/// code holding only a snapshot cannot ask the second kind. That is exactly what
/// made the listChanged gates look unimplementable until #36 checked where the
/// data actually was.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServerSnapshot {
    /// Native identity of CC MCPServerConnection.client, independent of config equality.
    pub connection_id: Option<u64>,
    pub client: McpClientSnapshot,
    /// Maps to: CC `MCPServerConnection.config` carried in AppState clients.
    pub config: Option<ScopedMcpServerConfig>,
    /// Maps to: CC `!!client.capabilities?.resources`. This is capability
    /// state, not a projection of the current resource list: a capable server
    /// with zero resources must still expose List/ReadMcpResourceTool.
    pub supports_resources: bool,
}

impl McpServerSnapshot {
    /// CC `client.name`.
    pub fn name(&self) -> &str {
        &self.client.name
    }

    /// CC `client.type`.
    pub fn connection_type(&self) -> McpServerConnectionType {
        self.client.status
    }
}

/// Maps to: CC `services/mcp/types.ts:28-35#McpStdioServerConfigSchema`.
pub fn mcp_stdio_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("stdio")).optional()),
            (
                "command",
                zod::string().min_with_message(1, "Command cannot be empty"),
            ),
            (
                "args",
                zod::array(zod::string()).default(serde_json::json!([])),
            ),
            ("env", zod::record(zod::string()).optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:41-41#McpXaaConfigSchema`.
fn mcp_xaa_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| zod::boolean())
}

/// Maps to: CC `services/mcp/types.ts:43-56#McpOAuthConfigSchema`.
fn mcp_oauth_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("clientId", zod::string().optional()),
            ("callbackPort", zod::number().int().positive().optional()),
            (
                "authServerMetadataUrl",
                zod::string()
                    .url()
                    .starts_with_message("https://", "authServerMetadataUrl must use https://")
                    .optional(),
            ),
            ("xaa", mcp_xaa_config_schema().clone().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:58-66#McpSSEServerConfigSchema`.
pub fn mcp_sse_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("sse"))),
            ("url", zod::string()),
            ("headers", zod::record(zod::string()).optional()),
            ("headersHelper", zod::string().optional()),
            ("oauth", mcp_oauth_config_schema().clone().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:69-76#McpSSEIDEServerConfigSchema`.
pub fn mcp_sseide_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("sse-ide"))),
            ("url", zod::string()),
            ("ideName", zod::string()),
            ("ideRunningInWindows", zod::boolean().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:79-87#McpWebSocketIDEServerConfigSchema`.
pub fn mcp_web_socket_ide_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("ws-ide"))),
            ("url", zod::string()),
            ("ideName", zod::string()),
            ("authToken", zod::string().optional()),
            ("ideRunningInWindows", zod::boolean().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:89-97#McpHTTPServerConfigSchema`.
pub fn mcp_http_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("http"))),
            ("url", zod::string()),
            ("headers", zod::record(zod::string()).optional()),
            ("headersHelper", zod::string().optional()),
            ("oauth", mcp_oauth_config_schema().clone().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:99-106#McpWebSocketServerConfigSchema`.
pub fn mcp_web_socket_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("ws"))),
            ("url", zod::string()),
            ("headers", zod::record(zod::string()).optional()),
            ("headersHelper", zod::string().optional()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:108-113#McpSdkServerConfigSchema`.
pub fn mcp_sdk_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("sdk"))),
            ("name", zod::string()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:116-122#McpClaudeAIProxyServerConfigSchema`.
pub fn mcp_claude_ai_proxy_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![
            ("type", zod::literal(serde_json::json!("claudeai-proxy"))),
            ("url", zod::string()),
            ("id", zod::string()),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:124-135#McpServerConfigSchema`.
pub fn mcp_server_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::union(vec![
            mcp_stdio_server_config_schema().clone(),
            mcp_sse_server_config_schema().clone(),
            mcp_sseide_server_config_schema().clone(),
            mcp_web_socket_ide_server_config_schema().clone(),
            mcp_http_server_config_schema().clone(),
            mcp_web_socket_server_config_schema().clone(),
            mcp_sdk_server_config_schema().clone(),
            mcp_claude_ai_proxy_server_config_schema().clone(),
        ])
    })
}

/// Maps to: CC `services/mcp/types.ts:171-175#McpJsonConfigSchema`.
pub fn mcp_json_config_schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| {
        zod::object(vec![(
            "mcpServers",
            zod::record(mcp_server_config_schema().clone()),
        )])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_type_strings_match_official_discriminators() {
        assert_eq!(ConfigScope::Project.as_str(), "project");
        assert_eq!(Transport::Stdio.as_str(), "stdio");
        assert_eq!(Transport::ClaudeAiProxy.as_str(), "claudeai-proxy");
        assert_eq!(McpServerConnectionType::NeedsAuth.as_str(), "needs-auth");
    }

    /// Maps to: CC `services/mcp/types.ts:229`
    /// `ServerResource = Resource & { server: string }`.
    ///
    /// This replaces a test that exercised `from_runtime_snapshot` /
    /// `to_client_snapshot`, the pair that used to convert between this type
    /// and a `server`-less twin. CC never had that twin — the only shape
    /// without `server` is the SDK's own `Resource`, which
    /// `fetchResourcesForClient` (`client.ts:2017-2020`) stamps on the way out
    /// — so both functions and the twin are gone, and there is nothing left to
    /// convert. What remains worth pinning is that `server` is carried, not
    /// derived from the map key it happens to be filed under.
    #[test]
    fn server_resource_carries_its_owning_server_like_official() {
        let resource = ServerResource {
            server: "memory".to_string(),
            uri: "mem://note".to_string(),
            name: "note".to_string(),
            description: Some("desc".to_string()),
            mime_type: Some("text/plain".to_string()),
        };

        assert_eq!(resource.server, "memory");
        assert_eq!(resource.uri, "mem://note");
    }

    #[test]
    fn scoped_mcp_server_config_projects_current_config_shape() {
        let config = crate::utils::config::McpServerConfig {
            command: Some("node".to_string()),
            args: Some(vec!["server.js".to_string()]),
            ..crate::utils::config::McpServerConfig::default()
        };
        let scoped = ScopedMcpServerConfig::from_config(ConfigScope::User, &config);
        assert_eq!(scoped.scope, ConfigScope::User);
        assert_eq!(scoped.transport, Transport::Stdio);
        assert_eq!(scoped.command.as_deref(), Some("node"));
        assert_eq!(scoped.args, vec!["server.js"]);
        assert!(scoped.env.is_empty());
    }
    #[test]
    fn mcp_config_schemas_match_official_bun_oracle() {
        // Actual Bun direct imports of CC utils/plugins/schemas.ts:162-1390 and
        // services/mcp/types.ts:29-136. Check parsed values AND complete ordered
        // Zod issue trees: defaults, transforms, unknown keys, paths and copy.
        // Oracle script: research/proof/plugin-schemas-0914/oracle.ts.
        let cases: serde_json::Value = serde_json::from_str(r###"[{"schema":"McpServerConfigSchema","input":{},"issues":[{"code":"invalid_union","errors":[[{"expected":"string","code":"invalid_type","path":["command"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sse"],"path":["type"],"message":"Invalid input: expected \"sse\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sse-ide"],"path":["type"],"message":"Invalid input: expected \"sse-ide\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws-ide"],"path":["type"],"message":"Invalid input: expected \"ws-ide\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["http"],"path":["type"],"message":"Invalid input: expected \"http\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws"],"path":["type"],"message":"Invalid input: expected \"ws\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sdk"],"path":["type"],"message":"Invalid input: expected \"sdk\""},{"expected":"string","code":"invalid_type","path":["name"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["claudeai-proxy"],"path":["type"],"message":"Invalid input: expected \"claudeai-proxy\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["id"],"message":"Invalid input: expected string, received undefined"}]],"path":[],"message":"Invalid input"}]},{"schema":"McpServerConfigSchema","input":{"command":"echo"},"data":{"command":"echo","args":[]}},{"schema":"McpServerConfigSchema","input":{"command":""},"issues":[{"code":"invalid_union","errors":[[{"origin":"string","code":"too_small","minimum":1,"inclusive":true,"path":["command"],"message":"Command cannot be empty"}],[{"code":"invalid_value","values":["sse"],"path":["type"],"message":"Invalid input: expected \"sse\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sse-ide"],"path":["type"],"message":"Invalid input: expected \"sse-ide\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws-ide"],"path":["type"],"message":"Invalid input: expected \"ws-ide\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["http"],"path":["type"],"message":"Invalid input: expected \"http\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws"],"path":["type"],"message":"Invalid input: expected \"ws\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sdk"],"path":["type"],"message":"Invalid input: expected \"sdk\""},{"expected":"string","code":"invalid_type","path":["name"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["claudeai-proxy"],"path":["type"],"message":"Invalid input: expected \"claudeai-proxy\""},{"expected":"string","code":"invalid_type","path":["url"],"message":"Invalid input: expected string, received undefined"},{"expected":"string","code":"invalid_type","path":["id"],"message":"Invalid input: expected string, received undefined"}]],"path":[],"message":"Invalid input"}]},{"schema":"McpServerConfigSchema","input":{"command":"echo","extra":1},"data":{"command":"echo","args":[]}},{"schema":"McpServerConfigSchema","input":{"type":"sse","url":""},"data":{"type":"sse","url":""}},{"schema":"McpServerConfigSchema","input":{"type":"sse-ide","url":"u","ideName":"i"},"data":{"type":"sse-ide","url":"u","ideName":"i"}},{"schema":"McpServerConfigSchema","input":{"type":"ws-ide","url":"u","ideName":"i","authToken":"a"},"data":{"type":"ws-ide","url":"u","ideName":"i","authToken":"a"}},{"schema":"McpServerConfigSchema","input":{"type":"http","url":"u","oauth":{"authServerMetadataUrl":"http://a"}},"issues":[{"code":"invalid_union","errors":[[{"code":"invalid_value","values":["stdio"],"path":["type"],"message":"Invalid input: expected \"stdio\""},{"expected":"string","code":"invalid_type","path":["command"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sse"],"path":["type"],"message":"Invalid input: expected \"sse\""},{"origin":"string","code":"invalid_format","format":"starts_with","prefix":"https://","path":["oauth","authServerMetadataUrl"],"message":"authServerMetadataUrl must use https://"}],[{"code":"invalid_value","values":["sse-ide"],"path":["type"],"message":"Invalid input: expected \"sse-ide\""},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws-ide"],"path":["type"],"message":"Invalid input: expected \"ws-ide\""},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"origin":"string","code":"invalid_format","format":"starts_with","prefix":"https://","path":["oauth","authServerMetadataUrl"],"message":"authServerMetadataUrl must use https://"}],[{"code":"invalid_value","values":["ws"],"path":["type"],"message":"Invalid input: expected \"ws\""}],[{"code":"invalid_value","values":["sdk"],"path":["type"],"message":"Invalid input: expected \"sdk\""},{"expected":"string","code":"invalid_type","path":["name"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["claudeai-proxy"],"path":["type"],"message":"Invalid input: expected \"claudeai-proxy\""},{"expected":"string","code":"invalid_type","path":["id"],"message":"Invalid input: expected string, received undefined"}]],"path":[],"message":"Invalid input"}]},{"schema":"McpServerConfigSchema","input":{"type":"http","url":"u","oauth":{"authServerMetadataUrl":"invalid","callbackPort":0}},"issues":[{"code":"invalid_union","errors":[[{"code":"invalid_value","values":["stdio"],"path":["type"],"message":"Invalid input: expected \"stdio\""},{"expected":"string","code":"invalid_type","path":["command"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["sse"],"path":["type"],"message":"Invalid input: expected \"sse\""},{"origin":"number","code":"too_small","minimum":0,"inclusive":false,"path":["oauth","callbackPort"],"message":"Too small: expected number to be >0"},{"code":"invalid_format","format":"url","path":["oauth","authServerMetadataUrl"],"message":"Invalid URL"},{"origin":"string","code":"invalid_format","format":"starts_with","prefix":"https://","path":["oauth","authServerMetadataUrl"],"message":"authServerMetadataUrl must use https://"}],[{"code":"invalid_value","values":["sse-ide"],"path":["type"],"message":"Invalid input: expected \"sse-ide\""},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["ws-ide"],"path":["type"],"message":"Invalid input: expected \"ws-ide\""},{"expected":"string","code":"invalid_type","path":["ideName"],"message":"Invalid input: expected string, received undefined"}],[{"origin":"number","code":"too_small","minimum":0,"inclusive":false,"path":["oauth","callbackPort"],"message":"Too small: expected number to be >0"},{"code":"invalid_format","format":"url","path":["oauth","authServerMetadataUrl"],"message":"Invalid URL"},{"origin":"string","code":"invalid_format","format":"starts_with","prefix":"https://","path":["oauth","authServerMetadataUrl"],"message":"authServerMetadataUrl must use https://"}],[{"code":"invalid_value","values":["ws"],"path":["type"],"message":"Invalid input: expected \"ws\""}],[{"code":"invalid_value","values":["sdk"],"path":["type"],"message":"Invalid input: expected \"sdk\""},{"expected":"string","code":"invalid_type","path":["name"],"message":"Invalid input: expected string, received undefined"}],[{"code":"invalid_value","values":["claudeai-proxy"],"path":["type"],"message":"Invalid input: expected \"claudeai-proxy\""},{"expected":"string","code":"invalid_type","path":["id"],"message":"Invalid input: expected string, received undefined"}]],"path":[],"message":"Invalid input"}]},{"schema":"McpServerConfigSchema","input":{"type":"ws","url":"u"},"data":{"type":"ws","url":"u"}},{"schema":"McpServerConfigSchema","input":{"type":"sdk","name":"n"},"data":{"type":"sdk","name":"n"}},{"schema":"McpServerConfigSchema","input":{"type":"claudeai-proxy","url":"u","id":"i"},"data":{"type":"claudeai-proxy","url":"u","id":"i"}}]"###).unwrap();
        for case in cases.as_array().unwrap() {
            let schema = match case["schema"].as_str().unwrap() {
                "McpServerConfigSchema" => mcp_server_config_schema(),
                other => panic!("unexpected oracle schema: {other}"),
            };
            match crate::utils::zod::safe_parse(schema, &case["input"]) {
                Ok(data) => {
                    assert!(case.get("data").is_some(), "expected error: {case}");
                    assert_eq!(data, case["data"], "{case}");
                }
                Err(error) => {
                    let issues: Vec<_> = error.issues.iter().map(|issue| issue.to_json()).collect();
                    assert_eq!(serde_json::json!(issues), case["issues"], "{case}");
                }
            }
        }
    }
    #[test]
    fn mcp_sdk_name_and_ide_flag_roundtrip_matches_official_source() {
        // Actual McpServerConfigSchema plus hashMcpConfig/areMcpConfigsEqual:
        // services/mcp/types.ts:69–87,108–113,163–169; utils.ts:157–168;
        // client.ts:1710–1723. Oracle imports the real schema and source bodies.
        let cases: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/oracles/mcp-config-fields-0915/oracle.json"
        ))).unwrap();
        for case in cases.as_array().unwrap() {
            let validated = zod::safe_parse(mcp_server_config_schema(), &case["input"]).unwrap();
            assert_eq!(validated, case["parsed"]);
            let raw: crate::utils::config::McpServerConfig =
                serde_json::from_value(validated.clone()).unwrap();
            let serialized = serde_json::to_value(&raw).unwrap();
            // This regression concerns the two previously lost fields; existing
            // optional fields in the broad native DTO are not a new wire format.
            for field in ["name", "ideRunningInWindows"] {
                assert_eq!(
                    serialized.get(field),
                    validated.get(field),
                    "{case}: {field}"
                );
            }
            let restored: crate::utils::config::McpServerConfig =
                serde_json::from_value(serialized).unwrap();
            let scoped = ScopedMcpServerConfig::from_config(ConfigScope::Dynamic, &restored);
            assert_eq!(scoped.name.as_deref(), validated["name"].as_str());
            assert_eq!(
                scoped.ide_running_in_windows,
                validated["ideRunningInWindows"].as_bool()
            );
            if scoped.transport == Transport::Sdk {
                assert!(
                    scoped.id.is_none(),
                    "SDK name is not claude.ai connector id"
                );
            }
            assert_eq!(
                crate::services::mcp::utils::hash_mcp_config(&scoped),
                case["hash"].as_str().unwrap()
            );
            let mut same = scoped.clone();
            same.scope = ConfigScope::User;
            assert_eq!(
                crate::services::mcp::client::are_mcp_configs_equal(&scoped, &same),
                case["sameAfterScope"].as_bool().unwrap()
            );
            let mut changed = scoped.clone();
            if changed.transport == Transport::Sdk {
                changed.name = Some("changed".into());
            } else {
                changed.ide_running_in_windows = Some(changed.ide_running_in_windows != Some(true));
            }
            assert_eq!(
                crate::services::mcp::utils::hash_mcp_config(&changed),
                case["changedHash"].as_str().unwrap()
            );
            assert_eq!(
                crate::services::mcp::client::are_mcp_configs_equal(&scoped, &changed),
                case["sameAfterChange"].as_bool().unwrap()
            );
        }
    }
}
