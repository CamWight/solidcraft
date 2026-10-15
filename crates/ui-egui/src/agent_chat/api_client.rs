//! The API backend (desktop only): a model behind an OpenAI-compatible chat completions endpoint
//! (Ollama, LM Studio, OpenAI, any URL the user gives). SolidCraft runs the tool loop itself, with
//! the same tools the ACP agents get: it starts the same MCP server (`solidcraft-cli mcp
//! --connect` to the chat's own endpoint) and is its client, so every tool call passes the same
//! gate, limits and folder checks. The key, when there is one, comes from the system keychain for
//! this connection only.
//!
//! The loop runs on a thread of its own: a model request can take a while.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};

use super::backend::{ChatBackend, Event, HttpFn, HttpRequest, PromptPart, StartContext, ToolStatus};
use super::process::{AgentProcess, FromAgent, ProcessHandle};

/// Model requests for one message, at most (each answer with tool calls is one).
pub const MAX_REQUESTS_PER_TURN: usize = 24;
/// Longest tool result handed back to the model.
const MAX_TOOL_TEXT: usize = 20_000;
/// Messages kept in the conversation (the system note stays).
const MAX_HISTORY: usize = 240;
/// How long one model request may take.
const REQUEST_TIMEOUT_S: u64 = 300;
/// How long one tool call may take (it may wait for the user's Allow).
const TOOL_TIMEOUT: Duration = Duration::from_secs(330);

/// Tools the model is not offered.
const HIDDEN_TOOLS: &[&str] = &["crash_reports"];

pub struct ApiConfig {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub key: Option<String>,
    pub http: HttpFn,
}

enum Cmd {
    Prompt(Vec<PromptPart>),
}

pub struct ApiBackend {
    cmds: Option<Sender<Cmd>>,
    events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    mcp: Option<ProcessHandle>,
}

impl ApiBackend {
    pub fn start(cfg: ApiConfig, ctx: StartContext) -> Result<ApiBackend, String> {
        if cfg.key.is_some() && !super::agents::key_safe_for(&cfg.base_url) {
            return Err(crate::i18n::tr("ui.ai_key_needs_https").to_string());
        }
        // The same MCP server an ACP agent would get.
        let mcp = match &ctx.bridge {
            Some(b) => {
                let argv = vec![b.cli.to_string_lossy().into_owned(), "mcp".into(), "--connect".into(), format!("127.0.0.1:{}", b.port)];
                Some(AgentProcess::spawn(&argv, &ctx.cwd, &["SOLIDCRAFT_CONTROL_TOKEN"], || {})?)
            }
            None => None,
        };
        let handle = mcp.as_ref().map(AgentProcess::handle);
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let (ev_tx, events) = channel::<Event>();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::from(ctx.wake);
        std::thread::spawn(move || {
            let mut w = Worker { cfg, mcp, next: 0, tools: Vec::new(), history: Vec::new(), events: ev_tx, wake, cancel: stop };
            w.run(cmd_rx);
        });
        Ok(ApiBackend { cmds: Some(cmd_tx), events, cancel, mcp: handle })
    }
}

impl ChatBackend for ApiBackend {
    fn poll(&mut self) -> Vec<Event> {
        self.events.try_iter().take(512).collect()
    }
    fn prompt(&mut self, parts: &[PromptPart]) {
        self.cancel.store(false, Ordering::Relaxed);
        if let Some(c) = &self.cmds {
            let _ = c.send(Cmd::Prompt(parts.to_vec()));
        }
    }
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    fn answer_permission(&mut self, _key: u64, _option: Option<&str>) {}
    fn authenticate(&mut self, _method: &str) {}
    fn pid(&self) -> Option<u32> {
        self.mcp.as_ref().and_then(ProcessHandle::pid)
    }
    fn shutdown(&mut self, wait: bool) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cmds = None;
        if let Some(h) = self.mcp.take() {
            h.shutdown(wait);
        }
    }
}

impl Drop for ApiBackend {
    fn drop(&mut self) {
        self.shutdown(false);
    }
}

