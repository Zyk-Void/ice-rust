//! `mcpScript`: run model-written JavaScript that searches, describes and calls
//! MCP tools, for multi-call work in one step. Ports pi-mcp-adapter
//! `mcp-code.ts` and `mcp-script-worker.mjs` on `boa_engine` (D9). Each script
//! gets a fresh context on its own thread; `tools.*` calls cross to the async
//! side through a thread-local bridge and use the `mcp` proxy's lookup, lazy
//! connect, approval and output guard. Like PMA's, this is a convenience layer
//! for trusted agent code, not an isolation boundary.
use super::*;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::object::builtins::JsPromise;
use boa_engine::{js_string, Context, JsArgs, JsResult, JsValue, NativeFunction, Source};
use std::cell::RefCell;

pub(super) const NAME: &str = "mcpScript";
const DEADLINE: Duration = Duration::from_secs(30);
/// Iterations of any one loop; stops `while (true) {}` without a wall clock.
const LOOP_LIMIT: u64 = 10_000_000;
const RECURSION_LIMIT: usize = 400;
const MAX_CODE: usize = 64 * 1024;
const MAX_OUTPUT_LINES: usize = 1_000;
const STACK: usize = 16 * 1024 * 1024;

const PRELUDE: &str = r#"
const __call = (op, payload) => JSON.parse(__host(op, JSON.stringify(payload === undefined ? {} : payload)));
const tools = Object.freeze({
  search: async (q) => __call("search", typeof q === "string" ? { query: q } : q),
  describe: async (q) => __call("describe", typeof q === "string" ? { path: q } : q),
  call: async (path, args) => __call("call", { path, args: args === undefined ? {} : args }),
});
const __text = (values) => values.map((v) => typeof v === "string" ? v : JSON.stringify(v)).join(" ");
const emit = (value) => { __host("emit", JSON.stringify(value === undefined ? null : value)); };
const console = Object.freeze({
  log: (...v) => { __host("log", __text(v)); },
  info: (...v) => { __host("log", __text(v)); },
  warn: (...v) => { __host("log", "warn: " + __text(v)); },
  error: (...v) => { __host("log", "error: " + __text(v)); },
});
"#;

/// A `tools.*` request from the script thread, answered with JSON.
struct HostCall {
    op: String,
    payload: Value,
    reply: std::sync::mpsc::Sender<Value>,
}

struct Bridge {
    calls: mpsc::UnboundedSender<HostCall>,
    output: Vec<String>,
    deadline: std::time::Instant,
}
thread_local! {
    static BRIDGE: RefCell<Option<Bridge>> = const { RefCell::new(None) };
}

fn host(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let op = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let payload = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let reply = BRIDGE.with(|bridge| {
        let mut bridge = bridge.borrow_mut();
        let Some(bridge) = bridge.as_mut() else {
            return Value::Null;
        };
        match op.as_str() {
            "emit" | "log" => {
                if bridge.output.len() < MAX_OUTPUT_LINES {
                    bridge.output.push(payload);
                }
                Value::Null
            }
            _ => {
                let failed =
                    |message: &str| json!({"ok":false,"error":{"code":"script_error","message":message}});
                let remaining = bridge
                    .deadline
                    .saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return failed("mcpScript deadline reached");
                }
                let (reply, answer) = std::sync::mpsc::channel();
                let call = HostCall {
                    op,
                    payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                    reply,
                };
                if bridge.calls.send(call).is_err() {
                    return failed("mcpScript was stopped");
                }
                answer
                    .recv_timeout(remaining)
                    .unwrap_or_else(|_| failed("mcpScript deadline reached"))
            }
        }
    });
    Ok(js_string!(reply.to_string()).into())
}

