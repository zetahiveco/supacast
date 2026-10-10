use crate::{calendar, clipboard_hist, notes, todos};
use base64::Engine;
use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::sync::mpsc::Sender;

const TRANSCRIBE_MODEL: &str = "gpt-4o-mini-transcribe";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMsg {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

/// Streaming events sent to the UI while the agent works.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Sent immediately when the request is kicked off.
    Start,
    Thinking { text: String },
    Delta { text: String },
    Tool { name: String },
    Done { text: String },
    Error { message: String },
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .expect("failed to build http client")
    })
}

fn system_prompt() -> String {
    let now = Local::now();
    format!(
        "You are Supacast, the AI inside the Supacast launcher app. \
You help the user manage todos, reminders, short notes and their local calendar. \
Be concise and practical — answers are shown in a small launcher window.

Current local datetime: {now} ({weekday})
When creating todos, reminders or calendar events, convert natural times like \
\"5pm\", \"tomorrow morning\" into ISO datetimes in the user's local time (YYYY-MM-DDTHH:MM). \
For 'note that ...' / 'save a note ...' requests, use the add_note tool (notes have no due time). \
For anything time-sensitive (news, weather, prices, scores, recent releases or docs), \
use the web_search tool instead of relying on your training data, and mention the sources.",
        now = now.format("%Y-%m-%d %H:%M"),
        weekday = now.format("%A"),
    )
}

fn agent_tools() -> Value {
    // Chat Completions format: each tool is { "type": "function", "function": {...} }.
    json!([
        {
            "type": "function",
            "function": {
                "name": "add_todo",
                "description": "Add a todo or reminder. Todos with a due time fire a system notification when due. Also use this for 'remind me to ...'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "What to do, e.g. 'Send email to Adnan'" },
                        "due": { "type": "string", "description": "Optional due time as ISO local datetime, e.g. 2026-10-08T17:00" }
                    },
                    "required": ["text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_todos",
                "description": "List the user's todos for today, tomorrow or all.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "scope": { "type": "string", "enum": ["today", "tomorrow", "all"] }
                    },
                    "required": ["scope"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "complete_todo",
                "description": "Mark a todo as done by id (ids come from list_todos).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "add_note",
                "description": "Save a short note for later (no due time). Use for 'note that ...', 'save a note ...', jotting something down.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "The note content, e.g. 'WiFi password at office is supacast-2026'" }
                    },
                    "required": ["text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_notes",
                "description": "List saved notes, newest first. Optionally filter by a search query.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Optional substring to filter notes by" }
                    },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "delete_note",
                "description": "Delete a note by id (ids come from list_notes).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "add_calendar_event",
                "description": "Create an event in the system calendar (Calendar.app on macOS, Outlook on Windows).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "start": { "type": "string", "description": "ISO local datetime, e.g. 2026-10-08T17:00" },
                        "duration_minutes": { "type": "integer", "default": 30 }
                    },
                    "required": ["title", "start"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_calendar_events",
                "description": "List system calendar events for today or tomorrow.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "scope": { "type": "string", "enum": ["today", "tomorrow"] }
                    },
                    "required": ["scope"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "copy_to_clipboard",
                "description": "Copy text to the user's clipboard (useful for long results, emails, lists).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" }
                    },
                    "required": ["text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web for current information the model can't know: news, weather, prices, sports scores, release notes, docs updated after the knowledge cutoff. Returns a concise answer with source URLs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query, e.g. 'rust 1.90 release notes'" }
                    },
                    "required": ["query"]
                }
            }
        }
    ])
}

