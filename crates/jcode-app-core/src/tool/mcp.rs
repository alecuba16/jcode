//! MCP management tool - connect, disconnect, list, reload MCP servers

use crate::mcp::{ContentBlock, McpManager, McpServerConfig, dispatch_name};
use crate::tool::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Deserialize)]
struct McpSearchInput {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    query: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpSearchResult {
    name: String,
    server: String,
    tool: String,
    description: String,
    input_schema: Value,
}

/// Fixed MCP discovery surface used when individual server definitions are deferred.
pub struct McpSearchTool {
    manager: Arc<RwLock<McpManager>>,
}

impl McpSearchTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for McpSearchTool {
    fn name(&self) -> &str {
        "mcp_search"
    }

    fn description(&self) -> &str {
        "Search available MCP tools by server, name, or description. Returns callable names and input schemas."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "Optional exact MCP server name."
                },
                "query": {
                    "type": "string",
                    "description": "Optional case-insensitive name or description search."
                }
            }
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: McpSearchInput = serde_json::from_value(input)?;
        let server_filter = params
            .server
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let query = params
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let manager = self.manager.read().await;
        let catalog = manager.searchable_tools().await;
        drop(manager);

        let matches: Vec<McpSearchResult> = catalog
            .into_iter()
            .filter_map(|(server, tool)| {
                if server_filter.is_some_and(|wanted| wanted != server) {
                    return None;
                }
                let name = dispatch_name(&server, &tool.name);
                if !super::session_mcp_dispatch_is_allowed(&ctx.session_id, &name, "mcp_search") {
                    return None;
                }
                if let Some(query) = &query {
                    let description = tool.description.as_deref().unwrap_or_default();
                    if !name.to_ascii_lowercase().contains(query)
                        && !server.to_ascii_lowercase().contains(query)
                        && !tool.name.to_ascii_lowercase().contains(query)
                        && !description.to_ascii_lowercase().contains(query)
                    {
                        return None;
                    }
                }
                Some(McpSearchResult {
                    name,
                    server,
                    tool: tool.name,
                    description: tool.description.unwrap_or_else(|| "MCP tool".to_string()),
                    input_schema: tool.input_schema,
                })
            })
            .collect();

        Ok(ToolOutput::new(serde_json::to_string_pretty(&matches)?)
            .with_title(format!("MCP tools ({})", matches.len())))
    }
}

#[derive(Debug, Deserialize)]
struct McpCallInput {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Value,
}

/// Fixed MCP execution surface used when individual server definitions are deferred.
pub struct McpCallTool {
    manager: Arc<RwLock<McpManager>>,
}

impl McpCallTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn name(&self) -> &str {
        "mcp_call"
    }

    fn description(&self) -> &str {
        "Call an MCP server tool discovered with mcp_search."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": {"type": "string", "description": "MCP server name."},
                "tool": {"type": "string", "description": "Raw MCP tool name."},
                "arguments": {
                    "type": "object",
                    "description": "Arguments matching the input schema returned by mcp_search."
                }
            },
            "required": ["server", "tool", "arguments"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let mut params: McpCallInput = serde_json::from_value(input)?;
        let dispatched_name = dispatch_name(&params.server, &params.tool);
        if !super::session_mcp_dispatch_is_allowed(&ctx.session_id, &dispatched_name, "mcp_call") {
            anyhow::bail!("MCP tool '{}' is not allowed", dispatched_name);
        }
        if params.arguments.is_null() {
            params.arguments = Value::Object(serde_json::Map::new());
        }

        let manager = self.manager.read().await;
        let result = manager
            .call_tool(&params.server, &params.tool, params.arguments)
            .await?;
        drop(manager);

        let mut output_parts = Vec::new();
        for block in result.content {
            match block {
                ContentBlock::Text { text } => output_parts.push(text),
                ContentBlock::Image { data, mime_type } => {
                    output_parts.push(format!("[Image: {} ({} bytes)]", mime_type, data.len()));
                }
                ContentBlock::Resource { resource } => {
                    if let Some(text) = resource.text {
                        output_parts.push(text);
                    } else if let Some(blob) = resource.blob {
                        output_parts.push(format!(
                            "[Resource: {} ({} bytes)]",
                            resource.uri,
                            blob.len()
                        ));
                    } else {
                        output_parts.push(format!("[Resource: {}]", resource.uri));
                    }
                }
            }
        }
        let output = output_parts.join("\n");
        let title = format!("mcp:{}:{}", params.server, params.tool);
        if result.is_error {
            Ok(ToolOutput::new(format!("Error: {}", output)).with_title(title))
        } else {
            Ok(ToolOutput::new(output).with_title(title))
        }
    }
}

#[derive(Debug, Deserialize)]
struct McpToolInput {
    action: String,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
}

pub struct McpManagementTool {
    manager: Arc<RwLock<McpManager>>,
    registry: Option<crate::tool::WeakRegistry>,
    /// Optional channel to emit ServerEvent::McpStatus after a toggle so the
    /// remote/client UI refreshes immediately. Set by the server path; the
    /// local TUI path manages its own mcp_server_names from the manager.
    event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::ServerEvent>>,
}

