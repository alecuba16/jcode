//! Shared MCP server toggle service.
//!
//! Both the `mcp` management tool (agent-facing) and the TUI `/mcp
//! enable|disable` command flip a server's enabled flag, persist the change to
//! `~/.jcode/mcp.json`, refresh the manager's in-memory config, and connect or
//! disconnect the server while (un)registering its tool proxies. That flow
//! used to live twice with divergent error handling; this module is the single
//! implementation. Call sites decide how to react to the result: the tool
//! logs and continues, the TUI surfaces errors to the user.
//!
//! Registry access goes through [`ToolRegistrar`], implemented by the
//! app-core tool registry, so this module stays independent of that type.

use std::sync::Arc;

use async_trait::async_trait;
use jcode_tool_core::Tool;
use tokio::sync::RwLock;

use super::manager::McpManager;
use super::protocol::{McpConfig, McpServerConfig};

/// Sink for the tool proxies a toggle adds or removes. Implemented by the
/// app-core registry; keeping the trait here lets the shared toggle flow live
/// in `jcode-base`.
#[async_trait]
pub trait ToolRegistrar: Send + Sync {
    async fn register_tool(&self, name: String, tool: Arc<dyn Tool>);
    async fn unregister_tools_for(&self, server_prefix: &str);
}

/// Result of persisting the toggled config, so callers can phrase their
/// user-facing text honestly. The in-memory toggle still applies even when
/// the disk write fails.
#[derive(Debug)]
pub struct TogglePersistence {
    /// Path the config was written to, when the jcode dir resolved.
    pub path: Option<std::path::PathBuf>,
    /// Whether the file write succeeded (false also when the jcode dir
    /// failed to resolve; the flag flip still applies in memory).
    pub save_ok: bool,
    /// Why the save failed, so callers that surface errors (the TUI) can show
    /// the cause instead of just "it failed".
    pub save_error: Option<String>,
}

impl TogglePersistence {
    /// Suffix describing the save outcome, for user-facing text.
    pub fn describe(&self) -> String {
        match (&self.path, self.save_ok) {
            (Some(path), true) => format!("Config saved to {}.", path.display()),
            (Some(path), false) => format!(
                "Config NOT saved (write to {} failed; toggle applies until restart).",
                path.display()
            ),
            (None, _) => "Config NOT saved (could not resolve ~/.jcode).".to_string(),
        }
    }
}

/// Flip a server's enabled flag and persist the new config to
/// `~/.jcode/mcp.json`. Sync and lock-free so both the async tool path and the
/// sync TUI command can call it before doing their async work. The caller
/// decides whether a save failure is fatal; the event log gets a
/// `toggle_save_failed` warn entry either way.
pub fn persist_toggle(
    config: &McpConfig,
    server_name: &str,
    enable: bool,
) -> anyhow::Result<(McpServerConfig, TogglePersistence)> {
    let mut server_config = config
        .servers
        .get(server_name)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Server '{}' not found in MCP config. Add it to ~/.jcode/mcp.json first.",
                server_name
            )
        })?;

    server_config.disabled = Some(!enable);
    server_config.enabled = Some(enable);

    let mut saved_config = config.clone();
    saved_config
        .servers
        .insert(server_name.to_string(), server_config.clone());

    let persistence = match crate::storage::jcode_dir() {
        Ok(jcode_dir) => {
            let mcp_path = jcode_dir.join("mcp.json");
            match saved_config.save_to_file(&mcp_path) {
                Ok(()) => TogglePersistence {
                    path: Some(mcp_path),
                    save_ok: true,
                    save_error: None,
                },
                Err(e) => {
                    crate::logging::event_warn(
                        "MCP_LIFECYCLE",
                        vec![
                            ("phase", "toggle_save_failed".to_string()),
                            ("server", server_name.to_string()),
                            ("error", e.to_string()),
                        ],
                    );
                    TogglePersistence {
                        path: Some(mcp_path),
                        save_ok: false,
                        save_error: Some(e.to_string()),
                    }
                }
            }
        }
        Err(e) => TogglePersistence {
            path: None,
            save_ok: false,
            save_error: Some(e.to_string()),
        },
    };

    Ok((server_config, persistence))
}

/// Connect the (just enabled) server and register its tool proxies in the
/// optional registrar. Returns the number of tools the server exposes and any
/// connection error; on error nothing is registered.
pub async fn connect_and_register(
    manager: &Arc<RwLock<McpManager>>,
    server_name: &str,
    server_config: &McpServerConfig,
    registrar: Option<&dyn ToolRegistrar>,
) -> (usize, Option<String>) {
    let mgr = manager.read().await;
    if let Err(e) = mgr.connect(server_name, server_config).await {
        return (0, Some(format!("{:#}", e)));
    }
    let tools = mgr.all_tools().await;
    let tool_count = tools.iter().filter(|(s, _)| s == server_name).count();
    drop(mgr);

    if let Some(registrar) = registrar {
        let mcp_tools = super::create_mcp_tools_for_server(Arc::clone(manager), server_name).await;
        for (name, tool) in mcp_tools {
            registrar.register_tool(name, tool).await;
        }
    }

    (tool_count, None)
}

/// Disconnect the (just disabled) server and unregister its tool proxies from
/// the optional registrar. Connection errors are logged, not returned: the
/// flag flip already succeeded and the config no longer spawns the server.
pub async fn disconnect_and_unregister(
    manager: &Arc<RwLock<McpManager>>,
    server_name: &str,
    registrar: Option<&dyn ToolRegistrar>,
) {
    let mgr = manager.read().await;
    let connected = mgr.connected_servers().await;
    if connected.iter().any(|name| name == server_name)
        && let Err(error) = mgr.disconnect(server_name).await
    {
        crate::logging::event_error(
            "mcp_toggle_disconnect_failed",
            vec![
                ("server", server_name.to_string()),
                ("error", error.to_string()),
            ],
        );
    }
    drop(mgr);

    if let Some(registrar) = registrar {
        registrar
            .unregister_tools_for(&super::dispatch_name(server_name, ""))
            .await;
    }
}