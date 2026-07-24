//! Cursor CLI ACP provider runtime.
//!
//! The runtime owns one Cursor `agent acp` subprocess per provider instance.
//! Cursor remains the authority for the model catalog. Jcode only stores the
//! advertised opaque IDs and applies an explicit, deterministic selection rule.

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use jcode_message_types::{ContentBlock, Message, Role, StreamEvent, ToolDefinition};
use jcode_provider_core::{EventStream, ModelRoute, Provider};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

const DEFAULT_COMMAND: &str = "agent";
const DEFAULT_ACP_ARG: &str = "acp";
const ACP_PROTOCOL_VERSION: u64 = 1;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;

/// Controlled Cursor ACP executable command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorAcpCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl CursorAcpCommand {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Read the command from explicit environment configuration.
    ///
    /// `JCODE_CURSOR_ACP_PATH` controls the executable. `JCODE_CURSOR_ACP_ARGS`
    /// is a whitespace-separated argument list and defaults to `acp`.
    pub fn from_env() -> Self {
        let program = std::env::var("JCODE_CURSOR_ACP_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_COMMAND.to_string());
        let args = std::env::var("JCODE_CURSOR_ACP_ARGS")
            .ok()
            .map(|value| value.split_whitespace().map(str::to_string).collect())
            .filter(|args: &Vec<String>| !args.is_empty())
            .unwrap_or_else(|| vec![DEFAULT_ACP_ARG.to_string()]);
        Self { program, args }
    }
}

impl Default for CursorAcpCommand {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Return the stable base part of a Cursor ACP model ID.
///
/// IDs are opaque to Jcode. This helper is only used for the optional
/// convenience resolution of a bare ID when exactly one advertised variant
/// exists.
pub fn model_base_id(model: &str) -> &str {
    model.split_once('[').map_or(model, |(base, _)| base)
}

/// Resolve a requested model against the catalog advertised by Cursor ACP.
///
/// Exact IDs always win. A bare ID is accepted only when one advertised
/// bracketed variant has that base. No static model list or silent fallback is
/// used.
pub fn resolve_model(
    requested: Option<&str>,
    advertised: &[String],
    current: Option<&str>,
) -> Result<String> {
    let current = current.map(str::trim).filter(|value| !value.is_empty());
    let mut models = Vec::new();
    for model in advertised {
        let model = model.trim();
        if !model.is_empty() && !models.iter().any(|known| known == model) {
            models.push(model.to_string());
        }
    }

    let Some(requested) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return current
            .map(ToString::to_string)
            .or_else(|| models.first().cloned())
            .ok_or_else(|| anyhow!("Cursor ACP did not advertise a current or available model"));
    };

    if let Some(exact) = models.iter().find(|model| model.as_str() == requested) {
        return Ok(exact.clone());
    }