impl McpManagementTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self {
            manager,
            registry: None,
            event_tx: None,
        }
    }

    pub fn with_registry(mut self, registry: crate::tool::Registry) -> Self {
        self.registry = Some(registry.downgrade());
        self
    }

    /// Set the event channel so the tool can emit McpStatus updates after a
    /// toggle. Used by the server/remote path; the local TUI path leaves this
    /// as None and refreshes from the manager directly.
    pub fn with_event_tx(
        mut self,
        event_tx: tokio::sync::mpsc::UnboundedSender<crate::protocol::ServerEvent>,
    ) -> Self {
        self.event_tx = Some(event_tx);
        self
    }

    /// Like [`Self::with_event_tx`] but accepts an Option, so callers that
    /// already hold `Option<...>` (e.g. the server's register_mcp_tools path)
    /// can pass it through without an extra match.
    pub fn with_event_tx_optional(
        mut self,
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::protocol::ServerEvent>>,
    ) -> Self {
        self.event_tx = event_tx;
        self
    }

    /// Emit an McpStatus event reflecting the currently connected servers and
    /// their tool counts, so the UI refreshes after a toggle.
    async fn emit_mcp_status(&self) {
        let Some(tx) = &self.event_tx else {
            return;
        };
        let manager = self.manager.read().await;
        let servers = manager.connected_servers().await;
        let all_tools = manager.all_tools().await;
        let status: Vec<String> = servers
            .iter()
            .map(|name| {
                let count = all_tools.iter().filter(|(s, _)| s == name).count();
                format!("{}:{}", name, count)
            })
            .collect();
        let _ = tx.send(crate::protocol::ServerEvent::McpStatus { servers: status });
    }
}

#[async_trait]
impl Tool for McpManagementTool {
    fn name(&self) -> &str {
        "mcp"
    }

    fn description(&self) -> &str {
        "Manage MCP (Model Context Protocol) servers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "intent": super::intent_schema_property(),
                "action": {
                    "type": "string",
                    "enum": ["list", "connect", "disconnect", "reload", "enable", "disable"],
                    "description": "Action. 'enable'/'disable' toggles whether a server's tools are injected into the prompt (persists to config)."
                },
                "server": {
                    "type": "string",
                    "description": "Server name."
                },
                "command": {
                    "type": "string",
                    "description": "Server command."
                },
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Command args."
                },
                "env": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "description": "Server env."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: ToolContext) -> Result<ToolOutput> {
        let params: McpToolInput = serde_json::from_value(input)?;
        let started = std::time::Instant::now();
        let action = params.action.clone();
        let server = params.server.clone().unwrap_or_else(|| "none".to_string());
        crate::logging::event_info(
            "MCP_LIFECYCLE",
            vec![
                ("phase", "management_start".to_string()),
                ("action", action.clone()),
                ("server", server.clone()),
                ("session_id", ctx.session_id.clone()),
                ("tool_call_id", ctx.tool_call_id.clone()),
            ],
        );

        let result = match params.action.as_str() {
            "list" => self.list_servers().await,
            "connect" => self.connect_server(params, &ctx.session_id).await,
            "disconnect" => self.disconnect_server(params).await,
            "reload" => self.reload_config(&ctx.session_id).await,
            "enable" => self.toggle_server(params, true, &ctx.session_id).await,
            "disable" => self.toggle_server(params, false, &ctx.session_id).await,
            _ => Ok(ToolOutput::new(format!(
                "Unknown action: {}. Use 'list', 'connect', 'disconnect', 'reload', 'enable', or 'disable'.",
                params.action
            ))),
        };

        match &result {
            Ok(_) => crate::logging::event_info(
                "MCP_LIFECYCLE",
                vec![
                    ("phase", "management_done".to_string()),
                    ("action", action),
                    ("server", server),
                    ("session_id", ctx.session_id),
                    ("tool_call_id", ctx.tool_call_id),
                    ("status", "ok".to_string()),
                    ("elapsed_ms", started.elapsed().as_millis().to_string()),
                ],
            ),
            Err(error) => crate::logging::event_warn(
                "MCP_LIFECYCLE",
                vec![
                    ("phase", "management_done".to_string()),
                    ("action", action),
                    ("server", server),
                    ("session_id", ctx.session_id),
                    ("tool_call_id", ctx.tool_call_id),
                    ("status", "error".to_string()),
                    ("error", error.to_string()),
                    ("elapsed_ms", started.elapsed().as_millis().to_string()),
                ],
            ),
        }

        result
    }
}