/// Execute one agent tool call. `web_search` needs the HTTP client and
/// credentials for the Responses API, so they are threaded through here.
fn execute_tool(
    http: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    name: &str,
    args: &Value,
) -> String {
    let result: Result<Value, String> = (|| {
        match name {
            "add_todo" => {
                let text = args["text"].as_str().ok_or("missing text")?;
                let due = args["due"].as_str();
                let todo = todos::add(text, due)?;
                Ok(json!({ "ok": true, "todo": todo }))
            }
            "web_search" => {
                let query = args["query"].as_str().ok_or("missing query")?;
                Ok(json!({ "result": web_search(http, base_url, api_key, model, query)? }))
            }
            "list_todos" => {
                let scope = args["scope"].as_str().unwrap_or("today");
                Ok(json!({ "todos": todos::list(scope) }))
            }
            "complete_todo" => {
                let id = args["id"].as_str().ok_or("missing id")?;
                todos::complete(id)?;
                Ok(json!({ "ok": true }))
            }
            "add_note" => {
                let text = args["text"].as_str().ok_or("missing text")?;
                let note = notes::add(text)?;
                Ok(json!({ "ok": true, "note": note }))
            }
            "list_notes" => {
                let query = args["query"].as_str();
                Ok(json!({ "notes": notes::list(query) }))
            }
            "delete_note" => {
                let id = args["id"].as_str().ok_or("missing id")?;
                notes::delete(id)?;
                Ok(json!({ "ok": true }))
            }
            "add_calendar_event" => {
                let title = args["title"].as_str().ok_or("missing title")?;
                let start = args["start"].as_str().ok_or("missing start")?;
                let duration = args["duration_minutes"].as_i64().unwrap_or(30);
                let event = calendar::add_event(title, start, duration)?;
                Ok(json!({ "ok": true, "event": event }))
            }
            "list_calendar_events" => {
                let scope = args["scope"].as_str().unwrap_or("today");
                let events = calendar::list_events(scope)?;
                Ok(json!({ "events": events }))
            }
            "copy_to_clipboard" => {
                let text = args["text"].as_str().ok_or("missing text")?;
                clipboard_hist::copy_to_clipboard(text)?;
                Ok(json!({ "ok": true }))
            }
            other => Err(format!("unknown tool: {other}")),
        }
    })();

    match result {
        Ok(v) => v.to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
}

/// One web search via the OpenAI Responses API built-in `web_search` tool.
/// The built-in tool only exists on /responses, so this is a self-contained
/// non-streaming round-trip: the searched answer plus source URLs is
/// returned as the agent tool result.
fn web_search(
    http: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    query: &str,
) -> Result<String, String> {
    let mut body = json!({
        "model": model,
        "input": query,
        "tools": [{ "type": "web_search", "search_context_size": "low" }],
    });
    // Same reasoning heuristic as chat_body: keep the search round-trip fast.
    if model.starts_with("gpt-5") || model.starts_with("o") {
        body["reasoning"] = json!({ "effort": "low" });
    }
    let resp = http
        .post(format!("{}/responses", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    let v: Value = resp.json().map_err(|e| format!("bad response: {e}"))?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("web search error ({status}): {msg}"));
    }

    // Extract the answer text and citation URLs from the output items.
    let mut text = String::new();
    let mut sources: Vec<(String, String)> = Vec::new(); // (url, title)
    if let Some(output) = v["output"].as_array() {
        for item in output {
            if item["type"] != "message" {
                continue;
            }
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if part["type"] != "output_text" {
                        continue;
                    }
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(part["text"].as_str().unwrap_or(""));
                    if let Some(anns) = part["annotations"].as_array() {
                        for a in anns {
                            if a["type"] == "url_citation" {
                                let url = a["url"].as_str().unwrap_or("");
                                let title = a["title"].as_str().unwrap_or("");
                                if !url.is_empty() && !sources.iter().any(|(u, _)| u == url) {
                                    sources.push((url.to_string(), title.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if text.trim().is_empty() {
        return Err("no results".into());
    }
    if !sources.is_empty() {
        text.push_str("\n\nSources:");
        for (url, title) in sources.iter().take(5) {
            text.push_str(&format!("\n- {title} — {url}"));
        }
    }
    Ok(text)
}

fn chat_body(model: &str, messages: &[Value]) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "tools": agent_tools(),
        "stream": true,
        "stream_options": { "include_usage": false },
    });
    // Reasoning models: low effort = much faster time-to-first-token,
    // which matters a lot in a launcher UX.
    if model.starts_with("gpt-5") || model.starts_with("o") {
        body["reasoning_effort"] = json!("low");
    }
    body
}

/// One streaming /chat/completions round. Streams reasoning + content deltas
/// through `tx`; returns the full text and any tool calls requested.
fn chat_stream_round(
    http: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
    model: &str,
    messages: &[Value],
    tx: &Sender<StreamEvent>,
) -> Result<(String, Vec<(String, String, String)>), String> {
    let resp = http
        .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .json(&chat_body(model, messages))
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("API error ({status}): {}", truncate(&body, 400)));
    }

    let mut text = String::new();
    // Accumulate streamed tool calls by index: (id, name, arguments).
    let mut calls: Vec<(String, String, String)> = Vec::new();
    // Tool-call names are announced the moment they start streaming in.
    let mut announced: Vec<bool> = Vec::new();

    let reader = BufReader::new(resp);
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data.trim() == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let Some(delta) = v["choices"][0]["delta"].as_object() else {
            continue;
        };

        // Reasoning ("thinking") tokens, when the model/provider exposes them.
        if let Some(think) = delta
            .get("reasoning")
            .or_else(|| delta.get("reasoning_content"))
            .and_then(|t| t.as_str())
        {
            if !think.is_empty() {
                let _ = tx.send(StreamEvent::Thinking { text: think.to_string() });
            }
        }

        if let Some(piece) = delta.get("content").and_then(|t| t.as_str()) {
            if !piece.is_empty() {
                text.push_str(piece);
                let _ = tx.send(StreamEvent::Delta { text: piece.to_string() });
            }
        }

        if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in tcs {
                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                while calls.len() <= idx {
                    calls.push((String::new(), String::new(), String::new()));
                    announced.push(false);
                }
                if let Some(id) = tc["id"].as_str() {
                    calls[idx].0 = id.to_string();
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    calls[idx].1.push_str(name);
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    calls[idx].2.push_str(args);
                }
                // Announce as soon as we know which tool is being called,
                // so the UI shows it while the round is still streaming.
                if !announced[idx] && !calls[idx].1.is_empty() {
                    announced[idx] = true;
                    let _ = tx.send(StreamEvent::Tool { name: calls[idx].1.clone() });
                }
            }
        }
    }

    Ok((text, calls))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

/// Run the Supacast agent with streaming output. Executes tool calls until
/// the model produces a final answer (streamed as `Delta` events).
/// Events go through a plain mpsc channel — the UI polls it every frame.
pub fn run_agent_stream(
    api_key: &str,
    base_url: &str,
    model: &str,
    history: &[ChatMsg],
    message: &str,
    tx: &Sender<StreamEvent>,
) -> Result<(), String> {
    if api_key.trim().is_empty() {
        let _ = tx.send(StreamEvent::Error {
            message: "No API key configured. Open Supacast Settings and add your key."
                .to_string(),
        });
        return Ok(());
    }

    let http = client();

    // Tell the UI right away that the request is underway.
    let _ = tx.send(StreamEvent::Start);

    // system + prior turns + the new user message
    let mut messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": system_prompt(),
    })];
    for msg in history {
        if msg.content.trim().is_empty() {
            continue;
        }
        messages.push(json!({
            "role": if msg.role == "assistant" { "assistant" } else { "user" },
            "content": msg.content,
        }));
    }
    messages.push(json!({ "role": "user", "content": message }));

    for _round in 0..5 {
        let (text, calls) = chat_stream_round(&http, base_url, api_key, model, &messages, tx)?;

        if calls.is_empty() {
            let _ = tx.send(StreamEvent::Done { text: text.clone() });
            return Ok(());
        }

        // Record the assistant's tool-call turn, execute, and loop.
        let call_json: Vec<Value> = calls
            .iter()
            .map(|(id, name, args)| {
                json!({
                    "id": id,
                    "type": "function",
                    "function": { "name": name, "arguments": args }
                })
            })
            .collect();
        messages.push(json!({
            "role": "assistant",
            "content": if text.is_empty() { Value::Null } else { json!(text) },
            "tool_calls": call_json,
        }));

        for (id, name, args) in &calls {
            let parsed: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
            let result = execute_tool(&http, base_url, api_key, model, name, &parsed);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": result,
            }));
        }
    }

    let _ = tx.send(StreamEvent::Error {
        message: "agent did not settle on a final answer".to_string(),
    });
    Ok(())
}