struct Worker {
    cfg: ApiConfig,
    mcp: Option<AgentProcess>,
    next: u64,
    /// The MCP tools as OpenAI function tools.
    tools: Vec<Value>,
    history: Vec<Value>,
    events: Sender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
        (self.wake)();
    }

    fn run(&mut self, cmds: Receiver<Cmd>) {
        if let Err(e) = self.start_mcp() {
            self.emit(Event::Error(e));
        }
        self.history.push(json!({"role": "system", "content": super::acp::SYSTEM_NOTE}));
        self.emit(Event::Ready { agent: format!("{} · {}", self.cfg.name, self.cfg.model), images: true });
        for cmd in cmds {
            match cmd {
                Cmd::Prompt(parts) => self.turn(parts),
            }
        }
        if let Some(mut m) = self.mcp.take() {
            m.shutdown(false);
        }
    }

    /// One MCP request to the server; its `result`.
    fn mcp_call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let Some(m) = self.mcp.as_ref() else { return Err("no SolidCraft tools in this session".into()) };
        self.next += 1;
        let id = self.next;
        m.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = std::time::Instant::now() + TOOL_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match m.from_agent.recv_timeout(left) {
                Ok(FromAgent::Line(l)) => {
                    let Ok(v) = serde_json::from_str::<Value>(l.trim()) else { continue };
                    if v.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(e) = v.get("error") {
                        return Err(e.get("message").and_then(Value::as_str).unwrap_or("error").to_string());
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(FromAgent::Exited(_)) => return Err("the SolidCraft tools stopped".into()),
                Err(_) => return Err("the SolidCraft tools did not answer".into()),
            }
        }
    }

    fn start_mcp(&mut self) -> Result<(), String> {
        if self.mcp.is_none() {
            return Ok(());
        }
        self.mcp_call(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "solidcraft-agent-chat", "version": env!("CARGO_PKG_VERSION")}}),
        )?;
        if let Some(m) = &self.mcp {
            m.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        }
        let list = self.mcp_call("tools/list", json!({}))?;
        self.tools = openai_tools(&list);
        Ok(())
    }

    fn turn(&mut self, parts: Vec<PromptPart>) {
        self.history.push(user_message(&parts));
        // Keep the system note; drop the oldest messages, never leaving a reply first.
        while self.history.len() > MAX_HISTORY {
            self.history.remove(1);
        }
        while self.history.len() > 2 && self.history.get(1).is_some_and(|m| m["role"] != "user") {
            self.history.remove(1);
        }
        for _ in 0..MAX_REQUESTS_PER_TURN {
            if self.cancel.load(Ordering::Relaxed) {
                self.emit(Event::TurnEnded { reason: "cancelled".into() });
                return;
            }
            let reply = match self.request() {
                Ok(r) => r,
                Err(e) => {
                    self.emit(Event::Error(e));
                    self.emit(Event::TurnEnded { reason: "error".into() });
                    return;
                }
            };
            let msg = reply.pointer("/choices/0/message").cloned().unwrap_or(Value::Null);
            if let Some(text) = msg.get("content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                self.emit(Event::Text { thought: false, text: text.to_string() });
            }
            let calls: Vec<Value> = msg.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default();
            let mut kept = json!({"role": "assistant", "content": msg.get("content").cloned().unwrap_or(Value::Null)});
            if !calls.is_empty() {
                kept["tool_calls"] = Value::Array(calls.clone());
            }
            self.history.push(kept);
            if calls.is_empty() {
                self.emit(Event::TurnEnded { reason: String::new() });
                return;
            }
            for (i, c) in calls.iter().enumerate().take(32) {
                let id = c.get("id").and_then(Value::as_str).map_or_else(|| format!("call-{i}"), str::to_string);
                let name = c.pointer("/function/name").and_then(Value::as_str).unwrap_or("").to_string();
                let args: Value = c
                    .pointer("/function/arguments")
                    .and_then(|a| if let Some(s) = a.as_str() { serde_json::from_str(s).ok() } else { Some(a.clone()) })
                    .unwrap_or(json!({}));
                let title = tool_title(&name, &args);
                let tool_id = format!("{id}-{}", self.next);
                self.emit(Event::Tool {
                    id: tool_id.clone(),
                    title: Some(title),
                    kind: Some(String::new()),
                    status: Some(ToolStatus::InProgress),
                    output: None,
                });
                let (text, failed) = if self.cancel.load(Ordering::Relaxed) {
                    ("cancelled by the user".to_string(), true)
                } else if HIDDEN_TOOLS.contains(&name.as_str())
                    || !self.tools.iter().any(|t| t.pointer("/function/name").and_then(Value::as_str) == Some(name.as_str()))
                {
                    (format!("unknown tool `{name}`"), true)
                } else {
                    match self.mcp_call("tools/call", json!({"name": name, "arguments": args})) {
                        Ok(r) => tool_text(&r),
                        Err(e) => (e, true),
                    }
                };
                let shown: String = text.chars().take(2000).collect();
                self.emit(Event::Tool {
                    id: tool_id,
                    title: None,
                    kind: None,
                    status: Some(if failed { ToolStatus::Failed } else { ToolStatus::Completed }),
                    output: Some(shown),
                });
                self.history.push(json!({"role": "tool", "tool_call_id": id, "content": text}));
            }
        }
        self.emit(Event::Text { thought: false, text: format!("\n({})", crate::i18n::tr("ui.ai_request_limit")) });
        self.emit(Event::TurnEnded { reason: "max_turn_requests".into() });
    }

    fn request(&self) -> Result<Value, String> {
        let mut body = json!({"model": self.cfg.model, "messages": self.history});
        if !self.tools.is_empty() {
            body["tools"] = Value::Array(self.tools.clone());
        }
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if let Some(k) = &self.cfg.key {
            headers.push(("Authorization".into(), format!("Bearer {k}")));
        }
        let req = HttpRequest {
            method: "POST",
            url: format!("{}/chat/completions", self.cfg.base_url),
            headers,
            body: Some(body.to_string().into_bytes()),
            timeout_s: REQUEST_TIMEOUT_S,
        };
        let (status, bytes) = (self.cfg.http)(&req)?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status != 200 {
            return Err(http_error(status, &v));
        }
        Ok(v)
    }
}