impl McpManagementTool {
    async fn list_servers(&self) -> Result<ToolOutput> {
        let manager = self.manager.read().await;
        let servers = manager.connected_servers().await;
        let all_tools = manager.all_tools().await;
        // Configured-but-not-connected servers, including disabled ones
        // (issue #436), so the full config state is visible.
        let mut configured: Vec<(String, bool)> = manager
            .config()
            .servers
            .iter()
            .filter(|(name, _)| !servers.contains(name))
            .map(|(name, cfg)| (name.clone(), cfg.is_enabled()))
            .collect();
        configured.sort();

        if servers.is_empty() && configured.is_empty() {
            return Ok(ToolOutput::new(
                "No MCP servers connected.\n\n\
                To connect a server, use:\n\
                {\"action\": \"connect\", \"server\": \"name\", \"command\": \"/path/to/server\", \"args\": []}\n\n\
                Or add servers to ~/.jcode/mcp.json or .jcode/mcp.json and use {\"action\": \"reload\"}.\n\
                .claude/mcp.json is also supported for compatibility."
            ).with_title("MCP: No servers"));
        }

        let mut output = String::new();
        output.push_str(&format!("Connected MCP servers: {}\n\n", servers.len()));

        for server in &servers {
            output.push_str(&format!("## {}\n", server));
            let server_tools: Vec<_> = all_tools.iter().filter(|(s, _)| s == server).collect();

            if server_tools.is_empty() {
                output.push_str("  (no tools)\n");
            } else {
                for (_, tool) in server_tools {
                    output.push_str(&format!(
                        "  - {}: {}\n",
                        crate::mcp::dispatch_name(server, &tool.name),
                        tool.description.as_deref().unwrap_or("(no description)")
                    ));
                }
            }
            output.push('\n');
        }

        if !configured.is_empty() {
            output.push_str("Configured but not connected:\n");
            for (name, enabled) in &configured {
                if *enabled {
                    output.push_str(&format!(
                        "  - {} (enabled; connect with {{\"action\": \"connect\", \"server\": \"{}\"}})\n",
                        name, name
                    ));
                } else {
                    output.push_str(&format!(
                        "  - {} (disabled in config; connect on demand with {{\"action\": \"connect\", \"server\": \"{}\"}})\n",
                        name, name
                    ));
                }
            }
        }

        Ok(ToolOutput::new(output).with_title("MCP: Server list"))
    }

    async fn connect_server(&self, params: McpToolInput, session_id: &str) -> Result<ToolOutput> {
        let server_name = params
            .server
            .ok_or_else(|| anyhow::anyhow!("'server' is required for connect action"))?;

        // With an explicit command this is an ad-hoc connect. Without one, fall
        // back to the configured server of that name, which also lets disabled
        // configured servers be connected on demand, session-scoped, without
        // rewriting config (issue #436).
        let config = if let Some(command) = params.command {
            McpServerConfig {
                command,
                args: params.args.unwrap_or_default(),
                env: params.env.unwrap_or_default(),
                shared: true,
                transport: None,
                url: None,
                headers: std::collections::HashMap::new(),
                enabled: None,
                disabled: None,
                timeout_secs: None,
            }
        } else {
            let manager = self.manager.read().await;
            let configured = manager.config().servers.get(&server_name).cloned();
            drop(manager);
            configured.ok_or_else(|| {
                anyhow::anyhow!(
                    "'command' is required for connect action ('{}' is not in the MCP config)",
                    server_name
                )
            })?
        };

        let manager = self.manager.read().await;

        // Check if already connected
        let connected = manager.connected_servers().await;
        if connected.contains(&server_name) {
            return Ok(ToolOutput::new(format!(
                "Server '{}' is already connected. Use 'disconnect' first to reconnect.",
                server_name
            ))
            .with_title("MCP: Already connected"));
        }
        drop(manager);

        // Connect
        let manager = self.manager.read().await;
        match manager.connect(&server_name, &config).await {
            Ok(()) => {
                let tools = manager.all_tools().await;
                let server_tools: Vec<_> =
                    tools.iter().filter(|(s, _)| s == &server_name).collect();

                let mut output = format!(
                    "Connected to MCP server '{}'\n\nAvailable tools ({}):\n",
                    server_name,
                    server_tools.len()
                );
                for (_, tool) in &server_tools {
                    output.push_str(&format!(
                        "  - {}: {}\n",
                        crate::mcp::dispatch_name(&server_name, &tool.name),
                        tool.description.as_deref().unwrap_or("(no description)")
                    ));
                }
                drop(manager);

                // Register the new tools in the registry
                if let Some(registry) = self
                    .registry
                    .as_ref()
                    .and_then(|registry| registry.upgrade())
                {
                    let mcp_tools = crate::mcp::create_mcp_tools(Arc::clone(&self.manager)).await;
                    let server_prefix = crate::mcp::dispatch_name(&server_name, "");
                    for (name, tool) in mcp_tools {
                        if name.starts_with(&server_prefix) {
                            registry.register(name, tool).await;
                        }
                    }
                }

                Ok(ToolOutput::new(output).with_title(format!("MCP: Connected {}", server_name)))
            }
            Err(e) => {
                crate::logging::event_warn(
                    "MCP_LIFECYCLE",
                    vec![
                        ("phase", "connect_failed".to_string()),
                        ("server", server_name.clone()),
                        ("session_id", session_id.to_string()),
                        ("error", e.to_string()),
                    ],
                );
                Ok(
                    ToolOutput::new(format!("Failed to connect to '{}': {}", server_name, e))
                        .with_title("MCP: Connection failed"),
                )
            }
        }
    }