/// Non-streaming convenience used by the dictate ring: collects the stream
/// into a single final answer.
pub fn run_agent(
    api_key: &str,
    base_url: &str,
    model: &str,
    message: &str,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    run_agent_stream(api_key, base_url, model, &[], message, &tx)?;

    let mut final_text = String::new();
    while let Ok(event) = rx.recv() {
        match event {
            StreamEvent::Done { text } => {
                final_text = text;
                break;
            }
            StreamEvent::Error { message } => return Err(message),
            _ => {}
        }
    }
    Ok(final_text)
}

// ---------------------------------------------------------------------------
// Transcription (voice dictate)
// ---------------------------------------------------------------------------

/// Transcribe a recorded audio clip (wav/m4a/webm/...) via the API endpoint.
/// The native recorder produces WAV; older recordings from the Tauri app may
/// be webm/mp4. Audio arrives as base64 to match the original IPC contract.
pub fn transcribe(api_key: &str, base_url: &str, audio_b64: &str, mime: &str) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err(
            "No API key configured. Open Supacast Settings and add your key.".to_string(),
        );
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_b64.trim())
        .map_err(|e| format!("bad audio data: {e}"))?;

    let ext = match mime {
        m if m.contains("wav") => "wav",
        m if m.contains("mp4") || m.contains("aac") => "m4a",
        m if m.contains("ogg") => "ogg",
        m if m.contains("webm") => "webm",
        _ => "wav",
    };

    let part = reqwest::blocking::multipart::Part::bytes(bytes)
        .file_name(format!("recording.{ext}"))
        .mime_str(mime)
        .map_err(|e| e.to_string())?;

    let form = reqwest::blocking::multipart::Form::new()
        .part("file", part)
        .text("model", TRANSCRIBE_MODEL);

    let resp = client()
        .post(format!(
            "{}/audio/transcriptions",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    let v: Value = resp.json().map_err(|e| format!("bad response: {e}"))?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(format!("transcription error ({status}): {msg}"));
    }
    Ok(v["text"].as_str().unwrap_or("").to_string())
}