/// Runs on the script's own thread: returns the script's value and its output.
fn run(
    code: &str,
    calls: mpsc::UnboundedSender<HostCall>,
    deadline: std::time::Instant,
) -> (Result<Value, String>, Vec<String>) {
    BRIDGE.with(|bridge| {
        *bridge.borrow_mut() = Some(Bridge {
            calls,
            output: Vec::new(),
            deadline,
        })
    });
    let result = (|| {
        let mut context = Context::default();
        let limits = context.runtime_limits_mut();
        limits.set_loop_iteration_limit(LOOP_LIMIT);
        limits.set_recursion_limit(RECURSION_LIMIT);
        context
            .register_global_callable(js_string!("__host"), 2, NativeFunction::from_fn_ptr(host))
            .map_err(|e| e.to_string())?;
        context
            .eval(Source::from_bytes(PRELUDE))
            .map_err(|e| e.to_string())?;
        let promise = context
            .eval(Source::from_bytes(&format!(
                "(async () => {{\n{code}\n}})()"
            )))
            .map_err(|e| e.to_string())?;
        context.run_jobs().map_err(|e| e.to_string())?;
        let promise = promise
            .as_object()
            .ok_or("script did not produce a promise")?;
        match JsPromise::from_object(promise)
            .map_err(|e| e.to_string())?
            .state()
        {
            PromiseState::Fulfilled(value) => Ok(value
                .to_json(&mut context)
                .map_err(|e| e.to_string())?
                .unwrap_or(Value::Null)),
            PromiseState::Rejected(error) => Err(error
                .to_string(&mut context)
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_else(|_| "script threw".into())),
            PromiseState::Pending => Err("script finished with an unresolved promise".into()),
        }
    })();
    let output = BRIDGE.with(|bridge| {
        bridge
            .borrow_mut()
            .take()
            .map(|b| b.output)
            .unwrap_or_default()
    });
    (result, output)
}

pub(super) struct ScriptTool {
    manager: std::sync::Weak<manager::Inner>,
    schema: Tool,
}
impl ScriptTool {
    pub(super) fn new(manager: std::sync::Weak<manager::Inner>) -> Self {
        let parameters = json!({
            "type": "object",
            "properties": {
                "code": {"type":"string","description":"Body of an async JavaScript function. Use `await tools.search({ query, server?, limit?, offset? })`, `await tools.describe({ path })`, `await tools.call(path, args)` (returns { ok: true, data } or { ok: false, error }), `emit(value)` and `console.log`. `return` the final value."}
            },
            "required": ["code"],
            "additionalProperties": false
        });
        Self {
            manager,
            schema: Tool {
                name: NAME.into(),
                description: "Run JavaScript that discovers, calls, loops over, filters or chains several MCP tool calls, then returns one result. For a single MCP call or search, use `mcp` instead. Calls run one at a time with the usual approvals; the script stops after 30 seconds.".into(),
                parameters: serde_json::from_value(parameters).expect("static schema"),
                constrained_sampling: None,
            },
        }
    }
}

#[async_trait]
impl AgentTool for ScriptTool {
    fn schema(&self) -> &Tool {
        &self.schema
    }
    fn label(&self) -> &str {
        "MCP script"
    }
    async fn execute(
        &self,
        call_id: &str,
        params: Value,
        signal: CancellationToken,
        _: Arc<dyn Fn(ToolResultPartial) + Send + Sync>,
    ) -> Result<AgentToolResult, AgentError> {
        let manager = self
            .manager
            .upgrade()
            .map(McpManager)
            .ok_or_else(|| AgentError::Tool("MCP session ended".into()))?;
        let code = params["code"]
            .as_str()
            .ok_or_else(|| AgentError::Tool("`code` is required".into()))?
            .to_owned();
        if code.len() > MAX_CODE {
            return Err(AgentError::Tool("mcpScript code exceeds 64 KiB".into()));
        }
        let (calls, mut requests) = mpsc::unbounded_channel();
        let (done, mut finished) = oneshot::channel();
        let deadline = std::time::Instant::now() + DEADLINE;
        std::thread::Builder::new()
            .name("mcp-script".into())
            .stack_size(STACK)
            .spawn(move || {
                let _ = done.send(run(&code, calls, deadline));
            })
            .map_err(|e| AgentError::Tool(format!("cannot start mcpScript: {e}")))?;
        let mut trace = Vec::new();
        let stop = tokio::time::Instant::from_std(deadline);
        // A script that overruns is abandoned: its calls fail from here on, and
        // its loop and recursion limits end the thread.
        let (result, output) = loop {
            tokio::select! {
                _ = signal.cancelled() => return Err(AgentError::Tool("mcpScript cancelled".into())),
                _ = tokio::time::sleep_until(stop) => {
                    return Err(AgentError::Tool("mcpScript timed out after 30 seconds".into()))
                }
                outcome = &mut finished => match outcome {
                    Ok(outcome) => break outcome,
                    Err(_) => return Err(AgentError::Tool("mcpScript stopped unexpectedly".into())),
                },
                Some(call) = requests.recv() => {
                    let started = std::time::Instant::now();
                    let reply = tokio::select! {
                        _ = signal.cancelled() => return Err(AgentError::Tool("mcpScript cancelled".into())),
                        _ = tokio::time::sleep_until(stop) => {
                            return Err(AgentError::Tool("mcpScript timed out after 30 seconds".into()))
                        }
                        reply = dispatch(&manager, &call, call_id, &signal) => reply,
                    };
                    trace.push(json!({
                        "op": call.op,
                        "target": call.payload.get("path").or_else(|| call.payload.get("query")),
                        "ok": reply.get("ok").and_then(Value::as_bool).unwrap_or(reply.get("error").is_none()),
                        "ms": started.elapsed().as_millis() as u64,
                    }));
                    let _ = call.reply.send(reply);
                }
            }
        };
        let result = result.map_err(|e| AgentError::Tool(format!("mcpScript failed: {e}")))?;
        let mut text = output.join("\n");
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&serde_json::to_string_pretty(&result).unwrap_or_default());
        Ok(AgentToolResult {
            content: vec![TextContentOrImage::text(
                manager
                    .output_guard()
                    .map_err(AgentError::Tool)?
                    .bound_text(text),
            )],
            details: json!({"calls":trace}),
            ..Default::default()
        })
    }
}