    let candidates: Vec<&String> = models
        .iter()
        .filter(|model| model_base_id(model) == requested)
        .collect();
    match candidates.as_slice() {
        [only] => Ok((*only).clone()),
        [] => bail!(
            "Cursor ACP does not advertise model '{}'. Available models: {}",
            requested,
            if models.is_empty() {
                "none".to_string()
            } else {
                models.join(", ")
            }
        ),
        _ => bail!(
            "Cursor ACP model '{}' is ambiguous. Use the exact advertised ID: {}",
            requested,
            candidates
                .iter()
                .map(|candidate| candidate.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

#[derive(Debug)]
struct IncomingMessage {
    id: Option<Value>,
    method: Option<String>,
    params: Value,
    result: Option<Value>,
    error: Option<Value>,
}

impl IncomingMessage {
    fn parse(line: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(line)
            .with_context(|| format!("invalid Cursor ACP JSON: {}", line.trim()))?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow!("Cursor ACP message must be a JSON object"))?;
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            bail!("Cursor ACP message is not JSON-RPC 2.0");
        }
        Ok(Self {
            id: object.get("id").cloned(),
            method: object
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_string),
            params: object.get("params").cloned().unwrap_or(Value::Null),
            result: object.get("result").cloned(),
            error: object.get("error").cloned(),
        })
    }
}

#[derive(Debug, Default)]
struct ModelCatalog {
    models: Vec<String>,
    current: Option<String>,
    config_id: Option<String>,
}

impl ModelCatalog {
    fn merge(&mut self, value: &Value) {
        let mut discovered = Vec::new();
        if let Some(models) = value.get("models") {
            discovered.extend(model_ids(models.get("availableModels").unwrap_or(models)));
            self.current = string_value(models.get("currentModelId"));
        }
        if let Some(options) = value.get("configOptions").and_then(Value::as_array) {
            for option in options {
                let is_model = option.get("category").and_then(Value::as_str) == Some("model")
                    || option.get("id").and_then(Value::as_str) == Some("model");
                if !is_model {
                    continue;
                }
                self.config_id = string_value(option.get("id")).or_else(|| Some("model".into()));
                discovered.extend(model_ids(option.get("options").unwrap_or(&Value::Null)));
                self.current = string_value(option.get("currentValue")).or(self.current.take());
            }
        }
        if string_value(value.get("configId")).as_deref() == Some("model")
            || value.get("category").and_then(Value::as_str) == Some("model")
        {
            self.config_id = string_value(value.get("configId")).or_else(|| Some("model".into()));
            discovered.extend(model_ids(value.get("options").unwrap_or(&Value::Null)));
            self.current = string_value(value.get("value"))
                .or_else(|| string_value(value.get("currentValue")))
                .or(self.current.take());
        }
        for model in discovered {
            if !model.is_empty() && !self.models.iter().any(|known| known == &model) {
                self.models.push(model);
            }
        }
        if let Some(current) = self.current.clone()
            && !self.models.iter().any(|model| model == &current)
        {
            self.models.push(current);
        }
    }
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn model_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .map(str::to_string)
                .or_else(|| string_value(item.get("id")))
                .or_else(|| string_value(item.get("value")))
        })
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect()
}

struct AcpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    session_id: String,
    catalog: ModelCatalog,
    supports_images: bool,
    thinking_active: bool,
}