    async fn disconnect_server(&self, params: McpToolInput) -> Result<ToolOutput> {
        let server_name = params
            .server
            .ok_or_else(|| anyhow::anyhow!("'server' is required for disconnect action"))?;

        let manager = self.manager.read().await;
        let connected = manager.connected_servers().await;

        if !connected.contains(&server_name) {
            return Ok(ToolOutput::new(format!(
                "Server '{}' is not connected.\n\nConnected servers: {}",
                server_name,
                if connected.is_empty() {
                    "(none)".to_string()
                } else {
                    connected.join(", ")
                }
            ))
            .with_title("MCP: Not connected"));
        }
        drop(manager);

        let manager = self.manager.read().await;
        manager.disconnect(&server_name).await?;
        drop(manager);

        // Unregister tools for this server
        if let Some(registry) = self
            .registry
            .as_ref()
            .and_then(|registry| registry.upgrade())
        {
            let removed = registry
                .unregister_prefix(&crate::mcp::dispatch_name(&server_name, ""))
                .await;
            crate::logging::event_info(
                "MCP_LIFECYCLE",
                vec![
                    ("phase", "tools_unregistered".to_string()),
                    ("server", server_name.clone()),
                    ("removed_tool_count", removed.len().to_string()),
                ],
            );
        }

        Ok(
            ToolOutput::new(format!("Disconnected from MCP server '{}'", server_name))
                .with_title(format!("MCP: Disconnected {}", server_name)),
        )
    }