/// An endpoint's refusal as people read it.
pub fn http_error(status: u16, body: &Value) -> String {
    let detail =
        body.pointer("/error/message").or_else(|| body.get("error")).and_then(Value::as_str).unwrap_or("").chars().take(300).collect::<String>();
    let what = match status {
        401 | 403 => crate::i18n::tr("ui.ai_http_key"),
        404 => crate::i18n::tr("ui.ai_http_not_found"),
        429 => crate::i18n::tr("ui.ai_http_busy"),
        _ => "",
    };
    format!(
        "HTTP {status}{}{}",
        if what.is_empty() { String::new() } else { format!(": {what}") },
        if detail.is_empty() { String::new() } else { format!(" ({detail})") }
    )
}

/// The message for what the user typed (and the view, as a data URL).
fn user_message(parts: &[PromptPart]) -> Value {
    let images = parts.iter().any(|p| matches!(p, PromptPart::Image { .. }));
    if !images {
        let text: Vec<&str> = parts.iter().filter_map(|p| if let PromptPart::Text(t) = p { Some(t.as_str()) } else { None }).collect();
        return json!({"role": "user", "content": text.join("\n")});
    }
    let content: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            PromptPart::Text(t) => json!({"type": "text", "text": t}),
            PromptPart::Image { mime, base64 } => json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{base64}")}}),
        })
        .collect();
    json!({"role": "user", "content": content})
}