impl Drop for AcpProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl AcpProcess {
    async fn spawn(command: &CursorAcpCommand, cwd: &Path) -> Result<Self> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to launch Cursor ACP command '{} {}'",
                    command.program,
                    command.args.join(" ")
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Cursor ACP process did not expose stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Cursor ACP process did not expose stdout"))?;
        let mut process = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            session_id: String::new(),
            catalog: ModelCatalog::default(),
            supports_images: false,
            thinking_active: false,
        };
        process.initialize().await?;
        process.new_session(cwd).await?;
        Ok(process)
    }

    async fn initialize(&mut self) -> Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "clientCapabilities": {},
                    "clientInfo": {
                        "name": "jcode",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                }),
                None,
            )
            .await?;
        self.supports_images = result
            .pointer("/agentCapabilities/promptCapabilities/image")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(())
    }

    async fn new_session(&mut self, cwd: &Path) -> Result<()> {
        let result = self
            .request(
                "session/new",
                json!({
                    "cwd": cwd.to_string_lossy(),
                    "mcpServers": [],
                }),
                None,
            )
            .await?;
        self.session_id = string_value(result.get("sessionId"))
            .ok_or_else(|| anyhow!("Cursor ACP session/new response did not include sessionId"))?;
        self.catalog.merge(&result);
        Ok(())
    }

    async fn set_model(&mut self, model: &str) -> Result<()> {
        let config_id =
            self.catalog.config_id.clone().ok_or_else(|| {
                anyhow!("Cursor ACP did not advertise a model configuration option")
            })?;
        let result = self
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": self.session_id,
                    "configId": config_id,
                    "value": model,
                }),
                None,
            )
            .await?;
        self.catalog.merge(&result);
        self.catalog.current = Some(model.to_string());
        Ok(())
    }

    async fn prompt(
        &mut self,
        prompt: Vec<Value>,
        tx: &mpsc::Sender<Result<StreamEvent>>,
    ) -> Result<Value> {
        let result = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": self.session_id,
                    "prompt": prompt,
                }),
                Some(tx),
            )
            .await?;
        if self.thinking_active {
            self.thinking_active = false;
            tx.send(Ok(StreamEvent::ThinkingEnd)).await.ok();
        }
        Ok(result)
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        tx: Option<&mpsc::Sender<Result<StreamEvent>>>,
    ) -> Result<Value> {
        let id = Value::from(self.next_id);
        self.next_id += 1;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;

        loop {
            let message = self.read_message(tx).await?;
            if let Some(incoming_method) = message.method.as_deref() {
                match incoming_method {
                    "session/update" => self.handle_update(&message.params, tx).await?,
                    "session/request_permission" => self.handle_permission(&message).await?,
                    _ => {
                        if let Some(request_id) = message.id {
                            self.write(json!({
                                "jsonrpc": "2.0",
                                "id": request_id,
                                "error": {"code": JSONRPC_METHOD_NOT_FOUND, "message": format!("Unsupported Cursor ACP client method: {incoming_method}")}
                            }))
                            .await?;
                        }
                    }
                }
                continue;
            }
            if message.id.as_ref() != Some(&id) {
                continue;
            }
            if let Some(error) = message.error {
                bail!("Cursor ACP request '{}' failed: {}", method, error);
            }
            return Ok(message.result.unwrap_or(Value::Null));
        }
    }

    async fn read_message(
        &mut self,
        tx: Option<&mpsc::Sender<Result<StreamEvent>>>,
    ) -> Result<IncomingMessage> {
        loop {
            let mut line = String::new();
            let bytes = if let Some(tx) = tx {
                tokio::select! {
                    _ = tx.closed() => bail!("Cursor ACP stream was cancelled"),
                    result = self.stdout.read_line(&mut line) => result?,
                }
            } else {
                self.stdout.read_line(&mut line).await?
            };
            if bytes == 0 {
                let status = self.child.try_wait().ok().flatten();
                bail!("Cursor ACP process exited before responding ({status:?})");
            }
            if line.trim().is_empty() {
                continue;
            }
            return IncomingMessage::parse(&line);
        }
    }

    async fn write(&mut self, value: Value) -> Result<()> {
        let mut encoded = serde_json::to_string(&value)?;
        encoded.push('\n');
        self.stdin.write_all(encoded.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn handle_permission(&mut self, message: &IncomingMessage) -> Result<()> {
        let Some(id) = message.id.clone() else {
            return Ok(());
        };
        let options = message
            .params
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let configured = std::env::var("JCODE_CURSOR_ACP_PERMISSION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "reject_once".to_string());
        let selected = options.iter().find_map(|option| {
            let id =
                string_value(option.get("optionId")).or_else(|| string_value(option.get("id")))?;
            (id == configured).then_some(id)
        });
        let result = if let Some(option_id) = selected {
            json!({"outcome": {"outcome": "selected", "optionId": option_id}})
        } else {
            json!({"outcome": {"outcome": "cancelled"}})
        };
        self.write(json!({"jsonrpc":"2.0","id":id,"result":result}))
            .await
    }

    async fn handle_update(
        &mut self,
        params: &Value,
        tx: Option<&mpsc::Sender<Result<StreamEvent>>>,
    ) -> Result<()> {
        let update = params.get("update").unwrap_or(params);
        self.catalog.merge(update);
        let kind = update
            .get("sessionUpdate")
            .or_else(|| update.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut events = Vec::new();
        match kind {
            "agent_message_chunk" => {
                if self.thinking_active {
                    self.thinking_active = false;
                    events.push(StreamEvent::ThinkingEnd);
                }
                if let Some(text) = content_text(update.get("content")) {
                    events.push(StreamEvent::TextDelta(text));
                }
            }
            "agent_thought_chunk" => {
                if !self.thinking_active {
                    self.thinking_active = true;
                    events.push(StreamEvent::ThinkingStart);
                }
                if let Some(text) = content_text(update.get("content")) {
                    events.push(StreamEvent::ThinkingDelta(text));
                }
            }
            "tool_call" | "tool_call_update" => {
                if let Some(title) = string_value(update.get("title")) {
                    events.push(StreamEvent::StatusDetail { detail: title });
                }
            }
            "config_option_update" | "current_mode_update" => {
                if let Some(value) = string_value(update.get("value")) {
                    events.push(StreamEvent::StatusDetail {
                        detail: format!("Cursor ACP configuration: {value}"),
                    });
                }
            }
            "plan" => events.push(StreamEvent::StatusDetail {
                detail: "Cursor ACP updated its plan".to_string(),
            }),
            "usage_update" => {
                let input = update
                    .get("inputTokens")
                    .or_else(|| update.get("input_tokens"))
                    .and_then(Value::as_u64);
                let output = update
                    .get("outputTokens")
                    .or_else(|| update.get("output_tokens"))
                    .and_then(Value::as_u64);
                if input.is_some() || output.is_some() {
                    events.push(StreamEvent::TokenUsage {
                        input_tokens: input,
                        output_tokens: output,
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            }
            _ => {}
        }
        if let Some(tx) = tx {
            for event in events {
                tx.send(Ok(event))
                    .await
                    .map_err(|_| anyhow!("Cursor ACP stream was cancelled"))?;
            }
        }
        Ok(())
    }
}

fn content_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| string_value(value.get("text")))
}

#[derive(Clone)]
pub struct CursorAcpProvider {
    command: CursorAcpCommand,
    cwd: Arc<PathBuf>,
    session: Arc<Mutex<Option<AcpProcess>>>,
    discovered_models: Arc<RwLock<Vec<String>>>,
    model: Arc<RwLock<Option<String>>>,
    supports_images: Arc<RwLock<bool>>,
}

impl CursorAcpProvider {
    pub fn new() -> Self {
        Self::with_command(CursorAcpCommand::from_env())
    }

    pub fn with_command(command: CursorAcpCommand) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let model = std::env::var("JCODE_CURSOR_ACP_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty());
        Self {
            command,
            cwd: Arc::new(cwd),
            session: Arc::new(Mutex::new(None)),
            discovered_models: Arc::new(RwLock::new(Vec::new())),
            model: Arc::new(RwLock::new(model)),
            supports_images: Arc::new(RwLock::new(false)),
        }
    }

    fn with_state(&self, process: &AcpProcess) {
        if let Ok(mut models) = self.discovered_models.write() {
            *models = process.catalog.models.clone();
        }
        if let Some(current) = process.catalog.current.clone()
            && let Ok(mut model) = self.model.write()
        {
            if model.is_none() {
                *model = Some(current);
            }
        }
        if let Ok(mut supports_images) = self.supports_images.write() {
            *supports_images = process.supports_images;
        }
    }

    async fn ensure_process<'a>(
        &'a self,
        guard: &'a mut Option<AcpProcess>,
    ) -> Result<&'a mut AcpProcess> {
        if guard.is_none() {
            *guard = Some(AcpProcess::spawn(&self.command, &self.cwd).await?);
        }
        Ok(guard.as_mut().expect("Cursor ACP process initialized"))
    }

    async fn configure_process(&self, process: &mut AcpProcess) -> Result<String> {
        self.with_state(process);
        let requested = self.model.read().ok().and_then(|model| model.clone());
        let selected = resolve_model(
            requested.as_deref(),
            &process.catalog.models,
            process.catalog.current.as_deref(),
        )?;
        if process.catalog.current.as_deref() != Some(selected.as_str()) {
            process.set_model(&selected).await?;
        }
        if let Ok(mut model) = self.model.write() {
            *model = Some(selected.clone());
        }
        self.with_state(process);
        Ok(selected)
    }

    async fn run_prompt(
        &self,
        prompt: Vec<Value>,
        tx: &mpsc::Sender<Result<StreamEvent>>,
    ) -> Result<()> {
        let mut guard = self.session.lock().await;
        let result = async {
            let process = self.ensure_process(&mut guard).await?;
            let selected = self.configure_process(process).await?;
            let response = process.prompt(prompt, tx).await?;
            tx.send(Ok(StreamEvent::SessionId(process.session_id.clone())))
                .await
                .map_err(|_| anyhow!("Cursor ACP stream was cancelled"))?;
            tx.send(Ok(StreamEvent::ConnectionType {
                connection: format!("cursor-acp:{}", selected),
            }))
            .await
            .map_err(|_| anyhow!("Cursor ACP stream was cancelled"))?;
            tx.send(Ok(StreamEvent::MessageEnd {
                stop_reason: string_value(response.get("stopReason")),
            }))
            .await
            .map_err(|_| anyhow!("Cursor ACP stream was cancelled"))?;
            self.with_state(process);
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if result.is_err() {
            guard.take();
        }
        result
    }

    pub fn command(&self) -> &CursorAcpCommand {
        &self.command
    }
}

impl Default for CursorAcpProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for CursorAcpProvider {
    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[ToolDefinition],
        system: &str,
        _resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        let prompt = build_prompt(system, messages);
        let provider = self.clone();
        let (tx, rx) = mpsc::channel::<Result<StreamEvent>>(128);
        tokio::spawn(async move {
            if let Err(error) = provider.run_prompt(prompt, &tx).await
                && !tx.is_closed()
            {
                let _ = tx.send(Err(error)).await;
            }
        });
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    fn name(&self) -> &str {
        "cursor-acp"
    }

    fn display_name(&self) -> String {
        "Cursor ACP".to_string()
    }

    fn model(&self) -> String {
        self.model
            .read()
            .ok()
            .and_then(|model| model.clone())
            .unwrap_or_else(|| "cursor-acp:discovering".to_string())
    }

    fn set_model(&self, model: &str) -> Result<()> {
        let requested = model.trim();
        if requested.is_empty() {
            bail!("Cursor ACP model cannot be empty");
        }
        let advertised = self
            .discovered_models
            .read()
            .map(|models| models.clone())
            .unwrap_or_default();
        if !advertised.is_empty() {
            let current = self.model.read().ok().and_then(|model| model.clone());
            let resolved = resolve_model(Some(requested), &advertised, current.as_deref())?;
            *self
                .model
                .write()
                .map_err(|_| anyhow!("Cursor ACP model state is poisoned"))? = Some(resolved);
        } else {
            *self
                .model
                .write()
                .map_err(|_| anyhow!("Cursor ACP model state is poisoned"))? =
                Some(requested.to_string());
        }
        Ok(())
    }

    fn available_models(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn available_models_display(&self) -> Vec<String> {
        self.discovered_models
            .read()
            .map(|models| models.clone())
            .unwrap_or_default()
    }

    fn available_models_for_switching(&self) -> Vec<String> {
        self.available_models_display()
    }

    fn model_routes(&self) -> Vec<ModelRoute> {
        self.available_models_display()
            .into_iter()
            .map(|model| ModelRoute {
                model,
                provider: "Cursor ACP".to_string(),
                api_method: "cursor-acp".to_string(),
                available: true,
                detail: "Advertised by Cursor CLI ACP".to_string(),
                cheapness: None,
            })
            .collect()
    }

    async fn prefetch_models(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        let result = async {
            let process = self.ensure_process(&mut guard).await?;
            self.configure_process(process).await?;
            self.with_state(process);
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if result.is_err() {
            guard.take();
        }
        result
    }

    fn handles_tools_internally(&self) -> bool {
        true
    }

    fn supports_image_input(&self) -> bool {
        self.supports_images.read().is_ok_and(|value| *value)
    }

    fn fork(&self) -> Arc<dyn Provider> {
        Arc::new(Self {
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            session: Arc::new(Mutex::new(None)),
            discovered_models: self.discovered_models.clone(),
            model: Arc::new(RwLock::new(
                self.model.read().ok().and_then(|model| model.clone()),
            )),
            supports_images: self.supports_images.clone(),
        })
    }
}

fn build_prompt(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut text = String::new();
    if !system.trim().is_empty() {
        text.push_str("System:\n");
        text.push_str(system.trim());
        text.push_str("\n\n");
    }
    text.push_str("Conversation:\n");
    let mut images = Vec::new();
    for message in messages {
        text.push_str(match message.role {
            Role::User => "User:\n",
            Role::Assistant => "Assistant:\n",
        });
        for block in &message.content {
            match block {
                ContentBlock::Text { text: value, .. } => {
                    text.push_str(value);
                    text.push('\n');
                }
                ContentBlock::ToolUse {
                    id, name, input, ..
                } => {
                    text.push_str(&format!("[tool_use id={id} name={name} input={input}]\n"));
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    text.push_str(&format!(
                        "[tool_result id={tool_use_id} error={}]\n{content}\n",
                        is_error.unwrap_or(false)
                    ));
                }
                ContentBlock::Image { media_type, data } => {
                    images.push(json!({"type":"image","data":data,"mimeType":media_type}));
                    text.push_str("[image]\n");
                }
                ContentBlock::Reasoning { text: value }
                | ContentBlock::ReasoningTrace { text: value }
                | ContentBlock::AnthropicThinking {
                    thinking: value, ..
                } => {
                    text.push_str(&format!("[reasoning]\n{value}\n"));
                }
                ContentBlock::OpenAIReasoning { summary, .. } => {
                    text.push_str(&format!("[reasoning]\n{}\n", summary.join("\n")));
                }
                ContentBlock::OpenAICompaction { .. } => {
                    text.push_str("[compaction]\n");
                }
            }
        }
        text.push('\n');
    }
    text.push_str("Assistant:\n");
    let mut prompt = vec![json!({"type":"text","text":text})];
    prompt.extend(images);
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn exact_model_id_wins_and_preserves_settings() {
        let models = ids(&["composer-2.5[fast=true]", "gpt-5.3-codex"]);
        assert_eq!(
            resolve_model(Some("composer-2.5[fast=true]"), &models, None).unwrap(),
            "composer-2.5[fast=true]"
        );
    }

    #[test]
    fn bare_model_resolves_only_one_variant() {
        let models = ids(&["gpt-5.6-sol[reasoning=medium,fast=false]"]);
        assert_eq!(
            resolve_model(Some("gpt-5.6-sol"), &models, None).unwrap(),
            "gpt-5.6-sol[reasoning=medium,fast=false]"
        );
    }

    #[test]
    fn ambiguous_bare_model_is_rejected() {
        let models = ids(&["composer-2.5[fast=true]", "composer-2.5[fast=false]"]);
        let error = resolve_model(Some("composer-2.5"), &models, None).unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
    }

    #[test]
    fn unsupported_model_is_rejected_without_fallback() {
        let models = ids(&["gpt-5.3-codex"]);
        let error =
            resolve_model(Some("not-advertised"), &models, Some("gpt-5.3-codex")).unwrap_err();
        assert!(error.to_string().contains("does not advertise"));
    }

    #[test]
    fn omitted_model_uses_cursor_current_model() {
        let models = ids(&["gpt-5.3-codex", "composer-2.5[fast=true]"]);
        assert_eq!(
            resolve_model(None, &models, Some("composer-2.5[fast=true]")).unwrap(),
            "composer-2.5[fast=true]"
        );
    }

    #[test]
    fn command_defaults_to_cursor_acp_protocol_argument() {
        let command = CursorAcpCommand::new("agent", ["acp"]);
        assert_eq!(command.program, "agent");
        assert_eq!(command.args, vec!["acp"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_acp_executable_returns_controlled_error() {
        let provider = CursorAcpProvider::with_command(CursorAcpCommand::new(
            "jcode-cursor-acp-command-that-does-not-exist",
            ["acp"],
        ));
        let error = provider.prefetch_models().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to launch Cursor ACP command")
        );
    }

    #[test]
    fn catalog_extracts_string_and_object_model_ids() {
        let mut catalog = ModelCatalog::default();
        catalog.merge(&json!({
            "models": {
                "currentModelId": "gpt-5.3-codex",
                "availableModels": [
                    "gpt-5.3-codex",
                    {"id": "composer-2.5[fast=true]"}
                ]
            },
            "configOptions": [{
                "id": "model",
                "category": "model",
                "currentValue": "gpt-5.3-codex",
                "options": [{"value": "gpt-5.3-codex"}]
            }]
        }));
        assert_eq!(catalog.config_id.as_deref(), Some("model"));
        assert_eq!(catalog.current.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(
            catalog.models,
            ids(&["gpt-5.3-codex", "composer-2.5[fast=true]"])
        );
    }

    #[test]
    fn catalog_applies_model_config_updates() {
        let mut catalog = ModelCatalog::default();
        catalog.merge(&json!({
            "configOptions": [{
                "id": "model",
                "category": "model",
                "currentValue": "gpt-5.3-codex",
                "options": [{"value": "gpt-5.3-codex"}, {"value": "composer-2.5[fast=true]"}]
            }]
        }));

        catalog.merge(&json!({
            "configId": "model",
            "value": "composer-2.5[fast=true]"
        }));

        assert_eq!(catalog.current.as_deref(), Some("composer-2.5[fast=true]"));
        assert_eq!(catalog.config_id.as_deref(), Some("model"));
        assert_eq!(catalog.models.len(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_acp_process_discovers_models_and_streams_prompt() {
        use tokio_stream::StreamExt;

        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"agentCapabilities":{"promptCapabilities":{"image":true}}}}\n' "$id"
      ;;
    *'"method":"session/new"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":'"$id"',"result":{"sessionId":"fake-session","models":{"currentModelId":"gpt-5.3-codex","availableModels":["gpt-5.3-codex","composer-2.5[fast=true]"]},"configOptions":[{"id":"model","category":"model","currentValue":"gpt-5.3-codex","options":[{"value":"gpt-5.3-codex"},{"value":"composer-2.5[fast=true]"}]}]}}'
      ;;
    *'"method":"session/set_config_option"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      ;;
    *'"method":"session/prompt"'*)
      printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello from fake Cursor"}}}}'
      printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id"
      ;;
  esac
done
"#;
        let provider = CursorAcpProvider::with_command(CursorAcpCommand::new("sh", ["-c", script]));

        provider.prefetch_models().await.unwrap();
        assert_eq!(
            provider.available_models_display(),
            ids(&["gpt-5.3-codex", "composer-2.5[fast=true]"])
        );
        assert_eq!(provider.model(), "gpt-5.3-codex");
        assert!(provider.supports_image_input());

        let mut stream = provider
            .complete(&[Message::user("hello")], &[], "", None)
            .await
            .unwrap();
        let mut text = String::new();
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::MessageEnd { stop_reason } => {
                    assert_eq!(stop_reason.as_deref(), Some("end_turn"));
                }
                _ => {}
            }
        }
        assert_eq!(text, "hello from fake Cursor");
    }
}
