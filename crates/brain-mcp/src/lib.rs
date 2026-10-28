//! brain-mcp: MCP server over stdio (JSON-RPC 2.0, one message per line). Hand-written JSON.
//! The crate only knows the protocol; tools are registered by the `brain` binary.

pub mod json;

pub use json::Json;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

/// MCP protocol versions this server accepts; the one the client asks for is returned if it is listed here.
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Runs the tool with its `arguments`.
pub type Handler = Box<dyn Fn(&Json) -> Result<String, String>>;

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of `arguments`.
    pub input_schema: Json,
    /// JSON Schema of the result. If present, the output of `run` is JSON and is also sent in `structuredContent`.
    pub output_schema: Option<Json>,
    /// `Ok(text)` or `Err(text)`; a tool error is sent as `isError: true`, not as a JSON-RPC error.
    pub run: Handler,
}

/// Read-only resource (`resources/read`), e.g. `brain://spec`.
pub struct Resource {
    pub uri: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub mime: &'static str,
    pub read: Box<dyn Fn() -> Result<String, String>>,
}

/// Guided prompt (`prompts/get`): returns the user message text with the arguments applied.
pub struct Prompt {
    pub name: &'static str,
    pub description: &'static str,
    /// (name, description, required)
    pub args: Vec<(&'static str, &'static str, bool)>,
    pub get: Handler,
}

/// Shared output: responses and async notifications (from other threads) never interleave mid-message.
#[derive(Clone)]
pub struct Notifier {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Notifier {
    pub fn new(w: Box<dyn Write + Send>) -> Notifier {
        Notifier { out: Arc::new(Mutex::new(w)) }
    }
    pub fn send(&self, msg: &Json) {
        if let Ok(mut o) = self.out.lock() {
            let _ = writeln!(o, "{}", msg.stringify());
            let _ = o.flush();
        }
    }
    /// `notifications/message` (MCP logging): `level` = debug|info|notice|warning|error.
    pub fn log(&self, level: &str, data: Json) {
        self.send(&Json::obj([
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str("notifications/message")),
            ("params", Json::obj([("level", Json::str(level)), ("logger", Json::str("brain")), ("data", data)])),
        ]));
    }
}

pub struct Server {
    pub name: &'static str,
    pub version: &'static str,
    pub instructions: &'static str,
    pub tools: Vec<Tool>,
    pub resources: Vec<Resource>,
    pub prompts: Vec<Prompt>,
}