async fn dispatch(
    manager: &McpManager,
    call: &HostCall,
    call_id: &str,
    signal: &CancellationToken,
) -> Value {
    let servers = manager.proxy_view();
    let text_of = |result: &AgentToolResult| {
        result
            .content
            .iter()
            .filter_map(|block| match block {
                TextContentOrImage::Text(text) => Some(text.text.as_str()),
                TextContentOrImage::Image(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    match call.op.as_str() {
        "search" => {
            let query = call.payload["query"].as_str().unwrap_or_default();
            let limit = call.payload["limit"].as_u64().unwrap_or(12).clamp(1, 100) as usize;
            let offset = call.payload["offset"].as_u64().unwrap_or(0) as usize;
            let hits = proxy::ranked(&servers, query, call.payload["server"].as_str());
            let items: Vec<Value> = hits
                .iter()
                .skip(offset)
                .take(limit)
                .map(|(_, server, entry)| {
                    json!({"path":entry.name,"server":server.name,"description":entry.description})
                })
                .collect();
            let next = offset + items.len();
            json!({"items":items,"total":hits.len(),"nextOffset":(next < hits.len()).then_some(next)})
        }
        "describe" => {
            let path = call.payload["path"].as_str().unwrap_or_default();
            match proxy::resolve(&servers, path, None) {
                proxy::Found::One(server, entry) => json!({
                    "path": entry.name,
                    "server": server.name,
                    "description": entry.description,
                    "inputSchema": entry.schema,
                }),
                _ => {
                    json!({"error":{"code":"tool_not_found","message":format!("No MCP tool matches {path:?}")}})
                }
            }
        }
        "call" => {
            let path = call.payload["path"].as_str().unwrap_or_default();
            let params = json!({"args": call.payload["args"]});
            match proxy::call(
                manager,
                &servers,
                path,
                None,
                &params,
                call_id,
                signal.clone(),
                Arc::new(|_| {}),
            )
            .await
            {
                Ok(result) if result.details.get("error").is_some() => json!({
                    "ok": false,
                    "error": {"code": result.details["error"], "message": text_of(&result)},
                }),
                Ok(result) => {
                    let data = match result.details.get("structuredContent") {
                        Some(structured) => structured.clone(),
                        None => json!(text_of(&result)),
                    };
                    json!({"ok":true,"data":data})
                }
                Err(error) => {
                    json!({"ok":false,"error":{"code":"call_failed","message":error.to_string()}})
                }
            }
        }
        other => {
            json!({"error":{"code":"unknown_operation","message":format!("unknown operation {other}")}})
        }
    }
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