    async fn reload_config(&self, session_id: &str) -> Result<ToolOutput> {
        // Load fresh config, resolved against the session's project directory
        // rather than the server process cwd (issue #420).
        let config = self.manager.read().await.load_fresh_config();

        if config.servers.is_empty() {
            // Unregister all existing MCP tools before reporting empty
            if let Some(registry) = self
                .registry
                .as_ref()
                .and_then(|registry| registry.upgrade())
            {
                registry.unregister_prefix("mcp__").await;
            }
            return Ok(ToolOutput::new(
                "No servers found in config.\n\n\
                Add servers to ~/.jcode/mcp.json (global) or .jcode/mcp.json (project):\n\
                {\n  \"servers\": {\n    \"server-name\": {\n      \"command\": \"/path/to/server\",\n      \"args\": [],\n      \"env\": {},\n      \"shared\": true\n    }\n  }\n}\n\n\
                .claude/mcp.json is also supported for compatibility."
            ).with_title("MCP: Empty config"));
        }

        // Unregister all existing MCP server tools before reload
        if let Some(registry) = self
            .registry
            .as_ref()
            .and_then(|registry| registry.upgrade())
        {
            registry.unregister_prefix("mcp__").await;
        }

        let mut manager = self.manager.write().await;
        let (successes, failures) = manager.reload().await?;

        let servers = manager.connected_servers().await;
        let all_tools = manager.all_tools().await;
        drop(manager);

        // Re-register tools from fresh connections
        if let Some(registry) = self
            .registry
            .as_ref()
            .and_then(|registry| registry.upgrade())
        {
            let mcp_tools = crate::mcp::create_mcp_tools(Arc::clone(&self.manager)).await;
            for (name, tool) in mcp_tools {
                registry.register(name, tool).await;
            }
        }

        let enabled_count = config
            .servers
            .values()
            .filter(|cfg| cfg.is_enabled())
            .count();
        let disabled_count = config.servers.len() - enabled_count;
        let mut output = format!(
            "Reloaded MCP config. Connected: {}/{}\n\n",
            successes, enabled_count
        );
        if disabled_count > 0 {
            output.push_str(&format!(
                "{} server(s) disabled in config (kept, not spawned).\n\n",
                disabled_count
            ));
        }

        // Show failures first
        if !failures.is_empty() {
            crate::logging::event_warn(
                "MCP_LIFECYCLE",
                vec![
                    ("phase", "reload_connect_failures".to_string()),
                    ("session_id", session_id.to_string()),
                    ("failure_count", failures.len().to_string()),
                    (
                        "servers",
                        failures
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                ],
            );
            output.push_str("## Connection Failures\n");
            for (name, error) in &failures {
                output.push_str(&format!("  - {}: {}\n", name, error));
            }
            output.push('\n');
        }

        for server in &servers {
            output.push_str(&format!("## {}\n", server));
            let server_tools: Vec<_> = all_tools.iter().filter(|(s, _)| s == server).collect();

            for (_, tool) in server_tools {
                output.push_str(&format!("  - {}\n", tool.name));
            }
            output.push('\n');
        }

        Ok(ToolOutput::new(output).with_title("MCP: Reloaded"))
    }

    /// Enable or disable an MCP server in config. When enabling, the server is
    /// connected and its tools registered. When disabling, the server is
    /// disconnected and its tools unregistered. The change persists to
    /// ~/.jcode/mcp.json so it survives restarts (opencode-style toggle).
    async fn toggle_server(
        &self,
        params: McpToolInput,
        enable: bool,
        session_id: &str,
    ) -> Result<ToolOutput> {
        let server_name = params
            .server
            .ok_or_else(|| anyhow::anyhow!("'server' is required for enable/disable action"))?;

        // Load current config, update the server's enabled flag, and save.
        let config = self.manager.read().await.load_fresh_config();
        let mut server_config = config.servers.get(&server_name).cloned().ok_or_else(|| {
            anyhow::anyhow!(
                "Server '{}' not found in MCP config. Add it to ~/.jcode/mcp.json first.",
                server_name
            )
        })?;

        if enable {
            server_config.disabled = Some(false);
            server_config.enabled = Some(true);
        } else {
            server_config.disabled = Some(true);
            server_config.enabled = Some(false);
        }

        // Update and save the global config file (~/.jcode/mcp.json)
        let mut saved_config = config.clone();
        saved_config
            .servers
            .insert(server_name.clone(), server_config.clone());

        let mut save_ok = true;
        if let Ok(jcode_dir) = crate::storage::jcode_dir() {
            let mcp_path = jcode_dir.join("mcp.json");
            if let Err(e) = saved_config.save_to_file(&mcp_path) {
                save_ok = false;
                crate::logging::event_warn(
                    "MCP_LIFECYCLE",
                    vec![
                        ("phase", "toggle_save_failed".to_string()),
                        ("server", server_name.clone()),
                        ("error", e.to_string()),
                    ],
                );
                // Continue anyway: the in-memory toggle still takes effect.
            }
        }

        // Refresh the manager's in-memory config so subsequent operations
        // (connect, list, connect_all) see the new enabled/disabled state
        // instead of the stale cached copy from process start.
        if save_ok {
            self.manager.write().await.reload_config();
        }

        if enable {
            // Connect the server and register its tools
            let manager = self.manager.read().await;
            let connected = manager.connected_servers().await;
            if connected.contains(&server_name) {
                drop(manager);
                self.emit_mcp_status().await;
                return Ok(ToolOutput::new(format!(
                    "Server '{}' is already connected and enabled.",
                    server_name
                ))
                .with_title(format!("MCP: {} already enabled", server_name)));
            }
            drop(manager);

            let manager = self.manager.read().await;
            match manager.connect(&server_name, &server_config).await {
                Ok(()) => {
                    let tools = manager.all_tools().await;
                    let server_tools: Vec<_> =
                        tools.iter().filter(|(s, _)| s == &server_name).collect();
                    let tool_count = server_tools.len();
                    drop(manager);

                    // Register tools in the registry
                    if let Some(registry) = self
                        .registry
                        .as_ref()
                        .and_then(|registry| registry.upgrade())
                    {
                        let mcp_tools = crate::mcp::create_mcp_tools_for_server(
                            Arc::clone(&self.manager),
                            &server_name,
                        )
                        .await;
                        let prefix = dispatch_name(&server_name, "");
                        for (name, tool) in mcp_tools {
                            if name.starts_with(&prefix) {
                                registry.register(name, tool).await;
                            }
                        }
                    }

                    crate::logging::event_info(
                        "MCP_LIFECYCLE",
                        vec![
                            ("phase", "server_enabled".to_string()),
                            ("server", server_name.clone()),
                            ("session_id", session_id.to_string()),
                            ("tool_count", tool_count.to_string()),
                        ],
                    );

                    self.emit_mcp_status().await;

                    Ok(ToolOutput::new(format!(
                        "Enabled MCP server '{}'. Connected with {} tool(s). Config saved to ~/.jcode/mcp.json.",
                        server_name, tool_count
                    ))
                    .with_title(format!("MCP: Enabled {}", server_name)))
                }
                Err(e) => {
                    drop(manager);
                    self.emit_mcp_status().await;
                    Ok(ToolOutput::new(format!(
                        "Server '{}' enabled in config but failed to connect: {}.\nUse 'reload' to retry.",
                        server_name, e
                    ))
                    .with_title(format!("MCP: {} enabled (connect failed)", server_name)))
                }
            }
        } else {
            // Disconnect the server and unregister its tools
            let manager = self.manager.read().await;
            let connected = manager.connected_servers().await;
            if connected.contains(&server_name) {
                manager.disconnect(&server_name).await?;
            }
            drop(manager);

            // Unregister tools for this server
            if let Some(registry) = self
                .registry
                .as_ref()
                .and_then(|registry| registry.upgrade())
            {
                let removed = registry
                    .unregister_prefix(&dispatch_name(&server_name, ""))
                    .await;
                crate::logging::event_info(
                    "MCP_LIFECYCLE",
                    vec![
                        ("phase", "tools_unregistered".to_string()),
                        ("server", server_name.clone()),
                        ("removed_tool_count", removed.len().to_string()),
                    ],
                );
            }

            crate::logging::event_info(
                "MCP_LIFECYCLE",
                vec![
                    ("phase", "server_disabled".to_string()),
                    ("server", server_name.clone()),
                    ("session_id", session_id.to_string()),
                ],
            );

            self.emit_mcp_status().await;

            Ok(ToolOutput::new(format!(
                "Disabled MCP server '{}'. Tools removed from prompt. Config saved to ~/.jcode/mcp.json.",
                server_name
            ))
            .with_title(format!("MCP: Disabled {}", server_name)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;
    use std::fs;
    use std::path::PathBuf;

    fn create_test_tool() -> McpManagementTool {
        // Use an explicit empty config so tests are hermetic: McpManager::new()
        // would load the developer's real ~/.jcode/mcp.json, and list output
        // now includes configured-but-not-connected servers (issue #436).
        let manager = Arc::new(RwLock::new(McpManager::with_config(
            crate::mcp::McpConfig::default(),
        )));
        McpManagementTool::new(manager)
    }

    fn create_test_context() -> ToolContext {
        ToolContext {
            session_id: "test-session".to_string(),
            message_id: "test-message".to_string(),
            tool_call_id: "test-tool-call".to_string(),
            working_dir: None,
            stdin_request_tx: None,
            graceful_shutdown_signal: None,
            execution_mode: crate::tool::ToolExecutionMode::Direct,
        }
    }

    struct LocalMcpConfigGuard {
        path: PathBuf,
        backup: Option<String>,
        created_dir: bool,
    }

    impl LocalMcpConfigGuard {
        fn new(content: &str) -> std::io::Result<Self> {
            let path = PathBuf::from(".jcode/mcp.json");
            let dir = path
                .parent()
                .ok_or_else(|| std::io::Error::other("missing parent"))?;
            let created_dir = if !dir.exists() {
                fs::create_dir_all(dir)?;
                true
            } else {
                false
            };
            let backup = if path.exists() {
                Some(fs::read_to_string(&path)?)
            } else {
                None
            };
            fs::write(&path, content)?;
            Ok(Self {
                path,
                backup,
                created_dir,
            })
        }
    }

    impl Drop for LocalMcpConfigGuard {
        fn drop(&mut self) {
            match &self.backup {
                Some(content) => {
                    let _ = fs::write(&self.path, content);
                }
                None => {
                    let _ = fs::remove_file(&self.path);
                    if self.created_dir
                        && let Some(dir) = self.path.parent()
                    {
                        let _ = fs::remove_dir(dir);
                    }
                }
            }
        }
    }

    #[test]
    fn test_tool_name() {
        let tool = create_test_tool();
        assert_eq!(tool.name(), "mcp");
    }

    #[test]
    fn test_tool_description() {
        let tool = create_test_tool();
        assert!(tool.description().contains("MCP"));
        assert!(tool.description().contains("Model Context Protocol"));
    }

    #[test]
    fn test_parameters_schema() {
        let tool = create_test_tool();
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["server"].is_object());
        assert!(schema["properties"]["command"].is_object());
    }

    #[tokio::test]
    async fn test_list_empty() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "list"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("No MCP servers connected"));
    }

    #[tokio::test]
    async fn test_list_shows_disabled_configured_server() {
        // Issue #436: disabled servers stay visible in the list with their
        // state, so users can see and enable them on demand.
        let mut config = crate::mcp::McpConfig::default();
        config.servers.insert(
            "off-server".to_string(),
            McpServerConfig {
                command: "some-bin".to_string(),
                args: vec![],
                env: HashMap::new(),
                shared: true,
                transport: None,
                url: None,
                headers: HashMap::new(),
                enabled: Some(false),
                disabled: None,
                timeout_secs: None,
            },
        );
        let manager = Arc::new(RwLock::new(McpManager::with_config(config)));
        let tool = McpManagementTool::new(manager);
        let ctx = create_test_context();

        let result = tool.execute(json!({"action": "list"}), ctx).await.unwrap();
        assert!(
            result.output.contains("off-server"),
            "disabled server must be listed: {}",
            result.output
        );
        assert!(
            result.output.contains("disabled in config"),
            "disabled state must be visible: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_connect_missing_server() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "connect", "command": "/bin/test"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("server"));
    }

    #[tokio::test]
    async fn test_connect_missing_command() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "connect", "server": "test"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command"));
    }

    #[tokio::test]
    async fn test_disconnect_not_connected() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "disconnect", "server": "nonexistent"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("not connected"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "invalid_action"});

        let result = tool.execute(input, ctx).await.unwrap();
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_toggle_enable_missing_server() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "enable", "server": "nonexistent"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in MCP config")
        );
    }

    #[tokio::test]
    async fn test_toggle_disable_missing_server() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "disable", "server": "nonexistent"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in MCP config")
        );
    }

    #[tokio::test]
    async fn test_toggle_enable_missing_server_field() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "enable"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("'server' is required")
        );
    }

    #[tokio::test]
    async fn test_reload_empty_config() {
        let _guard =
            LocalMcpConfigGuard::new("{\"servers\":{}}").expect("create temporary .jcode/mcp.json");
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "reload"});

        let result = tool.execute(input, ctx).await.unwrap();
        // With config merging, global config may have servers.
        // If both are empty: "No servers found in config"
        // If global has servers: "Reloaded MCP config" (may show connection failures)
        assert!(
            result.output.contains("No servers")
                || result.output.contains("Empty config")
                || result.output.contains("Connected servers: 0")
                || result.output.contains("Reloaded MCP config")
        );
    }

    // --- Happy-path tests for toggle_server and emit_mcp_status ---

    /// RAII guard that sets an env var for the duration of a test and restores
    /// the previous value (or removes it) on drop. Mirrors the helper used in
    /// `ambient/runner_tests.rs`; kept local so this module stays self-contained.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &std::path::Path) -> Self {
            let prev = std::env::var_os(key);
            crate::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                crate::env::set_var(self.key, prev);
            } else {
                crate::env::remove_var(self.key);
            }
        }
    }

    /// Write a single-server MCP config to `<jcode_home>/mcp.json` (the global
    /// config file jcode reads via `jcode_dir()`). `disabled=true` starts the
    /// server disabled so an `enable` toggle has flags to flip; `disabled=false`
    /// starts it enabled for a `disable` toggle.
    ///
    /// The command is a deliberately nonexistent binary so the post-save
    /// `connect` attempt in the enable path fails fast at `spawn()` (ENOENT)
    /// instead of hanging on the 30s JSON-RPC request timeout a process that
    /// exits without speaking MCP would trigger. The config save happens
    /// before that connect attempt, so persistence is testable either way.
    fn write_global_mcp_config(jcode_home: &std::path::Path, server_name: &str, disabled: bool) {
        let config = json!({
            "servers": {
                server_name: {
                    "command": "jcode-mcp-test-nonexistent-server",
                    "args": [],
                    "env": {},
                    "shared": false,
                    "disabled": disabled,
                }
            }
        });
        fs::write(jcode_home.join("mcp.json"), config.to_string()).expect("write global mcp.json");
    }

    /// Read back the global `mcp.json` as a JSON value so tests can assert on
    /// the persisted enabled/disabled flags after a toggle.
    fn read_global_mcp_config(jcode_home: &std::path::Path) -> serde_json::Value {
        let content = fs::read_to_string(jcode_home.join("mcp.json"))
            .expect("read global mcp.json after toggle");
        serde_json::from_str(&content).expect("mcp.json is valid JSON")
    }

    /// Enable a configured-but-disabled server and verify the enabled/disabled
    /// flags are flipped and persisted to `~/.jcode/mcp.json` on disk. The
    /// post-save connect uses `true` (exits immediately, no MCP handshake) so it
    /// fails fast without hanging; the config save happens before that attempt.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_toggle_enable_persists_flags_to_disk() {
        let _env = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().expect("temp jcode home");
        let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

        let server_name = "toggle-enable-srv";
        write_global_mcp_config(temp.path(), server_name, true);

        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "enable", "server": server_name});

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(15), tool.execute(input, ctx))
                .await
                .expect("toggle enable must not hang")
                .expect("toggle enable returns Ok even if the connect fails");

        // Config persistence is the key behavior: flags flipped to enabled.
        let saved = read_global_mcp_config(temp.path());
        let saved_server = saved
            .get("servers")
            .and_then(|s| s.get(server_name))
            .expect("server present after enable toggle");
        assert_eq!(saved_server["enabled"], json!(true));
        assert_eq!(saved_server["disabled"], json!(false));

        assert!(
            result.output.to_lowercase().contains("enabled"),
            "enable toggle should report enabled: {}",
            result.output
        );
    }

    /// Disable a configured-and-enabled server and verify the flags are flipped
    /// and persisted to disk. The server is never connected, so the disable
    /// path skips the disconnect/subprocess entirely.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_toggle_disable_persists_flags_to_disk() {
        let _env = crate::storage::lock_test_env();
        let temp = tempfile::tempdir().expect("temp jcode home");
        let _home = EnvVarGuard::set_path("JCODE_HOME", temp.path());

        let server_name = "toggle-disable-srv";
        write_global_mcp_config(temp.path(), server_name, false);

        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "disable", "server": server_name});

        let result = tool
            .execute(input, ctx)
            .await
            .expect("toggle disable returns Ok");

        let saved = read_global_mcp_config(temp.path());
        let saved_server = saved
            .get("servers")
            .and_then(|s| s.get(server_name))
            .expect("server present after disable toggle");
        assert_eq!(saved_server["enabled"], json!(false));
        assert_eq!(saved_server["disabled"], json!(true));

        assert!(
            result.output.contains("Disabled MCP server"),
            "disable toggle should report disabled: {}",
            result.output
        );
    }

    /// With no connected servers, `emit_mcp_status` still sends an
    /// `McpStatus` event with an empty list through the event channel.
    #[tokio::test]
    async fn test_emit_mcp_status_no_connected_servers() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tool = create_test_tool().with_event_tx(tx);

        tool.emit_mcp_status().await;

        let event = rx
            .try_recv()
            .expect("McpStatus event should be emitted on the channel");
        match event {
            crate::protocol::ServerEvent::McpStatus { servers } => {
                assert!(
                    servers.is_empty(),
                    "no connected servers -> empty status, got {servers:?}"
                );
            }
            other => panic!("expected McpStatus, got {other:?}"),
        }
    }

    /// Write a minimal stdio MCP server (a shell script) that answers
    /// `initialize` and `tools/list` (exposing one tool) so a real connect via
    /// `McpManager` succeeds and `all_tools()` is non-empty.
    #[cfg(unix)]
    fn write_fake_mcp_server(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("jcode-fake-mcp-server.sh");
        let script = r##"#!/bin/bash
while IFS= read -r line; do
  id=$(echo "$line" | grep -o '"id":[0-9]*' | grep -o '[0-9]*' | head -1)
  case "$line" in
    *'"initialize"'*)
      echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"jcode-test-fake","version":"0.0.1"}}}'
      ;;
    *'"tools/list"'*)
      echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"fake_tool","description":"fake tool for emit test","inputSchema":{"type":"object"}}]}}'
      ;;
    *'"shutdown"'*)
      exit 0
      ;;
  esac