impl Server {
    /// Serves JSON-RPC lines until EOF, writing through `out` (shared with notifications).
    pub fn run<R: BufRead>(&self, input: R, out: &Notifier) -> io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(resp) = self.handle_line(&line) {
                out.send(&resp);
            }
        }
        Ok(())
    }

    /// One input line → response (None for notifications).
    pub fn handle_line(&self, line: &str) -> Option<Json> {
        let msg = match Json::parse(line) {
            Ok(m) => m,
            Err(e) => return Some(error(Json::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(Json::as_str) else {
            // Client responses or garbage without a method: nothing to answer unless they carry an id.
            return id.map(|id| error(id, -32600, "invalid request: missing `method`"));
        };
        let id = id?; // notification (e.g. notifications/initialized): no response
        let params = msg.get("params").cloned().unwrap_or(Json::Obj(vec![]));
        Some(match self.dispatch(method, &params) {
            Ok(result) => Json::obj([("jsonrpc", Json::str("2.0")), ("id", id), ("result", result)]),
            Err((code, m)) => error(id, code, &m),
        })
    }

    fn dispatch(&self, method: &str, params: &Json) -> Result<Json, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Json::as_str).unwrap_or("");
                let v = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
                Ok(Json::obj([
                    ("protocolVersion", Json::str(v)),
                    (
                        "capabilities",
                        Json::obj([
                            ("tools", Json::obj([("listChanged", Json::Bool(false))])),
                            ("resources", Json::obj([("listChanged", Json::Bool(false)), ("subscribe", Json::Bool(false))])),
                            ("prompts", Json::obj([("listChanged", Json::Bool(false))])),
                            ("logging", Json::Obj(vec![])),
                        ]),
                    ),
                    ("serverInfo", Json::obj([("name", Json::str(self.name)), ("version", Json::str(self.version))])),
                    ("instructions", Json::str(self.instructions)),
                ]))
            }
            "ping" | "logging/setLevel" => Ok(Json::Obj(vec![])),
            "tools/list" => Ok(Json::obj([(
                "tools",
                Json::Arr(
                    self.tools
                        .iter()
                        .map(|t| {
                            let mut kv = vec![
                                ("name".to_string(), Json::str(t.name)),
                                ("description".into(), Json::str(t.description)),
                                ("inputSchema".into(), t.input_schema.clone()),
                            ];
                            if let Some(o) = &t.output_schema {
                                kv.push(("outputSchema".into(), o.clone()));
                            }
                            Json::Obj(kv)
                        })
                        .collect(),
                ),
            )])),
            "tools/call" => {
                let name = params.get("name").and_then(Json::as_str).ok_or((-32602, "missing `name`".to_string()))?;
                let tool = self.tools.iter().find(|t| t.name == name).ok_or((-32602, format!("unknown tool `{name}`")))?;
                let args = params.get("arguments").cloned().unwrap_or(Json::Obj(vec![]));
                let (text, is_error) = match (tool.run)(&args) {
                    Ok(t) => (t, false),
                    Err(t) => (t, true),
                };
                let mut kv = vec![
                    ("content".to_string(), Json::Arr(vec![Json::obj([("type", Json::str("text")), ("text", Json::str(&text))])])),
                    ("isError".into(), Json::Bool(is_error)),
                ];
                if tool.output_schema.is_some() && !is_error
                    && let Ok(v @ Json::Obj(_)) = Json::parse(&text) {
                        kv.push(("structuredContent".into(), v));
                    }
                Ok(Json::Obj(kv))
            }
            "resources/list" => Ok(Json::obj([(
                "resources",
                Json::Arr(
                    self.resources
                        .iter()
                        .map(|r| {
                            Json::obj([
                                ("uri", Json::str(r.uri)),
                                ("name", Json::str(r.name)),
                                ("description", Json::str(r.description)),
                                ("mimeType", Json::str(r.mime)),
                            ])
                        })
                        .collect(),
                ),
            )])),
            "resources/templates/list" => Ok(Json::obj([("resourceTemplates", Json::Arr(vec![]))])),
            "resources/read" => {
                let uri = params.get("uri").and_then(Json::as_str).ok_or((-32602, "missing `uri`".to_string()))?;
                let r = self.resources.iter().find(|r| r.uri == uri).ok_or((-32002, format!("unknown resource `{uri}`")))?;
                let text = (r.read)().map_err(|e| (-32603, e))?;
                Ok(Json::obj([(
                    "contents",
                    Json::Arr(vec![Json::obj([("uri", Json::str(r.uri)), ("mimeType", Json::str(r.mime)), ("text", Json::str(text))])]),
                )]))
            }
            "prompts/list" => Ok(Json::obj([(
                "prompts",
                Json::Arr(
                    self.prompts
                        .iter()
                        .map(|p| {
                            Json::obj([
                                ("name", Json::str(p.name)),
                                ("description", Json::str(p.description)),
                                (
                                    "arguments",
                                    Json::Arr(
                                        p.args
                                            .iter()
                                            .map(|(n, d, r)| Json::obj([("name", Json::str(*n)), ("description", Json::str(*d)), ("required", Json::Bool(*r))]))
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            )])),
            "prompts/get" => {
                let name = params.get("name").and_then(Json::as_str).ok_or((-32602, "missing `name`".to_string()))?;
                let p = self.prompts.iter().find(|p| p.name == name).ok_or((-32602, format!("unknown prompt `{name}`")))?;
                let args = params.get("arguments").cloned().unwrap_or(Json::Obj(vec![]));
                for (n, _, req) in &p.args {
                    if *req && args.get(n).is_none() {
                        return Err((-32602, format!("missing argument `{n}` for prompt `{name}`")));
                    }
                }
                let text = (p.get)(&args).map_err(|e| (-32602, e))?;
                Ok(Json::obj([
                    ("description", Json::str(p.description)),
                    (
                        "messages",
                        Json::Arr(vec![Json::obj([
                            ("role", Json::str("user")),
                            ("content", Json::obj([("type", Json::str("text")), ("text", Json::str(text))])),
                        ])]),
                    ),
                ]))
            }
            _ => Err((-32601, format!("unsupported method `{method}`"))),
        }
    }
}

fn error(id: Json, code: i64, msg: &str) -> Json {
    Json::obj([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("error", Json::obj([("code", Json::Num(code as f64)), ("message", Json::str(msg))])),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server {
            name: "brain",
            version: "0.1.0",
            instructions: "x",
            tools: vec![
                Tool {
                    name: "echo",
                    description: "returns `text`",
                    input_schema: Json::obj([("type", Json::str("object"))]),
                    output_schema: None,
                    run: Box::new(|a| a.get("text").and_then(Json::as_str).map(str::to_string).ok_or("missing text".into())),
                },
                Tool {
                    name: "obj",
                    description: "returns an object",
                    input_schema: Json::obj([("type", Json::str("object"))]),
                    output_schema: Some(Json::obj([("type", Json::str("object"))])),
                    run: Box::new(|_| Ok("{\"n\":1}".into())),
                },
            ],
            resources: vec![Resource { uri: "brain://x", name: "x", description: "d", mime: "text/plain", read: Box::new(|| Ok("hello".into())) }],
            prompts: vec![Prompt {
                name: "p",
                description: "d",
                args: vec![("code", "c", true)],
                get: Box::new(|a| Ok(format!("fix {}", a.get("code").and_then(Json::as_str).unwrap_or("")))),
            }],
        }
    }

    #[test]
    fn session() {
        let s = server();
        let init = s.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#).unwrap();
        assert_eq!(init.get("result").unwrap().get("protocolVersion").unwrap().as_str(), Some("2025-06-18"));
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(s.handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap().stringify(), r#"{"jsonrpc":"2.0","id":7,"result":{}}"#);
        let list = s.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap().stringify();
        assert!(list.contains(r#""name":"echo""#));
        let ok = s.handle_line(r#"{"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"echo","arguments":{"text":"hello"}}}"#).unwrap().stringify();
        assert!(ok.contains(r#""text":"hello""#) && ok.contains(r#""isError":false"#) && ok.contains(r#""id":"a""#));
        let bad = s.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{}}}"#).unwrap().stringify();
        assert!(bad.contains(r#""isError":true"#));
    }

    #[test]
    fn structured_resources_prompts() {
        let s = server();
        let r = s.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"obj"}}"#).unwrap().stringify();
        assert!(r.contains(r#""structuredContent":{"n":1}"#), "{r}");
        let l = s.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap().stringify();
        assert!(l.contains(r#""outputSchema""#));
        let rr = s.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"brain://x"}}"#).unwrap().stringify();
        assert!(rr.contains(r#""text":"hello""#));
        let p = s.handle_line(r#"{"jsonrpc":"2.0","id":4,"method":"prompts/get","params":{"name":"p","arguments":{"code":"B0302"}}}"#).unwrap().stringify();
        assert!(p.contains("fix B0302"));
        let bad = s.handle_line(r#"{"jsonrpc":"2.0","id":5,"method":"prompts/get","params":{"name":"p"}}"#).unwrap().stringify();
        assert!(bad.contains("missing argument"));
    }

    #[test]
    fn protocol_errors() {
        let s = server();
        assert!(s.handle_line("{nope").unwrap().stringify().contains("-32700"));
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":9,"method":"x/y"}"#).unwrap().stringify().contains("-32601"));
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"zzz"}}"#).unwrap().stringify().contains("-32602"));
    }
}
