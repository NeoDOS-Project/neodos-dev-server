use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::Value;

pub type ToolHandler = Box<dyn Fn(&HashMap<String, Value>) -> String + Send + Sync>;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub handler: ToolHandler,
}

pub struct McpServer {
    name: String,
    version: String,
    tools: Vec<ToolSpec>,
    initialized: bool,
}

impl McpServer {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            tools: Vec::new(),
            initialized: false,
        }
    }

    pub fn register_tool(&mut self, spec: ToolSpec) {
        self.tools.push(spec);
    }

    pub fn register_all_tools(&mut self, tools: &crate::tools::McpTools) {
        for tool in tools.all_tools() {
            self.register_tool(tool);
        }
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn run_stdio(&mut self) {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let stdout = io::stdout();
        let mut writer = stdout.lock();

        let mut buffer = String::new();

        loop {
            buffer.clear();
            match reader.read_line(&mut buffer) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[MCP] read error: {e}");
                    break;
                }
            }

            let line = buffer.trim();
            if line.is_empty() {
                continue;
            }

            let response = self.handle_line(line);
            if let Some(resp) = response {
                writeln!(writer, "{resp}").ok();
                writer.flush().ok();
            }
        }
    }

    fn handle_line(&mut self, line: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                return Some(make_error(None, -32700, &format!("Parse error: {e}")));
            }
        };

        let method = msg.get("method")?.as_str()?;
        let params_map: HashMap<String, Value> = msg
            .get("params")
            .and_then(|p| p.as_object())
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        let id = msg.get("id").cloned();

        let result: Result<Value, String> = match method {
            "initialize" => self.handle_initialize(&params_map),
            "notifications/initialized" | "initialized" => {
                self.initialized = true;
                Ok(serde_json::json!({}))
            }
            "ping" => Ok(serde_json::json!({})),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tools_call(&params_map),
            _ => {
                if id.is_some() {
                    return Some(make_error(id, -32601, &format!("Method not found: {method}")));
                }
                return None;
            }
        };

        match result {
            Ok(val) => {
                if let Some(id_val) = id {
                    Some(make_response(id_val, &val))
                } else {
                    None
                }
            }
            Err(err_msg) => {
                Some(make_error(id, -32603, &err_msg))
            }
        }
    }

    fn handle_initialize(&mut self, _params: &HashMap<String, Value>) -> Result<Value, String> {
        let server_caps = serde_json::json!({
            "tools": {},
            "resources": {},
            "prompts": {},
        });
        Ok(serde_json::json!({
            "protocolVersion": "1.0",
            "capabilities": server_caps,
            "serverInfo": {
                "name": self.name,
                "version": self.version,
            }
        }))
    }

    fn handle_tools_list(&self) -> Result<Value, String> {
        let tools: Vec<Value> = self.tools.iter().map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        }).collect();
        Ok(serde_json::json!({ "tools": tools }))
    }

    fn handle_tools_call(&self, params: &HashMap<String, Value>) -> Result<Value, String> {
        let name = params.get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing tool name")?;
        let args_map: HashMap<String, Value> = params.get("arguments")
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let tool = self.tools.iter().find(|t| t.name == name)
            .ok_or_else(|| format!("Unknown tool: {name}"))?;

        let result = (tool.handler)(&args_map);
        Ok(serde_json::json!({
            "content": [{"type": "text", "text": result}]
        }))
    }
}

fn make_response(id: Value, result: &Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }).to_string()
}

fn make_error(id: Option<Value>, code: i32, message: &str) -> String {
    let json_id = id.unwrap_or(Value::Null);
    let msg = message.to_string();
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": json_id,
        "error": {
            "code": code,
            "message": msg,
        }
    }).to_string()
}