done
"##;
        fs::write(&path, script).expect("write fake mcp server script");
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// Happy path for `emit_mcp_status`: a connected server with tools is
    /// formatted as `"name:count"` in the `McpStatus` event payload.
    #[tokio::test]
    #[cfg(unix)]
    async fn test_emit_mcp_status_formats_name_and_tool_count() {
        let temp = tempfile::tempdir().expect("temp dir for fake mcp server");
        let command = write_fake_mcp_server(temp.path());
        let server_name = "jcode-test-fake".to_string();

        let mut config = crate::mcp::McpConfig::default();
        config.servers.insert(
            server_name.clone(),
            McpServerConfig {
                command: command.to_string_lossy().to_string(),
                args: vec![],
                env: HashMap::new(),
                shared: false,
                transport: None,
                url: None,
                headers: HashMap::new(),
                enabled: None,
                disabled: None,
                timeout_secs: None,
            },
        );
        let manager = Arc::new(RwLock::new(McpManager::with_config(config.clone())));
        let server_config = config
            .servers
            .get(&server_name)
            .cloned()
            .expect("server config present");

        // Connect the fake server through the real manager so
        // connected_servers() and all_tools() report it.
        manager
            .read()
            .await
            .connect(&server_name, &server_config)
            .await
            .expect("fake MCP server must connect");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tool = McpManagementTool::new(Arc::clone(&manager)).with_event_tx(tx);
        tool.emit_mcp_status().await;

        let event = rx
            .try_recv()
            .expect("McpStatus event should be emitted after connect");
        match event {
            crate::protocol::ServerEvent::McpStatus { servers } => {
                assert_eq!(
                    servers,
                    vec![format!("{}:1", server_name)],
                    "status should be 'name:count' for the single connected server, got {servers:?}"
                );
            }
            other => panic!("expected McpStatus, got {other:?}"),
        }

        // Clean up the subprocess.
        manager.read().await.disconnect_all().await;
    }

    /// `with_event_tx_optional(Some(tx))` wires the event channel so
    /// `emit_mcp_status` emits an `McpStatus` event. With no servers connected
    /// the payload is an empty list, which still proves the channel was set.
    #[tokio::test]
    async fn test_with_event_tx_optional_some_sets_event_channel() {
        let manager = Arc::new(RwLock::new(McpManager::with_config(
            crate::mcp::McpConfig::default(),
        )));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tool = McpManagementTool::new(Arc::clone(&manager)).with_event_tx_optional(Some(tx));
        tool.emit_mcp_status().await;
        match rx
            .try_recv()
            .expect("McpStatus event should be emitted when event_tx is Some")
        {
            crate::protocol::ServerEvent::McpStatus { servers } => {
                assert!(
                    servers.is_empty(),
                    "no servers connected, expected an empty status list"
                );
            }
            other => panic!("expected McpStatus, got {other:?}"),
        }
    }

    /// `with_event_tx_optional(None)` leaves the tool with no channel, so
    /// `emit_mcp_status` returns early without emitting anything and without
    /// panicking.
    #[tokio::test]
    async fn test_with_event_tx_optional_none_emits_nothing() {
        let manager = Arc::new(RwLock::new(McpManager::with_config(
            crate::mcp::McpConfig::default(),
        )));
        let tool = McpManagementTool::new(Arc::clone(&manager)).with_event_tx_optional(None);
        // With no event_tx the helper short-circuits; this must not panic and
        // emits no event (there is no channel to observe, so no-panic + return
        // is the contract).
        tool.emit_mcp_status().await;
    }
}