/// MCP `tools/list` as OpenAI function tools (the hidden ones left out).
pub fn openai_tools(list: &Value) -> Vec<Value> {
    list.get("tools")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(128)
                .filter_map(|t| {
                    let name = t.get("name")?.as_str()?;
                    if HIDDEN_TOOLS.contains(&name) {
                        return None;
                    }
                    let params = t.get("inputSchema").cloned().filter(Value::is_object).unwrap_or(json!({"type": "object", "properties": {}}));
                    Some(json!({"type": "function", "function": {
                        "name": name,
                        "description": t.get("description").and_then(Value::as_str).unwrap_or(""),
                        "parameters": params,
                    }}))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A tool call's result as text for the model, and whether it failed.
fn tool_text(r: &Value) -> (String, bool) {
    let failed = r.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let parts: Vec<String> = r
        .get("content")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(16)
                .map(|c| match c.get("type").and_then(Value::as_str) {
                    Some("text") => c.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                    Some("image") => "[a picture of the view]".to_string(),
                    _ => String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut text = parts.join("\n");
    if text.len() > MAX_TOOL_TEXT {
        let mut cut = MAX_TOOL_TEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n…(cut)");
    }
    (text, failed)
}

/// What a tool call card says: the tool and, for `execute` and `batch`, the commands.
fn tool_title(name: &str, args: &Value) -> String {
    match name {
        "execute" => format!("execute: {}", args.get("command").and_then(Value::as_str).unwrap_or("?")),
        "batch" => {
            let ids: Vec<&str> = args
                .get("commands")
                .and_then(Value::as_array)
                .map(|c| c.iter().filter_map(|s| s.get("command").and_then(Value::as_str)).take(4).collect())
                .unwrap_or_default();
            format!("batch: {}", ids.join(", "))
        }
        _ => name.to_string(),
    }
}

/// Ask an endpoint for its models (Test connection): how many, and whether `model` is one.
pub fn test_endpoint(http: &HttpFn, base_url: &str, key: Option<&str>, model: &str) -> Result<String, String> {
    let mut headers = Vec::new();
    if let Some(k) = key {
        if !super::agents::key_safe_for(base_url) {
            return Err(crate::i18n::tr("ui.ai_key_needs_https").to_string());
        }
        headers.push(("Authorization".to_string(), format!("Bearer {k}")));
    }
    let req = HttpRequest { method: "GET", url: format!("{base_url}/models"), headers, body: None, timeout_s: 15 };
    let (status, bytes) = http(&req)?;
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status != 200 {
        return Err(http_error(status, &v));
    }
    let ids: Vec<&str> =
        v.get("data").and_then(Value::as_array).map(|a| a.iter().filter_map(|m| m.get("id").and_then(Value::as_str)).collect()).unwrap_or_default();
    let found = model.is_empty() || ids.contains(&model);
    Ok(crate::i18n::format(if found { "ui.ai_test_ok" } else { "ui.ai_test_no_model" }, &[("count", &ids.len().to_string()), ("model", model)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tools_become_function_tools_without_crash_reports() {
        let list = json!({"tools": [
            {"name": "batch", "description": "Run commands", "inputSchema": {"type": "object", "properties": {"commands": {"type": "array"}}}},
            {"name": "crash_reports", "description": "x", "inputSchema": {"type": "object"}},
            {"name": "measure"},
        ]});
        let t = openai_tools(&list);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0]["function"]["name"], "batch");
        assert_eq!(t[1]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn pictures_go_as_data_urls_and_text_alone_as_a_string() {
        assert_eq!(user_message(&[PromptPart::Text("hi".into())])["content"], "hi");
        let m = user_message(&[PromptPart::Text("hi".into()), PromptPart::Image { mime: "image/png".into(), base64: "AA==".into() }]);
        assert_eq!(m["content"][1]["image_url"]["url"], "data:image/png;base64,AA==");
    }

    /// A scripted endpoint: one answer with a tool call, then a plain answer. The loop runs the
    /// tool (here without SolidCraft tools: an unknown tool fails) and reports the turn's end.
    #[test]
    fn the_tool_loop_runs_calls_and_ends_the_turn() {
        let answers = std::sync::Mutex::new(vec![
            json!({"choices": [{"message": {"role": "assistant", "content": "Done."}}]}),
            json!({"choices": [{"message": {"role": "assistant", "content": "Measuring.", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "measure", "arguments": "{}"}}]}}]}),
        ]);
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let log = seen.clone();
        let http: HttpFn = Arc::new(move |r: &HttpRequest| {
            log.lock().unwrap().push(serde_json::from_slice(r.body.as_deref().unwrap_or(b"null")).unwrap());
            let a = answers.lock().unwrap().pop().unwrap_or(json!({}));
            Ok((200, a.to_string().into_bytes()))
        });
        let cfg = ApiConfig { name: "Test".into(), base_url: "http://localhost:1/v1".into(), model: "m".into(), key: None, http };
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        let mut b = ApiBackend::start(cfg, ctx).unwrap();
        b.prompt(&[PromptPart::Text("measure it".into())]);
        let mut events = Vec::new();
        let start = std::time::Instant::now();
        while !events.iter().any(|e| matches!(e, Event::TurnEnded { .. })) && start.elapsed() < Duration::from_secs(10) {
            events.extend(b.poll());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(events.first(), Some(Event::Ready { .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::Tool { status: Some(ToolStatus::Failed), .. })), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::Text { text, .. } if text == "Done.")), "{events:?}");
        assert!(matches!(events.last(), Some(Event::TurnEnded { reason }) if reason.is_empty()), "{events:?}");
        // The second request carried the tool's answer.
        let sent = seen.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["messages"].as_array().unwrap().last().unwrap()["role"], "tool");
        b.shutdown(true);
    }

    #[test]
    fn keys_never_go_over_plain_http_to_another_computer() {
        let http: HttpFn = Arc::new(|_: &HttpRequest| Ok((200, b"{}".to_vec())));
        assert!(test_endpoint(&http, "http://10.0.0.2:11434/v1", Some("k"), "m").is_err());
        let cfg = ApiConfig { name: "x".into(), base_url: "http://10.0.0.2/v1".into(), model: "m".into(), key: Some("k".into()), http };
        let ctx = StartContext { cwd: std::env::temp_dir(), bridge: None, tools: None, wake: Box::new(|| {}) };
        assert!(ApiBackend::start(cfg, ctx).is_err());
    }
}
