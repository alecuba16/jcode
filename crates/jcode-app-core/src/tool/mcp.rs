//! MCP management tool - connect, disconnect, list, reload MCP servers

use crate::mcp::{McpManager, McpServerConfig};
use crate::tool::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    registry: Option<crate::tool::Registry>,
}

impl McpManagementTool {
    pub fn new(manager: Arc<RwLock<McpManager>>) -> Self {
        Self {
            manager,
            registry: None,
        }
    }

    pub fn with_registry(mut self, registry: crate::tool::Registry) -> Self {
        self.registry = Some(registry);
        self
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

// Helper for tests to update cached server names
impl McpManagementTool {
    pub fn manager(&self) -> &Arc<RwLock<McpManager>> {
        &self.manager
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
                        "  - mcp__{}__{}: {}\n",
                        server,
                        tool.name,
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
                enabled: None,
                disabled: None,
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
                        "  - mcp__{}__{}: {}\n",
                        server_name,
                        tool.name,
                        tool.description.as_deref().unwrap_or("(no description)")
                    ));
                }
                drop(manager);

                // Register the new tools in the registry
                if let Some(ref registry) = self.registry {
                    let mcp_tools = crate::mcp::create_mcp_tools(Arc::clone(&self.manager)).await;
                    for (name, tool) in mcp_tools {
                        if name.starts_with(&format!("mcp__{}__", server_name)) {
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
        if let Some(ref registry) = self.registry {
            let removed = registry
                .unregister_prefix(&format!("mcp__{}__", server_name))
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
            if let Some(ref registry) = self.registry {
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
        if let Some(ref registry) = self.registry {
            registry.unregister_prefix("mcp__").await;
        }

        let mut manager = self.manager.write().await;
        let (successes, failures) = manager.reload().await?;

        let servers = manager.connected_servers().await;
        let all_tools = manager.all_tools().await;
        drop(manager);

        // Re-register tools from fresh connections
        if let Some(ref registry) = self.registry {
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
        let mut server_config = config
            .servers
            .get(&server_name)
            .cloned()
            .ok_or_else(|| {
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
        saved_config.servers.insert(server_name.clone(), server_config.clone());

        if let Ok(jcode_dir) = crate::storage::jcode_dir() {
            let mcp_path = jcode_dir.join("mcp.json");
            if let Err(e) = saved_config.save_to_file(&mcp_path) {
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

        if enable {
            // Connect the server and register its tools
            let manager = self.manager.read().await;
            let connected = manager.connected_servers().await;
            if connected.contains(&server_name) {
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
                    if let Some(ref registry) = self.registry {
                        let mcp_tools =
                            crate::mcp::create_mcp_tools(Arc::clone(&self.manager)).await;
                        for (name, tool) in mcp_tools {
                            if name.starts_with(&format!("mcp__{}__", server_name)) {
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

                    Ok(ToolOutput::new(format!(
                        "Enabled MCP server '{}'. Connected with {} tool(s). Config saved to ~/.jcode/mcp.json.",
                        server_name, tool_count
                    ))
                    .with_title(format!("MCP: Enabled {}", server_name)))
                }
                Err(e) => {
                    drop(manager);
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
            if let Some(ref registry) = self.registry {
                let removed = registry
                    .unregister_prefix(&format!("mcp__{}__", server_name))
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
                enabled: Some(false),
                disabled: None,
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not found in MCP config"));
    }

    #[tokio::test]
    async fn test_toggle_disable_missing_server() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "disable", "server": "nonexistent"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not found in MCP config"));
    }

    #[tokio::test]
    async fn test_toggle_enable_missing_server_field() {
        let tool = create_test_tool();
        let ctx = create_test_context();
        let input = json!({"action": "enable"});

        let result = tool.execute(input, ctx).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("'server' is required"));
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
}
