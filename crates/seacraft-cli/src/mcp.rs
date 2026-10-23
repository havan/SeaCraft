//! SeaCraft's MCP server: JSON-RPC 2.0 over stdio, one message per line.
//!
//! The server implements the Model Context Protocol's lifecycle and tools
//! (`initialize`, `notifications/initialized`, `ping`, `tools/list`,
//! `tools/call`) for a hull model, so an agent can inspect the control net,
//! edit it, measure hydrostatics, solve cross curves, write a linesplan and
//! render a preview — the same work `seacraft` does from the command line.
//!
//! Two backends serve a tool call:
//!
//! * **Headless, in process** (default). [`McpServer`] holds a [`Surface`]
//!   (the demo hull, or the file given to `seacraft mcp --open`), the
//!   [`HydroOptions`] that came with it and the `kg` used when a call omits
//!   one. Everything is measured on the refined mesh, exactly as the library
//!   does.
//! * **A running app** (`--connect HOST:PORT`), when `seacraft-gui --control
//!   HOST:PORT` is listening: each tool is translated to the app's control
//!   method that does the same thing and sent as one JSON line over TCP, so a
//!   person can watch the model change. [`Remote`] writes
//!   `{"method": …, "params": {…}}` and reads `{"ok": true, "result": …}` or
//!   `{"ok": false, "error": "…"}` (`crates/seacraft-gui/src/control.rs`). A
//!   request is never retried — replaying an edit would apply it twice. An
//!   unreachable app falls back to headless with a note on stderr.
//!
//! The input is treated as hostile: a message line is capped at [`MAX_LINE`]
//! bytes and answered with a JSON-RPC error past it (the rest of the line is
//! consumed so the stream stays in step), batches at [`MAX_BATCH_MESSAGES`],
//! tool arguments are checked against the tool's schema, every index and
//! number is range-checked before use, and no path in this module indexes a
//! vector without a bounds check. A malformed message can only produce an
//! error reply.
//!
//! Adapted from Tangency Labs' `fieldcraft/crates/mcp` (`server.rs`,
//! `tools.rs`, `backend.rs`) — MIT OR Apache-2.0 — with the tool set, the
//! state and the backend split rewritten for SeaCraft's hull model. See
//! `ATTRIBUTION.md`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use seacraft_core::analysis::AnalysisMode;
use seacraft_core::hydro::{self, HydroOptions};
use seacraft_core::math::Vec3;
use seacraft_core::mesh::Surface;
use seacraft_core::stability::{
    cross_curves, evaluate_imo_criteria, hydrostatics_free_trim, upright_lcb, StabilityCondition,
};
use seacraft_core::subdiv::{SubdivisionMode, SubdivisionOptions};
use seacraft_core::{
    curves, demo, develop, export, fef, linesplan, loading, michell, offsets, optimize,
    parametric::{self, HullKind, HullSpec},
    planing, raster, resistance, table, topology, transform, trimopt,
};
use serde_json::{json, Map, Value};

/// The newest protocol revision this server speaks (`initialize` answers with
/// it unless the client asks for an older one in [`SUPPORTED_VERSIONS`]).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision the server answers to, newest first.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Longest message line accepted, in bytes. Past it the server answers with an
/// error and drops the rest of the line instead of buffering it.
pub const MAX_LINE: usize = 1 << 20;

/// Most messages in one JSON-RPC batch.
pub const MAX_BATCH_MESSAGES: usize = 64;

/// Largest picture side the `render_png` tool draws, in pixels.
const MAX_SIDE: usize = 2048;
/// Default picture size.
const DEFAULT_SIZE: (usize, usize) = (960, 600);
/// Longest reply line accepted from a connected app.
const MAX_REPLY: u64 = 32 << 20;
/// Subdivision levels a tool may ask for.
const MAX_LEVEL: u8 = 6;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
/// MCP: a `resources/read` for a URI the server does not serve.
const RESOURCE_NOT_FOUND: i64 = -32002;

const HULL_URI: &str = "seacraft://hull";
const TOOLS_URI: &str = "seacraft://tools";

const INSTRUCTIONS: &str = "SeaCraft models a ship hull as a subdivision surface and measures it.
Axes: x forward, y to starboard, z up. Units: metres, kg and tonnes (displacement in t, volume in m3).
A .fef file is FREE!ship's text exchange format: control points, edges, faces and layers of the control net.
The net is refined (Loop / Catmull-Clark hybrid, FREE!ship's weights) before anything is measured, so
hydrostatics, sections and linesplans all use the refined mesh, never the control polygon.
A symmetric layer models only y >= 0 (the starboard half) and is mirrored on evaluation; a half model needs a
transom face or its waterplane never closes.
KB, BM, KM and KG are heights above the keel (the lowest hydrostatic point); LCB and LCF are in model x.
Heel is positive to starboard (degrees). Trim is a length in metres, positive when the bow is higher than the stern.
Workflow: load_hull {path} (or {demo: true}) -> hull_info for extents, layers and validity ->
hydro_compute {draft, trim, heel_deg, kg} for one floating condition (free_trim solves trim so LCB meets LCG) ->
stability_cross_curves for KN/GZ and the IMO A.749 criteria -> render_png to look at the result.
Other measurements: bonjean_curves, table_of_offsets, hydro_table, trim_scan, resistance_curve, planing_curve,
unfold_plate. Edit with move_point, set_crease, split_face, insert_edge and transform_hull.
seakeeping_raos gives heave and pitch RAOs in regular head seas; seakeeping_irregular {hs, tp, gamma?} integrates them over a
wave spectrum for significant motions, relative motion, accelerations and MSI.
optimize_hull {speed_kn} reshapes the hull (Cp, LCB, L/B) to cut resistance at a speed, holding displacement; it edits the model.
Write results with export_linesplan (.svg/.dxf/.igs) and save_hull (.fef).
Project data: project_get and project_set read and write the design sidecar (loading conditions, compartments, damage cases, windage).
Evaluate loading conditions with loading_condition {name} (or inline case / case_path).
Evaluate damage stability with damage_stability {name} (or inline case / case_path).
Tool failures come back as a normal result with isError true and a message; protocol errors are JSON-RPC errors.
Resources: seacraft://hull (summary of the loaded hull) and seacraft://tools (catalogue with each tool's effect).";

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

// ---------------------------------------------------------------- framing --

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// One line of at most `max` bytes, without the newline. A longer line is read
/// to its end and dropped so the stream stays in step; `Err(InvalidData)`
/// reports it.
fn read_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let (done, used) = {
            let available = r.fill_buf()?;
            if available.is_empty() {
                if too_long {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "request too large",
                    ));
                }
                return Ok((!line.is_empty()).then_some(line));
            }
            let (part, done, used) = match available.iter().position(|b| *b == b'\n') {
                Some(i) => (available.get(..i).unwrap_or(&[]), true, i + 1),
                None => (available, false, available.len()),
            };
            if !too_long {
                line.extend_from_slice(part);
                if line.len() > max {
                    too_long = true;
                    line = Vec::new();
                }
            }
            (done, used)
        };
        r.consume(used);
        if done {
            if too_long {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "request too large",
                ));
            }
            return Ok(Some(line));
        }
    }
}

// ----------------------------------------------------------------- server --

/// The MCP server: a hull model and the settings a tool call without arguments
/// measures it with, plus an optional forwarding connection to a running app.
pub struct McpServer {
    surface: Surface,
    opts: HydroOptions,
    kg: f64,
    /// The file the model came from, if any (reported by `hull_info`).
    source: Option<PathBuf>,
    project: seacraft_core::project::Project,
    project_error: Option<String>,
    remote: Option<Remote>,
    initialized: bool,
}

#[derive(Clone)]
struct ServerSnapshot {
    surface: Surface,
    opts: HydroOptions,
    kg: f64,
    source: Option<PathBuf>,
    project: seacraft_core::project::Project,
    project_error: Option<String>,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
    fn snapshot(&self) -> ServerSnapshot {
        ServerSnapshot {
            surface: self.surface.clone(),
            opts: self.opts.clone(),
            kg: self.kg,
            source: self.source.clone(),
            project: self.project.clone(),
            project_error: self.project_error.clone(),
        }
    }

    fn restore_snapshot(&mut self, snap: ServerSnapshot) {
        self.surface = snap.surface;
        self.opts = snap.opts;
        self.kg = snap.kg;
        self.source = snap.source;
        self.project = snap.project;
        self.project_error = snap.project_error;
    }
    /// The demo hull (a full model), measured at its design draft.
    pub fn new() -> Self {
        Self::from_surface(demo::demo_hull(false))
    }

    /// A server over an existing model.
    pub fn from_surface(surface: Surface) -> Self {
        let opts = HydroOptions::from_surface(
            &surface,
            seacraft_core::subdiv::SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        );
        Self {
            surface,
            opts,
            kg: 1.0,
            source: None,
            project: seacraft_core::project::Project::default(),
            project_error: None,
            remote: None,
            initialized: false,
        }
    }

    /// A server whose model is the `.fef` file at `path`.
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let mut server = Self::from_surface(read_fef(path)?);
        server.source = Some(path.to_path_buf());
        match seacraft_core::project::Project::load_for_model(path) {
            Ok(Some(p)) => server.project = p,
            Ok(None) => {}
            Err(e) => server.project_error = Some(e),
        }
        Ok(server)
    }

    /// A server that forwards tool calls to `seacraft-gui --control addr`.
    ///
    /// Reads the authentication token from `token_file` if given, or the default token
    /// file for `addr` ($XDG_RUNTIME_DIR/seacraft-control-<port>.token or temp dir).
    pub fn connect(addr: &str, token_file: Option<&Path>) -> Result<Self, String> {
        let token_path = match token_file {
            Some(p) => p.to_path_buf(),
            None => {
                let port = addr
                    .rsplit(':')
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .or_else(|| {
                        use std::net::ToSocketAddrs;
                        addr.to_socket_addrs().ok()?.next().map(|sa| sa.port())
                    })
                    .ok_or_else(|| format!("cannot determine port from `{addr}`"))?;
                seacraft_core::control_auth::default_token_path(port)
            }
        };
        // An explicit file must be readable; a missing default file means the
        // app runs `--control-no-auth` (if it does not, its first reply says so).
        let token = match seacraft_core::control_auth::read_token_file(&token_path) {
            Ok(t) => Some(t),
            Err(e) if token_file.is_none() && e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(format!(
                    "cannot read control token file `{}`: {e}",
                    token_path.display()
                ))
            }
        };
        Self::connect_with_token(addr, token)
    }

    /// A server that forwards tool calls with an explicit token or no authentication (if None).
    pub fn connect_with_token(addr: &str, token: Option<String>) -> Result<Self, String> {
        let remote = Remote::connect_with_token(addr, token)
            .map_err(|e| format!("cannot reach {addr}: {e}"))?;
        let mut server = Self::new();
        server.remote = Some(remote);
        Ok(server)
    }

    /// A server that forwards tool calls without authentication (for --control-no-auth).
    #[cfg(test)]
    pub fn connect_no_auth(addr: &str) -> Result<Self, String> {
        Self::connect_with_token(addr, None)
    }

    /// Whether tool calls are forwarded to an app instead of running here.
    pub fn is_remote(&self) -> bool {
        self.remote.is_some()
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. Only protocol
    /// messages go to `output`; diagnostics belong on stderr.
    pub fn serve(
        &mut self,
        mut input: impl BufRead,
        mut output: impl Write,
    ) -> std::io::Result<()> {
        loop {
            let reply = match read_line(&mut input, MAX_LINE) {
                Ok(None) => return Ok(()),
                Ok(Some(bytes)) => match String::from_utf8(bytes) {
                    Ok(line) => self.handle_line(&line),
                    Err(_) => Some(
                        error(Value::Null, PARSE_ERROR, "parse error: input is not UTF-8")
                            .to_string(),
                    ),
                },
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Some(
                    error(
                        Value::Null,
                        INVALID_REQUEST,
                        format!("request too large (at most {MAX_LINE} bytes a line)"),
                    )
                    .to_string(),
                ),
                Err(e) => return Err(e),
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
    }

    /// One line in, at most one line out (notifications get no reply).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if line.len() > MAX_LINE {
            return Some(error(Value::Null, INVALID_REQUEST, "request too large").to_string());
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if batch.is_empty() => {
                Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
            }
            Ok(Value::Array(batch)) if batch.len() > MAX_BATCH_MESSAGES => Some(error(
                Value::Null,
                INVALID_REQUEST,
                format!("batch too large (at most {MAX_BATCH_MESSAGES} messages)"),
            )),
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> =
                    batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// One message in, its reply out (none for notifications and responses).
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "a message must be an object",
            ));
        };
        let id = o.get("id").cloned();
        if let Some(id) = &id {
            if !(id.is_string() || id.is_i64() || id.is_u64()) {
                return Some(error(
                    Value::Null,
                    INVALID_REQUEST,
                    "`id` must be a string or an integer",
                ));
            }
        }
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A reply to a request of ours (none are sent) or junk.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "missing `method`",
            ));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            // A notification: act, never reply.
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        Some(match self.request(method, &params) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .and_then(|v| SUPPORTED_VERSIONS.iter().find(|s| **s == v))
                    .copied()
                    .unwrap_or(PROTOCOL_VERSION);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {
                        "tools": {"listChanged": false},
                        "resources": {"listChanged": false},
                    },
                    "serverInfo": {"name": "seacraft-mcp", "title": "SeaCraft", "version": seacraft_core::VERSION},
                    "instructions": format!("{INSTRUCTIONS}\nBackend: {}.", self.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "resources/list" => Ok(json!({"resources": [
                {"uri": HULL_URI, "name": "hull", "title": "Loaded hull",
                 "description": "Summary of the loaded hull: dimensions, counts, layers, validity.",
                 "mimeType": "application/json"},
                {"uri": TOOLS_URI, "name": "tools", "title": "Tool catalogue",
                 "description": "Every tool with its title, one-line description and effect.",
                 "mimeType": "text/markdown"},
            ]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            "resources/read" => {
                let uri = params
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "missing resource `uri`".to_string()))?;
                let (mime, text) = match uri {
                    HULL_URI => {
                        let v = match &mut self.remote {
                            Some(r) => r.tool("hull_info", &Map::new()),
                            None => Ok(self.hull_info()),
                        }
                        .map_err(|e| (INTERNAL_ERROR, e))?;
                        (
                            "application/json",
                            serde_json::to_string_pretty(&v).unwrap_or_default(),
                        )
                    }
                    TOOLS_URI => ("text/markdown", tool_catalogue()),
                    other => {
                        return Err((
                            RESOURCE_NOT_FOUND,
                            format!("resource `{}` not found", clip(other, 100)),
                        ))
                    }
                };
                Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
            }
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a) => a.clone(),
                };
                check_arguments(name, &args).map_err(|e| (INVALID_PARAMS, e))?;
                Ok(self.call_tool(name, &args))
            }
            _ => Err((
                METHOD_NOT_FOUND,
                format!("method `{}` not found", clip(method, 200)),
            )),
        }
    }

    /// Run a tool: the app when connected, this process otherwise. A failure is
    /// a tool result with `isError: true`, never a panic.
    pub fn call_tool(&mut self, name: &str, args: &Value) -> Value {
        let empty = Map::new();
        let a = args.as_object().unwrap_or(&empty);
        if self.remote.is_some() {
            if name == "batch" {
                self.batch_remote(a)
            } else {
                let Some(remote) = &mut self.remote else {
                    unreachable!()
                };
                match remote.tool(name, a) {
                    Ok(v) => text_result(&v, false),
                    Err(e) => text_result(&Value::String(e), true),
                }
            }
        } else {
            let snapshot = self.snapshot();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if name == "batch" {
                    self.batch(a)
                } else {
                    #[cfg(test)]
                    if name == "__test_panic" {
                        panic!("deliberate test panic");
                    }
                    match self.run_tool(name, a) {
                        Ok(v) => text_result(&v, false),
                        Err(e) => text_result(&Value::String(e), true),
                    }
                }
            }));
            match res {
                Ok(v) => v,
                Err(payload) => {
                    self.restore_snapshot(snapshot);
                    let msg = panic_message(&*payload);
                    text_result(&Value::String(format!("internal error: {msg}")), true)
                }
            }
        }
    }

    fn batch(&mut self, a: &Map<String, Value>) -> Value {
        let Some(calls) = a.get("calls").and_then(Value::as_array) else {
            return text_result(
                &json!({
                    "failed": {"index": 0, "name": "batch", "error": "`calls` must be an array"},
                    "completed": 0,
                }),
                true,
            );
        };
        if calls.len() > 100 {
            return text_result(
                &json!({
                    "failed": {"index": 0, "name": "batch", "error": "batch too long (at most 100 calls)"},
                    "completed": 0,
                }),
                true,
            );
        }

        let snapshot = self.snapshot();
        let mut results = Vec::with_capacity(calls.len());

        for (i, call) in calls.iter().enumerate() {
            let Some(call_obj) = call.as_object() else {
                self.restore_snapshot(snapshot);
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "", "error": "each call must be an object"},
                        "completed": i,
                    }),
                    true,
                );
            };

            let Some(name) = call_obj.get("name").and_then(Value::as_str) else {
                self.restore_snapshot(snapshot);
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "", "error": "missing call `name`"},
                        "completed": i,
                    }),
                    true,
                );
            };

            if name == "batch" {
                self.restore_snapshot(snapshot);
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "batch", "error": "nested batch is rejected"},
                        "completed": i,
                    }),
                    true,
                );
            }

            let args = match call_obj.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a) => a.clone(),
            };

            if let Err(e) = check_arguments(name, &args) {
                self.restore_snapshot(snapshot);
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": name, "error": e},
                        "completed": i,
                    }),
                    true,
                );
            }

            let empty_map = Map::new();
            let args_map = args.as_object().unwrap_or(&empty_map);
            let run_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                #[cfg(test)]
                if name == "__test_panic" {
                    panic!("deliberate test panic");
                }
                self.run_tool(name, args_map)
            }));

            let tool_res = match run_res {
                Ok(Ok(val)) => text_result(&val, false),
                Ok(Err(err)) => text_result(&Value::String(err), true),
                Err(payload) => {
                    let msg = panic_message(&*payload);
                    text_result(&Value::String(format!("internal error: {msg}")), true)
                }
            };

            if tool_res
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                self.restore_snapshot(snapshot);
                let err_text = tool_res["content"][0]["text"]
                    .as_str()
                    .unwrap_or("error")
                    .to_string();
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": name, "error": err_text},
                        "completed": i,
                    }),
                    true,
                );
            }

            let content_text = tool_res["content"][0]["text"].as_str().unwrap_or_default();
            let parsed = serde_json::from_str::<Value>(content_text)
                .unwrap_or_else(|_| Value::String(content_text.to_string()));
            results.push(parsed);
        }

        text_result(&json!({"results": results}), false)
    }

    fn batch_remote(&mut self, a: &Map<String, Value>) -> Value {
        let Some(calls) = a.get("calls").and_then(Value::as_array) else {
            return text_result(
                &json!({
                    "failed": {"index": 0, "name": "batch", "error": "`calls` must be an array"},
                    "completed": 0,
                }),
                true,
            );
        };
        if calls.len() > 100 {
            return text_result(
                &json!({
                    "failed": {"index": 0, "name": "batch", "error": "batch too long (at most 100 calls)"},
                    "completed": 0,
                }),
                true,
            );
        }

        let Some(remote) = &mut self.remote else {
            return text_result(&Value::String("not connected".into()), true);
        };

        let mark_id = match remote.request("history.mark", json!({})) {
            Ok(v) => v.get("mark").and_then(Value::as_u64),
            Err(e) => {
                return text_result(
                    &Value::String(format!("failed to snapshot GUI history: {e}")),
                    true,
                );
            }
        };

        let mut results = Vec::with_capacity(calls.len());

        for (i, call) in calls.iter().enumerate() {
            let Some(call_obj) = call.as_object() else {
                if let Some(mark) = mark_id {
                    let _ = remote.request("history.restore", json!({"mark": mark}));
                }
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "", "error": "each call must be an object"},
                        "completed": i,
                    }),
                    true,
                );
            };

            let Some(name) = call_obj.get("name").and_then(Value::as_str) else {
                if let Some(mark) = mark_id {
                    let _ = remote.request("history.restore", json!({"mark": mark}));
                }
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "", "error": "missing call `name`"},
                        "completed": i,
                    }),
                    true,
                );
            };

            if name == "batch" {
                if let Some(mark) = mark_id {
                    let _ = remote.request("history.restore", json!({"mark": mark}));
                }
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": "batch", "error": "nested batch is rejected"},
                        "completed": i,
                    }),
                    true,
                );
            }

            let args = match call_obj.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a) => a.clone(),
            };

            if let Err(e) = check_arguments(name, &args) {
                if let Some(mark) = mark_id {
                    let _ = remote.request("history.restore", json!({"mark": mark}));
                }
                return text_result(
                    &json!({
                        "failed": {"index": i, "name": name, "error": e},
                        "completed": i,
                    }),
                    true,
                );
            }

            let empty_map = Map::new();
            let args_map = args.as_object().unwrap_or(&empty_map);
            match remote.tool(name, args_map) {
                Ok(val) => {
                    results.push(val);
                }
                Err(e) => {
                    if let Some(mark) = mark_id {
                        let _ = remote.request("history.restore", json!({"mark": mark}));
                    }
                    return text_result(
                        &json!({
                            "failed": {"index": i, "name": name, "error": e},
                            "completed": i,
                        }),
                        true,
                    );
                }
            }
        }

        if let Some(mark) = mark_id {
            let _ = remote.request("history.forget", json!({"mark": mark}));
        }

        text_result(&json!({"results": results}), false)
    }

    fn describe(&self) -> String {
        match &self.remote {
            Some(r) => format!(
                "connected to the SeaCraft app at {addr} (the user watches its window)",
                addr = r.addr
            ),
            None => "a headless in-process SeaCraft session".to_string(),
        }
    }

    fn run_tool(&mut self, name: &str, a: &Map<String, Value>) -> Result<Value, String> {
        match name {
            "hull_info" => Ok(self.hull_info()),
            "load_hull" => self.load_hull(a),
            "generate_hull" => self.generate_hull(a),
            "save_hull" => self.save_hull(a),
            "project_get" => self.project_get(a),
            "project_set" => self.project_set(a),
            "move_point" => self.move_point(a),
            "set_crease" => self.set_crease(a),
            "split_face" => self.split_face(a),
            "insert_edge" => self.insert_edge(a),
            "hydro_compute" => self.hydro_compute(a),
            "stability_cross_curves" => self.stability_curves(a),
            "loading_condition" => self.loading_condition(a),
            "weather_criterion" => self.weather_criterion(a),
            "damage_stability" => self.damage_stability(a),
            "seakeeping_raos" => self.seakeeping_raos(a),
            "seakeeping_irregular" => self.seakeeping_irregular(a),
            "bonjean_curves" => self.bonjean_curves(a),
            "table_of_offsets" => self.table_of_offsets(a),
            "resistance_curve" => self.resistance_curve(a),
            "planing_curve" => self.planing_curve(a),
            "transform_hull" => self.transform_hull(a),
            "unfold_plate" => self.unfold_plate(a),
            "hydro_table" => self.hydro_table(a),
            "trim_scan" => self.trim_scan(a),
            "optimize_hull" => self.optimize_hull(a),
            "export_linesplan" => self.export_linesplan(a),
            "render_png" => self.render_png(a),
            "design_report" => self.design_report(a),
            other => Err(format!("unknown tool `{}`", clip(other, 100))),
        }
    }

    // ------------------------------------------------------------- tools --

    fn hull_info(&self) -> Value {
        let s = &self.surface;
        let (min, max) = s.extents();
        let counts = s.edge_face_counts();
        let boundary = counts.iter().filter(|&&c| c == 1).count();
        let non_manifold = counts.iter().filter(|&&c| c > 2).count();
        let creases = s.edges.iter().filter(|e| e.crease).count();
        let quads = s.faces.iter().filter(|f| f.len() == 4).count();
        let triangles = s.faces.iter().filter(|f| f.len() == 3).count();
        let layers: Vec<Value> = s
            .layers
            .iter()
            .map(|l| {
                json!({
                    "name": l.name,
                    "id": l.id,
                    "visible": l.visible,
                    "symmetric": l.symmetric,
                    "use_in_hydrostatics": l.use_in_hydrostatics,
                    "density": l.density,
                    "thickness": l.thickness,
                })
            })
            .collect();
        json!({
            "name": s.settings.name,
            "designer": s.settings.designer,
            "source": self.source.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "backend": if self.is_remote() { "app" } else { "headless" },
            "principal": {
                "length": s.settings.length,
                "beam": s.settings.beam,
                "draft": s.settings.draft,
                "water_density": s.settings.water_density,
                "appendage_coefficient": s.settings.appendage_coefficient,
                "mainframe_x": s.settings.mainframe_location,
            },
            "extents": {
                "min": min,
                "max": max,
                "length": max.x - min.x,
                "beam": max.y - min.y,
                "depth": max.z - min.z,
                "keel": s.lowest_hydrostatics_point(),
            },
            "points": s.points.len(),
            "faces": s.faces.len(),
            "edges": {
                "total": s.edges.len(),
                "crease": creases,
                "boundary": boundary,
                "non_manifold": non_manifold,
            },
            "face_mix": {
                "quads": quads,
                "triangles": triangles,
                "n_gons": s.faces.len() - quads - triangles,
            },
            "control_area_m2": s.control_area(),
            "hydrostatics_layers": s.layers.iter().filter(|l| l.use_in_hydrostatics).map(|l| l.name.clone()).collect::<Vec<_>>(),
            "layers": layers,
            "validation": match s.validate() {
                Ok(()) => json!({"ok": true}),
                Err(e) => json!({"ok": false, "error": e}),
            },
        })
    }

    fn load_hull(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let demo = bool_arg(a, "demo").unwrap_or(false);
        let mut parse_err = None;
        match str_arg(a, "path") {
            Some(path) if !path.trim().is_empty() => {
                let surface = read_fef(Path::new(path))?;
                self.surface = surface;
                self.opts = HydroOptions::from_surface(
                    &self.surface,
                    seascape_subdiv(self.opts.subdivision.level),
                );
                self.source = Some(PathBuf::from(path));
                match seacraft_core::project::Project::load_for_model(Path::new(path)) {
                    Ok(Some(p)) => {
                        self.project = p;
                        self.project_error = None;
                    }
                    Ok(None) => {
                        self.project = seacraft_core::project::Project::default();
                        self.project_error = None;
                    }
                    Err(e) => {
                        self.project = seacraft_core::project::Project::default();
                        self.project_error = Some(e.clone());
                        parse_err = Some(e);
                    }
                }
            }
            _ if demo => {
                self.surface = demo::demo_hull(bool_arg(a, "symmetric").unwrap_or(false));
                self.opts = HydroOptions::from_surface(
                    &self.surface,
                    seascape_subdiv(self.opts.subdivision.level),
                );
                self.source = None;
                self.project = seacraft_core::project::Project::default();
                self.project_error = None;
            }
            _ => return Err("load_hull: give `path` or `demo: true`".to_string()),
        }
        let mut info = self.hull_info();
        if let Some(err) = parse_err {
            if let Some(obj) = info.as_object_mut() {
                obj.insert("project_error".into(), json!(err));
            }
        }
        Ok(info)
    }

    fn generate_hull(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let kind = match str_arg(a, "kind") {
            Some(k) => k.parse::<HullKind>()?,
            None => HullKind::Displacement,
        };
        let mut spec = HullSpec::default_for(kind);
        if let Some(v) = number_arg(a, "lwl") {
            spec.lwl = v;
        }
        if let Some(v) = number_arg(a, "beam") {
            spec.beam = v;
        }
        if let Some(v) = number_arg(a, "draft") {
            spec.draft = v;
        }
        if let Some(v) = number_arg(a, "depth") {
            spec.depth = v;
        }
        if let Some(v) = number_arg(a, "cb") {
            spec.cb = Some(v);
        }
        if let Some(v) = number_arg(a, "lcb_frac").or_else(|| number_arg(a, "lcb")) {
            spec.lcb_frac = Some(v);
        }
        if let Some(v) = number_arg(a, "deadrise_deg").or_else(|| number_arg(a, "deadrise")) {
            spec.deadrise_deg = v;
        }
        if let Some(v) = number_arg(a, "transom_frac").or_else(|| number_arg(a, "transom")) {
            spec.transom_frac = v;
        }
        if let Some(v) = usize_arg(a, "stations") {
            spec.stations = v;
        }
        if let Some(v) = usize_arg(a, "points_per_station") {
            spec.points_per_station = v;
        }

        let (surface, rep) = parametric::generate(&spec)?;
        self.surface = surface;
        self.opts =
            HydroOptions::from_surface(&self.surface, seascape_subdiv(self.opts.subdivision.level));
        self.source = None;
        self.project = seacraft_core::project::Project::default();
        self.project_error = None;

        let mut out = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
        if let Some(obj) = out.as_object_mut() {
            obj.insert("points".into(), json!(self.surface.points.len()));
            obj.insert("faces".into(), json!(self.surface.faces.len()));
            obj.insert("layers".into(), json!(self.surface.layers.len()));
        }
        Ok(out)
    }

    fn save_hull(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let path = match str_arg(a, "path") {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => match &self.source {
                Some(p) => p.clone(),
                None => return Err("save_hull: give `path` (this model has no file yet)".into()),
            },
        };
        let text = fef::write(&self.surface);
        std::fs::write(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
        self.source = Some(path.clone());
        // The sidecar travels with the model, as in the GUI: written when it
        // holds something or already exists, never over one that failed to load.
        let sidecar = seacraft_core::project::sidecar_path(&path);
        let project = if self.project_error.is_some() {
            None
        } else if !self.project.is_empty() || sidecar.exists() {
            self.project
                .save(&sidecar)
                .map_err(|e| format!("save_hull: {e}"))?;
            Some(sidecar.to_string_lossy().to_string())
        } else {
            None
        };
        Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": text.len(),
            "points": self.surface.points.len(),
            "faces": self.surface.faces.len(),
            "project": project,
        }))
    }

    fn move_point(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let index = usize_arg(a, "index").ok_or("move_point: `index` must be a count of points")?;
        let count = self.surface.points.len();
        if index >= count {
            return Err(format!(
                "move_point: `index` {index} is out of range (the model has {count} points)"
            ));
        }
        let xyz = a
            .get("xyz")
            .and_then(Value::as_array)
            .ok_or("move_point: `xyz` must be [x, y, z]")?;
        if xyz.len() != 3 {
            return Err(format!(
                "move_point: `xyz` needs 3 numbers, got {}",
                xyz.len()
            ));
        }
        let mut p = Vec3::ZERO;
        for (i, v) in xyz.iter().enumerate() {
            let n = v
                .as_f64()
                .filter(|n| n.is_finite() && n.abs() <= 1.0e6)
                .ok_or(
                    "move_point: `xyz` must be three finite numbers within 1000 km of the origin",
                )?;
            match i {
                0 => p.x = n,
                1 => p.y = n,
                _ => p.z = n,
            }
        }
        let before = match self.surface.points.get(index) {
            Some(cp) => cp.p,
            None => return Err("move_point: point vanished".to_string()),
        };
        // A symmetric layer is modelled for y >= 0 only; mirroring would put a
        // point moved to negative y back on top of itself.
        let clamped = if p.y < 0.0 && self.point_is_symmetric(index) {
            p.y = 0.0;
            true
        } else {
            false
        };
        if let Some(cp) = self.surface.points.get_mut(index) {
            cp.p = p;
        }
        Ok(json!({
            "index": index,
            "before": before,
            "after": p,
            "clamped": clamped,
        }))
    }

    fn point_is_symmetric(&self, index: usize) -> bool {
        let mut touched = false;
        let mut all = true;
        for (fi, face) in self.surface.faces.iter().enumerate() {
            if face.contains(&index) {
                touched = true;
                let symmetric = self
                    .surface
                    .face_layer
                    .get(fi)
                    .and_then(|&l| self.surface.layers.get(l))
                    .map(|l| l.symmetric)
                    .unwrap_or(false);
                all &= symmetric;
            }
        }
        if touched {
            all
        } else if self.surface.layers.is_empty() {
            false
        } else {
            self.surface.layers.iter().all(|l| l.symmetric)
        }
    }

    fn set_crease(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let count = self.surface.points.len();
        let a_idx = usize_arg(a, "a").ok_or("set_crease: `a` must be a point index")?;
        let b_idx = usize_arg(a, "b").ok_or("set_crease: `b` must be a point index")?;
        if a_idx >= count || b_idx >= count {
            return Err(format!(
                "set_crease: point index out of range (the model has {count} points)"
            ));
        }
        let crease = bool_arg(a, "crease").unwrap_or(true);
        let key = seacraft_core::mesh::ordered(a_idx, b_idx);
        let found = self.surface.edges.iter().position(|e| e.key() == key);
        let Some(i) = found else {
            return Err(format!(
                "set_crease: points {a_idx} and {b_idx} share no edge"
            ));
        };
        if let Some(e) = self.surface.edges.get_mut(i) {
            e.crease = crease;
        }
        Ok(json!({"a": key.0, "b": key.1, "crease": crease, "edges": self.surface.edges.len()}))
    }

    fn split_face(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let faces = self.surface.faces.len();
        let face = usize_arg(a, "face").filter(|f| *f < faces).ok_or_else(|| {
            format!(
                "split_face: `face` must be from 0 to {}",
                faces.saturating_sub(1)
            )
        })?;
        let report = topology::split_face(&mut self.surface, face)
            .map_err(|e| format!("split_face: {e}"))?;
        Ok(json!({
            "face": report.face,
            "point": report.point,
            "pieces": report.pieces,
            "faces": report.faces_after,
            "points": report.points_after,
            "points_before": report.points_before,
            "faces_before": report.faces_before,
        }))
    }

    fn insert_edge(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let faces = self.surface.faces.len();
        let points = self.surface.points.len();
        let face = usize_arg(a, "face").filter(|f| *f < faces).ok_or_else(|| {
            format!(
                "insert_edge: `face` must be from 0 to {}",
                faces.saturating_sub(1)
            )
        })?;
        let pick = |key: &str| -> Result<usize, String> {
            usize_arg(a, key).filter(|v| *v < points).ok_or_else(|| {
                format!(
                    "insert_edge: `{key}` must be from 0 to {}",
                    points.saturating_sub(1)
                )
            })
        };
        let (x, y) = (pick("a")?, pick("b")?);
        let before = self.surface.faces.len();
        topology::insert_edge(&mut self.surface, face, x, y)
            .map_err(|e| format!("insert_edge: {e}"))?;
        Ok(json!({
            "face": face,
            "a": x,
            "b": y,
            "faces": self.surface.faces.len(),
            "faces_before": before,
            "edges": self.surface.edges.len(),
        }))
    }

    fn hydro_compute(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let mut opts = self.opts.clone();
        self.apply_condition(a, &mut opts)?;
        if !opts.draft.is_finite() || opts.draft <= 0.0 {
            return Err(
                "hydro_compute: `draft` must be positive (give one, or load a model with a design draft)"
                    .to_string(),
            );
        }
        let kg = number_arg(a, "kg").unwrap_or(self.kg);
        let lcg = number_arg(a, "lcg");
        let free = bool_arg(a, "free_trim").unwrap_or(false) || lcg.is_some();
        let result = if free {
            let hull = hydro::Hull::new(&self.surface, opts.subdivision);
            hydrostatics_free_trim(&hull, &opts, lcg).0
        } else {
            hydro::compute(&self.surface, &opts)
        };
        let mut v = serde_json::to_value(&result).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("gm_m".into(), json!(result.gm(kg)));
            o.insert("kg".into(), json!(kg));
            o.insert("free_to_trim".into(), json!(free));
        }
        // Keep the condition that was actually measured on the server for the
        // next call that omits `draft`.
        self.opts.draft = result.draft;
        Ok(v)
    }

    fn stability_curves(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let mut base = self.opts.clone();
        self.apply_condition(a, &mut base)?;
        if !base.draft.is_finite() || base.draft <= 0.0 {
            return Err("stability_cross_curves: `draft` must be positive".to_string());
        }
        let kg = number_arg(a, "kg").unwrap_or(self.kg);
        let lcg = number_arg(a, "lcg");
        let free = bool_arg(a, "free_trim").unwrap_or(false) || lcg.is_some();
        let hull = hydro::Hull::new(&self.surface, base.subdivision);
        let heels = match a.get("heels") {
            Some(v) => numbers(v, "heels")?,
            None => vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0],
        };
        if heels.is_empty() {
            return Err("stability_cross_curves: `heels` is empty".into());
        }
        let displacements = match a.get("displacements") {
            Some(v) => numbers(v, "displacements")?,
            None => {
                let volume = hull.displaced_volume(base.draft, base.trim, 0.0);
                let t = volume * base.water_density / 1000.0;
                vec![(t * 0.5).round(), t.round(), (t * 1.25).round()]
            }
        };
        if displacements.is_empty() || displacements.iter().any(|d| !d.is_finite() || *d <= 0.0) {
            return Err("stability_cross_curves: `displacements` must be positive tonnes".into());
        }
        let lcg_x = if free {
            Some(lcg.unwrap_or_else(|| upright_lcb(&hull, base.draft)))
        } else {
            None
        };
        let cond = StabilityCondition {
            kg,
            trim: base.trim,
            lcg_x,
        };
        let curves = cross_curves(&hull, &heels, &displacements, &cond, &base);
        let imo: Vec<Value> = (0..displacements.len())
            .filter_map(|i| evaluate_imo_criteria(&curves, i))
            .filter_map(|c| serde_json::to_value(c).ok())
            .collect();
        Ok(json!({
            "condition": {
                "kg": kg,
                "trim": base.trim,
                "lcg_x": lcg_x,
                "free_to_trim": free,
            },
            "heels_deg": heels,
            "displacements_t": displacements,
            "curves": serde_json::to_value(&curves).map_err(|e| e.to_string())?,
            "imo_criteria": imo,
        }))
    }

    fn project_get(&mut self, _a: &Map<String, Value>) -> Result<Value, String> {
        let mut v = serde_json::to_value(&self.project).map_err(|e| format!("project_get: {e}"))?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn project_set(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let p_val = a.get("project").ok_or("project_set: give `project`")?;
        let p_text = serde_json::to_string(p_val).map_err(|e| format!("project_set: {e}"))?;
        let project = seacraft_core::project::Project::from_json(&p_text)
            .map_err(|e| format!("project_set: {e}"))?;
        self.project = project;
        self.project_error = None;
        Ok(json!({
            "format": self.project.format,
            "version": self.project.version,
            "empty": self.project.is_empty(),
            "loading_conditions": self.project.loading_conditions.len(),
            "backend": "headless",
        }))
    }

    fn loading_condition(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let cond: loading::LoadingCondition =
            if let Some(case_val) = a.get("case").filter(|v| !v.is_null()) {
                serde_json::from_value(case_val.clone())
                    .map_err(|e| format!("loading_condition: invalid `case`: {e}"))?
            } else if let Some(path) = str_arg(a, "case_path").filter(|p| !p.trim().is_empty()) {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("loading_condition: cannot read {path}: {e}"))?;
                serde_json::from_str(&text)
                    .map_err(|e| format!("loading_condition: invalid JSON in {path}: {e}"))?
            } else {
                let name = str_arg(a, "name");
                let c = self
                    .project
                    .loading_condition(name)
                    .map_err(|e| format!("loading_condition: {e}"))?;
                self.project.with_windage(c)
            };
        loading::validate(&cond).map_err(|e| format!("loading_condition: {e}"))?;

        let mut opts = self.opts.clone();
        if a.contains_key("level") {
            opts.subdivision.level = level_arg(a)?;
        }
        let res = loading::evaluate(&self.surface, &opts, &cond);
        let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn weather_criterion(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let mut base = self.opts.clone();
        self.apply_condition(a, &mut base)?;
        if !base.draft.is_finite() || base.draft <= 0.0 {
            return Err("weather_criterion: `draft` must be positive".to_string());
        }
        let kg = match a.get("kg") {
            None | Some(Value::Null) => self.kg,
            Some(v) => v
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("weather_criterion: `kg` must be a non-negative finite number")?,
        };
        let lcg = number_arg(a, "lcg");
        let free = bool_arg(a, "free_trim").unwrap_or(false) || lcg.is_some();

        let bilge = match str_arg(a, "bilge") {
            None => {
                if let Some(ak) = number_arg(a, "bilge_keel_area_m2") {
                    if ak < 0.0 {
                        return Err("weather_criterion: `bilge_keel_area_m2` must be >= 0".into());
                    }
                    seacraft_core::weather::BilgeKind::Keels { area_m2: ak }
                } else {
                    seacraft_core::weather::BilgeKind::Round
                }
            }
            Some("keels") => {
                let ak = number_arg(a, "bilge_keel_area_m2").unwrap_or(0.0);
                if ak < 0.0 {
                    return Err("weather_criterion: `bilge_keel_area_m2` must be >= 0".into());
                }
                seacraft_core::weather::BilgeKind::Keels { area_m2: ak }
            }
            Some(s) => seacraft_core::weather::BilgeKind::parse(s)
                .map_err(|e| format!("weather_criterion: {e}"))?,
        };

        let windage_extra: Vec<seacraft_core::weather::WindageItem> = match a.get("windage_extra") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| format!("weather_criterion: invalid `windage_extra`: {e}"))?,
        };
        for w in &windage_extra {
            if !w.area_m2.is_finite() || w.area_m2 < 0.0 || !w.z_centroid.is_finite() {
                return Err(format!(
                    "weather_criterion: windage item `{}` must have finite `area_m2 >= 0` and `z_centroid`",
                    w.name
                ));
            }
        }

        let downflooding_deg = match a.get("downflooding_deg") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let df = v
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .ok_or("weather_criterion: `downflooding_deg` must be positive")?;
                Some(df)
            }
        };

        let wind_pressure_pa = match a.get("wind_pressure_pa") {
            None | Some(Value::Null) => seacraft_core::weather::DEFAULT_WIND_PRESSURE_PA,
            Some(v) => v
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("weather_criterion: `wind_pressure_pa` must be non-negative")?,
        };

        let hull = hydro::Hull::new(&self.surface, base.subdivision);
        let lcg_x = if free {
            Some(lcg.unwrap_or_else(|| upright_lcb(&hull, base.draft)))
        } else {
            None
        };
        let cond = StabilityCondition {
            kg,
            trim: base.trim,
            lcg_x,
        };
        let w_input = seacraft_core::weather::WeatherInput {
            windage_extra,
            bilge,
            downflooding_deg,
            wind_pressure_pa,
        };
        let res = seacraft_core::weather::evaluate_hull_weather(&hull, &base, &cond, &w_input);
        let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn damage_stability(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let case: seacraft_core::damage::DamageCase =
            if let Some(case_val) = a.get("case").filter(|v| !v.is_null()) {
                serde_json::from_value(case_val.clone())
                    .map_err(|e| format!("damage_stability: invalid `case`: {e}"))?
            } else if let Some(path) = a
                .get("case_path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                let contents = std::fs::read_to_string(path)
                    .map_err(|e| format!("damage_stability: cannot read {path}: {e}"))?;
                serde_json::from_str(&contents)
                    .map_err(|e| format!("damage_stability: invalid JSON in {path}: {e}"))?
            } else {
                let name = str_arg(a, "name");
                let c = self
                    .project
                    .damage_case(name)
                    .map_err(|e| format!("damage_stability: {e}"))?;
                self.project
                    .resolve_damage_case(c)
                    .map_err(|e| format!("damage_stability: {e}"))?
            };
        seacraft_core::damage::validate(&case).map_err(|e| format!("damage_stability: {e}"))?;

        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let mut opts = self.opts.clone();
        opts.subdivision.level = level;

        let res = seacraft_core::damage::evaluate(&self.surface, &opts, &case)
            .map_err(|e| format!("damage_stability: {e}"))?;
        let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn seakeeping_raos(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let draft = match a.get("draft").and_then(|v| v.as_f64()) {
            Some(d) => d,
            None if self.surface.settings.draft > 0.0 => self.surface.settings.draft,
            None => {
                return Err(
                    "seakeeping_raos: draft must be specified or set in project settings".into(),
                )
            }
        };
        if draft <= 0.0 {
            return Err("seakeeping_raos: draft must be positive".into());
        }
        let speed_kn = a.get("speed_kn").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let kg = a.get("kg").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let kyy = a.get("kyy").and_then(|v| v.as_f64());
        let stations = a
            .get("stations")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(21);
        let lambda_min = a.get("lambda_min").and_then(|v| v.as_f64());
        let lambda_max = a.get("lambda_max").and_then(|v| v.as_f64());
        let points = a.get("points").and_then(|v| v.as_u64()).map(|n| n as usize);

        let spec = seacraft_core::seakeeping::SeakeepingSpec {
            draft,
            speed_kn,
            kg,
            kyy,
            stations,
            lambda_min,
            lambda_max,
            points,
            ..Default::default()
        };

        let rep = seacraft_core::seakeeping::head_sea_raos(&self.surface, &spec)
            .map_err(|e| format!("seakeeping_raos: {e}"))?;

        let mut v = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn seakeeping_irregular(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let params = Value::Object(a.clone());
        let spec = seacraft_core::seastate::IrregularSpec::from_params(
            &params,
            self.surface.settings.draft,
        )
        .map_err(|e| format!("seakeeping_irregular: {e}"))?;
        let rep = seacraft_core::seastate::irregular_head_sea(&self.surface, &spec)
            .map_err(|e| format!("seakeeping_irregular: {e}"))?;
        let mut v = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
        }
        Ok(v)
    }

    fn bonjean_curves(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let stations = count_arg(a, "stations", 21, 2, 200)?;
        let steps = count_arg(a, "draft_steps", 10, 2, 50)?;
        let mut subdiv = self.opts.subdivision;
        subdiv.level = level;
        let hull = hydro::Hull::new(&self.surface, subdiv);
        let depth = hull.model_max.z - hull.keel;
        if depth <= 0.0 {
            return Err(
                "bonjean_curves: the hull has no height above the keel (is a layer hidden?)"
                    .to_string(),
            );
        }
        let drafts = match a.get("drafts") {
            None | Some(Value::Null) => curves::draft_range(depth / steps as f64, depth, steps),
            Some(v) => {
                let mut given = numbers(v, "drafts").map_err(|e| format!("bonjean_curves: {e}"))?;
                if given.len() < 2 {
                    return Err("bonjean_curves: `drafts` needs at least two ordinates".into());
                }
                if given.iter().any(|d| *d <= 0.0) {
                    return Err("bonjean_curves: `drafts` must be positive".into());
                }
                given.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                given
            }
        };
        let bj = curves::bonjean(&hull, stations, &drafts);
        Ok(json!({
            "backend": "headless",
            "level": level,
            "depth_m": depth,
            "design_draft_m": self.opts.draft,
            "drafts_m": bj.drafts_m,
            "sections": bj.sections,
            "area_max_m2": bj.max_area(),
        }))
    }

    fn table_of_offsets(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let stations = count_arg(a, "stations", 21, 2, 200)?;
        let wl_steps = count_arg(a, "waterline_steps", 6, 1, 50)?;
        let b_steps = count_arg(a, "buttock_steps", 4, 1, 50)?;
        let mut subdiv = self.opts.subdivision;
        subdiv.level = level;
        let hull = hydro::Hull::new(&self.surface, subdiv);
        let waterlines = match a.get("waterlines") {
            None | Some(Value::Null) => offsets::default_waterlines(&hull, wl_steps),
            Some(v) => positives(v, "waterlines")?,
        };
        let buttocks = match a.get("buttocks") {
            None | Some(Value::Null) => offsets::default_buttocks(&hull, b_steps),
            Some(v) => positives(v, "buttocks")?,
        };
        let table = offsets::table_of_offsets(&hull, stations, &waterlines, &buttocks);
        let mut v = serde_json::to_value(&table).map_err(|e| e.to_string())?;
        if let Some(o) = v.as_object_mut() {
            o.insert("backend".into(), json!("headless"));
            o.insert("level".into(), json!(level));
            o.insert("max_half_breadth_m".into(), json!(table.max_half_breadth()));
        }
        Ok(v)
    }

    fn resistance_curve(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let mut opts = self.opts.clone();
        opts.subdivision.level = level;
        if let Some(d) = number_arg(a, "draft") {
            if !(d.is_finite() && d > 0.0) {
                return Err("resistance_curve: `draft` must be positive".into());
            }
            opts.draft = d;
        }
        let hull = hydro::Hull::new(&self.surface, opts.subdivision);
        let speeds = match a.get("speeds") {
            Some(v) => positives(v, "speeds")?,
            None => vec![6.0, 8.0, 10.0, 12.0, 14.0],
        };

        let method = str_arg(a, "method").unwrap_or("holtrop").to_lowercase();
        let cat_spacing = number_arg(a, "cat_spacing");
        if method == "michell" {
            let grid = michell::michell_grid_from_surface(&self.surface, opts.draft, 65, 25, opts.subdivision)
                .ok_or("resistance_curve: cannot extract Michell offsets grid from hull surface (positive draft and waterline needed)")?;
            let holtrop_in = resistance::HoltropInput::measure(&hull, &opts);
            let form_factor = if let Some(ref h_in) = holtrop_in {
                let r = resistance::holtrop(h_in, 1.0);
                if h_in.warnings().is_empty() {
                    r.form_factor
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let viscosity = if (opts.water_density - 1000.0).abs() < 25.0 {
                resistance::VISCOSITY_FRESH_WATER
            } else {
                resistance::VISCOSITY_SEA_WATER
            };
            let mopts = michell::MichellOptions {
                nx: 65,
                nz: 25,
                ntheta: 128,
                catamaran_spacing: cat_spacing,
            };
            let rows = michell::michell_resistance_curve(
                &grid,
                form_factor,
                viscosity,
                opts.water_density,
                resistance::GRAVITY,
                &speeds,
                &mopts,
            );
            let mut warnings = Vec::new();
            let lb = if grid.beam_wl > 1e-6 {
                grid.length_wl / grid.beam_wl
            } else {
                0.0
            };
            if lb < 6.0 {
                warnings.push(format!(
                    "L/B {lb:.2} is outside thin-ship recommended range (L/B >= 6.0)"
                ));
            }
            for r in &rows {
                if !(0.20..=1.0).contains(&r.fn_) {
                    warnings.push(format!(
                        "Fn {:.3} at {:.2} kn is outside typical Michell applicability range (0.20-1.0)",
                        r.fn_, r.speed_kn
                    ));
                }
            }
            return Ok(json!({
                "backend": "headless",
                "method": "michell",
                "cat_spacing": cat_spacing,
                "length_wl": grid.length_wl,
                "beam_wl": grid.beam_wl,
                "draft": grid.draft,
                "wetted_surface_m2": grid.wetted_surface_m2,
                "form_factor": form_factor,
                "warnings": warnings,
                "speeds": rows,
            }));
        }

        if method == "dsyhs" {
            let dsyhs_in = resistance::DsyhsInput::measure(&hull, &opts)
                .ok_or("resistance_curve: cannot measure the hull for DSYHS (positive draft and waterline needed)")?;
            let mut warnings = dsyhs_in.warnings();
            let rows = resistance::dsyhs_curve(&dsyhs_in, &speeds);
            for r in &rows {
                if !(0.15..=0.75).contains(&r.fn_) {
                    warnings.push(format!(
                        "Fn {:.3} at {:.2} kn is outside DSYHS valid range (0.15-0.75, clamped)",
                        r.fn_, r.speed_kn
                    ));
                }
            }
            return Ok(json!({
                "backend": "headless",
                "method": "dsyhs",
                "input": dsyhs_in,
                "warnings": warnings,
                "speeds": rows,
            }));
        }

        let mut input = resistance::HoltropInput::measure(&hull, &opts)
            .ok_or("resistance_curve: cannot measure the hull (a positive draft and a waterline are needed)")?;
        if let Some(v) = number_arg(a, "cstern") {
            if v.abs() > 25.0 {
                return Err("resistance_curve: `cstern` must be -25 to 25".into());
            }
            input.c_stern = v;
        }
        input.bulb_area_m2 = number_arg(a, "bulb_area").unwrap_or(0.0).max(0.0);
        input.bulb_centre_z = number_arg(a, "bulb_z").unwrap_or(0.0).max(0.0);
        let rows = resistance::curve(&input, &speeds);
        let diameter = number_arg(a, "propeller_diameter").unwrap_or(0.0).max(0.0);
        let propulsion: Vec<resistance::Propulsion> = if diameter > 0.0 {
            let propeller = resistance::Propeller {
                diameter_m: diameter,
                expanded_area_ratio: number_arg(a, "expanded_area").unwrap_or(0.70),
                open_water_efficiency: number_arg(a, "eta_o").unwrap_or(0.60),
            };
            rows.iter()
                .map(|r| {
                    let mut per_speed = input.clone();
                    per_speed.viscous_coefficient = r.viscous_coefficient;
                    per_speed.propulsion(&propeller)
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(json!({
            "backend": "headless",
            "input": input,
            "warnings": input.warnings(),
            "speeds": rows,
            "propulsion": propulsion,
        }))
    }

    fn planing_curve(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let mut opts = self.opts.clone();
        opts.subdivision.level = level;
        if let Some(d) = number_arg(a, "draft") {
            if !(d.is_finite() && d > 0.0) {
                return Err("planing_curve: `draft` must be positive".into());
            }
            opts.draft = d;
        }
        let setup = planing::setup(
            &self.surface,
            opts.subdivision,
            &opts,
            number_arg(a, "beam"),
            number_arg(a, "lcg_x"),
            number_arg(a, "deadrise"),
        )
        .map_err(|e| format!("planing_curve: {e}"))?;
        let speeds = match a.get("speeds") {
            Some(v) => positives(v, "speeds")?,
            None => vec![20.0, 25.0, 30.0],
        };
        let rows = planing::planing_curve(&setup.hull, &speeds);
        if rows.is_empty() {
            return Err(format!(
                "planing_curve: no planing equilibrium between {} and {} kn",
                speeds.first().copied().unwrap_or(0.0),
                speeds.last().copied().unwrap_or(0.0)
            ));
        }
        Ok(json!({
            "backend": "headless",
            "setup": setup,
            "speeds": rows,
        }))
    }

    fn transform_hull(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let subdiv = self.opts.subdivision;
        let mirror = match str_arg(a, "mirror") {
            None => None,
            Some(name) => Some(transform::Axis::parse(name).ok_or_else(|| {
                format!("transform_hull: `mirror` must be x, y or z (got `{name}`)")
            })?),
        };
        let vec3 = |key: &str| -> Result<Option<Vec3>, String> {
            match a.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => {
                    let items = numbers(v, key)?;
                    if items.len() != 3 {
                        return Err(format!("transform_hull: `{key}` must be [x, y, z]"));
                    }
                    Ok(Some(Vec3::new(items[0], items[1], items[2])))
                }
            }
        };
        let request = transform::TransformRequest {
            lwl: number_arg(a, "lwl"),
            bwl: number_arg(a, "bwl"),
            draft: number_arg(a, "draft"),
            scale: vec3("scale")?,
            translate: vec3("translate")?.unwrap_or(Vec3::ZERO),
            mirror,
        };
        let (out, report) = transform::run(&self.surface, &request, subdiv)
            .map_err(|e| format!("transform_hull: {e}"))?;
        let new_draft = out.settings.draft;
        self.surface = out;
        if new_draft > 0.0 {
            self.opts.draft = new_draft;
        }
        Ok(json!({
            "backend": "headless",
            "report": report,
            "draft": self.opts.draft,
            "faces": self.surface.faces.len(),
            "points": self.surface.points.len(),
        }))
    }

    fn optimize_hull(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let spec = optimize::OptimizeSpec::from_params(a, Some(self.kg))
            .map_err(|e| format!("optimize_hull: {e}"))?;
        let mut opts = self.opts.clone();
        if a.contains_key("level") {
            opts.subdivision.level = level_arg(a)?;
        }
        let (out, report) = optimize::optimize_hull(&self.surface, &opts, &spec)
            .map_err(|e| format!("optimize_hull: {e}"))?;
        // The optimised hull is measured at the draft that holds the original
        // displacement; carry it across or every number after this would be
        // read at the old waterline.
        self.opts.draft = report.optimised.draft_m;
        self.surface = out;
        Ok(json!({
            "backend": "headless",
            "level": opts.subdivision.level,
            "kg": spec.kg,
            "report": report,
            "draft": self.opts.draft,
            "faces": self.surface.faces.len(),
            "points": self.surface.points.len(),
        }))
    }

    fn trim_scan(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let mut opts = self.opts.clone();
        if let Some(d) = number_arg(a, "draft") {
            if !(d.is_finite() && d > 0.0) {
                return Err("trim_scan: `draft` must be positive".into());
            }
            opts.draft = d;
        }
        if a.contains_key("level") {
            opts.subdivision.level = level_arg(a)?;
        }
        let speed = number_arg(a, "speed_kn").ok_or("trim_scan: give `speed_kn`")?;
        let from = number_arg(a, "from").unwrap_or(-0.6);
        let to = number_arg(a, "to").unwrap_or(0.6);
        if !(to.is_finite() && from.is_finite() && to > from) {
            return Err("trim_scan: `to` must be greater than `from`".into());
        }
        let steps = match a.get("steps") {
            None | Some(Value::Null) => 13,
            Some(_) => match number_arg(a, "steps") {
                Some(n) if (2.0..=200.0).contains(&n) => n.round() as usize,
                _ => return Err("trim_scan: `steps` must be from 2 to 200".into()),
            },
        };
        let hold = bool_arg(a, "hold_volume").unwrap_or(true);
        let trims = trimopt::trim_range(from, to, steps);
        let scan = trimopt::scan(&self.surface, opts.subdivision, &opts, speed, &trims, hold)
            .map_err(|e| format!("trim_scan: {e}"))?;
        Ok(json!({
            "backend": "headless",
            "level": opts.subdivision.level,
            "speed_kn": scan.speed_kn,
            "target_volume_m3": scan.target_volume_m3,
            "at_boundary": scan.at_boundary,
            "best": scan.best,
            "warnings": scan.warnings,
            "points": scan.points,
        }))
    }

    fn hydro_table(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let mut opts = self.opts.clone();
        if let Some(to) = number_arg(a, "to") {
            if !(to.is_finite() && to > 0.0) {
                return Err("hydro_table: `to` must be a positive draft".into());
            }
            opts.draft = to;
        }
        if a.contains_key("level") {
            opts.subdivision.level = level_arg(a)?;
        }
        let drafts = match a.get("drafts") {
            None | Some(Value::Null) => {
                let steps = match a.get("steps") {
                    None | Some(Value::Null) => 10,
                    Some(_) => match number_arg(a, "steps") {
                        Some(n) if (1.0..=200.0).contains(&n) => n.round() as usize,
                        _ => return Err("hydro_table: `steps` must be from 1 to 200".into()),
                    },
                };
                table::draft_range(opts.draft, steps)
            }
            Some(v) => {
                let list = numbers(v, "drafts")?;
                if list.is_empty() {
                    return Err("hydro_table: `drafts` is empty".into());
                }
                if list.iter().any(|d| *d <= 0.0) {
                    return Err("hydro_table: `drafts` must be positive".into());
                }
                list
            }
        };
        let t = table::table(&self.surface, &opts, &drafts);
        Ok(json!({
            "backend": "headless",
            "level": opts.subdivision.level,
            "name": t.name,
            "water_density": t.water_density,
            "drafts": t.rows.len(),
            "rows": t.rows,
        }))
    }

    fn unfold_plate(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let level = match a.get("level") {
            None | Some(Value::Null) => self.opts.subdivision.level,
            Some(_) => level_arg(a)?,
        };
        let axis = match str_arg(a, "axis") {
            None => develop::BandAxis::Z,
            Some(name) => develop::BandAxis::parse(name)
                .ok_or_else(|| format!("unfold_plate: `axis` must be x, y or z (got `{name}`)"))?,
        };
        let mut subdiv = self.opts.subdivision;
        subdiv.level = level;
        let hull = hydro::Hull::new(&self.surface, subdiv);
        let pick = |axis: develop::BandAxis, given: Option<f64>, low: bool| {
            given.unwrap_or(match (axis, low) {
                (develop::BandAxis::X, true) => hull.model_min.x,
                (develop::BandAxis::X, false) => hull.model_max.x,
                (develop::BandAxis::Y, true) => hull.model_min.y,
                (develop::BandAxis::Y, false) => hull.model_max.y,
                (develop::BandAxis::Z, true) => hull.keel,
                (develop::BandAxis::Z, false) => hull.model_max.z,
            })
        };
        let selection = develop::PatchSelection {
            slabs: vec![
                (
                    axis,
                    pick(axis, number_arg(a, "from"), true),
                    pick(axis, number_arg(a, "to"), false),
                ),
                (
                    develop::BandAxis::X,
                    pick(develop::BandAxis::X, number_arg(a, "x_from"), true),
                    pick(develop::BandAxis::X, number_arg(a, "x_to"), false),
                ),
            ],
            starboard_only: !bool_arg(a, "both_sides").unwrap_or(false),
        };
        let faces = develop::select(&hull.verts, &hull.tris, &selection);
        if faces.is_empty() {
            return Err("unfold_plate: the band selected no faces; widen the range".into());
        }
        let unfolded = develop::unfold(&hull.verts, &faces);
        let mut written = None;
        if let Some(path) = str_arg(a, "path").filter(|s| !s.trim().is_empty()) {
            let title = format!(
                "{} - plate on {:?} {:.2}..{:.2} m",
                self.surface.settings.name, axis, selection.slabs[0].1, selection.slabs[0].2
            );
            std::fs::write(path, develop::svg(&unfolded, &title))
                .map_err(|e| format!("unfold_plate: cannot write {path}: {e}"))?;
            written = Some(path.to_string());
        }
        Ok(json!({
            "backend": "headless",
            "level": level,
            "faces": faces.len(),
            "plate": unfolded,
            "written": written,
        }))
    }

    fn export_linesplan(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let path = str_arg(a, "path")
            .filter(|p| !p.trim().is_empty())
            .ok_or("export_linesplan: give `path` (.svg, .dxf or .igs)")?;
        let path = PathBuf::from(path);
        let opts = linesplan::LinesplanOptions {
            stations: count_arg(a, "stations", 11, 1, 200)?,
            buttocks: count_arg(a, "buttocks", 5, 0, 200)?,
            waterlines: count_arg(a, "waterlines", 7, 0, 200)?,
            level: level_arg(a)?,
            mode: SubdivisionMode::Loop,
            combs: bool_arg(a, "combs").unwrap_or(false),
            comb_scale: number_arg(a, "comb_scale").unwrap_or(0.6),
        };
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let (format, text) = match extension.as_str() {
            "dxf" => ("dxf", linesplan::dxf(&self.surface, opts)),
            "igs" | "iges" => ("igs", linesplan::iges(&self.surface, opts)),
            "svg" | "" => ("svg", linesplan::svg(&self.surface, opts)),
            other => {
                return Err(format!(
                    "export_linesplan: unsupported extension `{}` (use .svg, .dxf or .igs)",
                    clip(other, 20)
                ))
            }
        };
        std::fs::write(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(json!({
            "path": path.to_string_lossy(),
            "format": format,
            "bytes": text.len(),
            "stations": opts.stations,
            "buttocks": opts.buttocks,
            "waterlines": opts.waterlines,
            "combs": opts.combs,
        }))
    }

    fn render_png(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let path = str_arg(a, "path")
            .filter(|p| !p.trim().is_empty())
            .ok_or("render_png: give `path`")?;
        let path = PathBuf::from(path);
        let mode_name = str_arg(a, "mode").unwrap_or("shaded").to_ascii_lowercase();
        let mode = match mode_name.as_str() {
            "shaded" => AnalysisMode::Shaded,
            "zebra" => AnalysisMode::Zebra {
                stripes: number_arg(a, "stripes").unwrap_or(14.0).clamp(2.0, 200.0),
            },
            "curvature" => AnalysisMode::Curvature {
                radius: number_arg(a, "radius").unwrap_or(1.0),
            },
            "draft" => AnalysisMode::Draft {
                angle_deg: number_arg(a, "draft_angle").unwrap_or(3.0),
            },
            other => {
                return Err(format!(
                    "render_png: unknown `mode` `{}` (shaded, zebra, curvature, draft)",
                    clip(other, 40)
                ))
            }
        };
        let view_name = str_arg(a, "view")
            .unwrap_or("perspective")
            .to_ascii_lowercase();
        let (min, max) = self.surface.extents();
        let centre = (min + max) * 0.5;
        let size = (max - min).length().max(0.5);
        let view = match view_name.as_str() {
            "perspective" => raster::View::perspective(centre, size, 2.2, 0.30, 45.0),
            "profile" => raster::View::ortho(Vec3::new(0.0, 1.0, 0.0), centre, size),
            "plan" => raster::View::ortho(Vec3::new(0.0, 0.0, -1.0), centre, size),
            "body" => raster::View::ortho(Vec3::new(1.0, 0.0, 0.0), centre, size),
            other => {
                return Err(format!(
                    "render_png: unknown `view` `{}` (perspective, profile, plan, body)",
                    clip(other, 40)
                ))
            }
        };
        let width = side_arg(a, "width", DEFAULT_SIZE.0);
        let height = side_arg(a, "height", DEFAULT_SIZE.1);
        let level = level_arg(a)?;
        let mesh = export::render_mesh(&self.surface, level, SubdivisionMode::Loop);
        let opts = raster::RenderOptions {
            width,
            height,
            mode,
            supersample: 2,
            ..Default::default()
        };
        let canvas = raster::render(&mesh.verts, &mesh.faces, &view, &opts);
        let bytes = raster::png(canvas.w, canvas.h, &canvas.rgba)
            .ok_or_else(|| format!("render_png: cannot encode a {width}x{height} PNG"))?;
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": bytes.len(),
            "width": width,
            "height": height,
            "mode": mode.label(),
            "view": view_name,
            "faces": mesh.faces.len(),
            "level": level,
        }))
    }

    fn design_report(&mut self, a: &Map<String, Value>) -> Result<Value, String> {
        let path = str_arg(a, "path")
            .filter(|p| !p.trim().is_empty())
            .ok_or("design_report: give `path`")?;
        let path = PathBuf::from(path);

        let mut opts = seacraft_core::report::ReportOptions::from_surface(&self.surface);
        if let Some(d) = number_arg(a, "draft") {
            if d <= 0.0 {
                return Err("design_report: `draft` must be positive".into());
            }
            opts.draft = d;
        }
        if let Some(kg) = number_arg(a, "kg") {
            if kg < 0.0 {
                return Err("design_report: `kg` must be non-negative".into());
            }
            opts.kg = kg;
        }
        if let Some(lcg) = number_arg(a, "lcg") {
            opts.lcg_x = Some(lcg);
        }
        if let Some(v) = a.get("speeds_kn").or_else(|| a.get("speeds")) {
            opts.speeds_kn = positives(v, "speeds_kn")?;
        }
        if let Some(nr) = bool_arg(a, "include_resistance") {
            opts.include_resistance = nr;
        }
        if let Some(pl) = bool_arg(a, "include_planing") {
            opts.include_planing = pl;
        }
        if a.get("level").is_some() {
            opts.level = level_arg(a)?;
        } else {
            opts.level = self.opts.subdivision.level;
        }
        opts.mode = self.opts.subdivision.mode;

        let html = seacraft_core::report::design_report(&self.surface, &opts)
            .map_err(|e| format!("design_report: {e}"))?;
        std::fs::write(&path, html.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;

        let sections = seacraft_core::report::report_sections(&opts);

        let subdiv = SubdivisionOptions {
            level: opts.level,
            mode: opts.mode,
        };
        let hull = hydro::Hull::new(&self.surface, subdiv);
        let h_opts = hydro::HydroOptions::from_surface(&self.surface, subdiv);
        let cond = StabilityCondition {
            kg: opts.kg,
            trim: 0.0,
            lcg_x: opts.lcg_x.or(Some(upright_lcb(&hull, opts.draft))),
        };
        let target_disp =
            hull.displaced_volume(opts.draft, 0.0, 0.0) * h_opts.water_density / 1000.0;
        let curves = cross_curves(&hull, &opts.heels, &[target_disp], &cond, &h_opts);
        let imo_pass = evaluate_imo_criteria(&curves, 0)
            .map(|c| c.passed)
            .unwrap_or(false);

        Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": html.len(),
            "sections": sections,
            "imo_pass": imo_pass,
            "imo_passed": imo_pass,
        }))
    }

    /// The floating condition a request asks for, applied to `opts`.
    fn apply_condition(
        &self,
        a: &Map<String, Value>,
        opts: &mut HydroOptions,
    ) -> Result<(), String> {
        if let Some(d) = number_arg(a, "draft") {
            opts.draft = d;
        }
        if let Some(t) = number_arg(a, "trim") {
            opts.trim = t;
        }
        if let Some(h) = number_arg(a, "heel_deg") {
            if !(-180.0..=180.0).contains(&h) {
                return Err("`heel_deg` must be between -180 and 180".to_string());
            }
            opts.heel_deg = h;
        }
        if let Some(d) = number_arg(a, "density") {
            if !d.is_finite() || d <= 0.0 {
                return Err("`density` must be positive kg/m3".to_string());
            }
            opts.water_density = d;
        }
        if a.contains_key("stations") {
            opts.stations = count_arg(a, "stations", opts.stations, 1, 400)?;
        }
        if a.contains_key("level") {
            opts.subdivision.level = level_arg(a)?;
        }
        // The midship station: absent leaves the model's own setting, a
        // positive value measures there, and 0 asks for the LCF.
        if let Some(x) = number_arg(a, "mainframe_x") {
            opts.mainframe_x = if x > 0.0 { Some(x) } else { None };
        }
        Ok(())
    }
}

/// A subdivision setting with the server's current scheme and `level`.
fn seascape_subdiv(level: u8) -> seacraft_core::subdiv::SubdivisionOptions {
    seacraft_core::subdiv::SubdivisionOptions {
        level: level.min(MAX_LEVEL),
        mode: SubdivisionMode::Loop,
    }
}

fn read_fef(path: &Path) -> Result<Surface, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    fef::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

// -------------------------------------------------------------- arguments --

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn str_arg<'a>(a: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}

fn number_arg(a: &Map<String, Value>, key: &str) -> Option<f64> {
    a.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn bool_arg(a: &Map<String, Value>, key: &str) -> Option<bool> {
    a.get(key).and_then(Value::as_bool)
}

fn usize_arg(a: &Map<String, Value>, key: &str) -> Option<usize> {
    a.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

/// A count clamped to `lo..=hi`, defaulting to `default`.
fn count_arg(
    a: &Map<String, Value>,
    key: &str,
    default: usize,
    lo: usize,
    hi: usize,
) -> Result<usize, String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(default.clamp(lo, hi)),
        Some(v) => match v.as_u64().and_then(|n| usize::try_from(n).ok()) {
            Some(n) => Ok(n.clamp(lo, hi)),
            None => Err(format!("`{key}` must be a whole number")),
        },
    }
}

/// A picture side in pixels, clamped so a hostile size draws a picture rather
/// than allocating terabytes.
fn side_arg(a: &Map<String, Value>, key: &str, default: usize) -> usize {
    a.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(default)
        .clamp(16, MAX_SIDE)
}

fn level_arg(a: &Map<String, Value>) -> Result<u8, String> {
    Ok(count_arg(a, "level", 3, 0, MAX_LEVEL as usize)? as u8)
}

/// An array of finite numbers, from `key`'s value.
fn numbers(v: &Value, key: &str) -> Result<Vec<f64>, String> {
    let Some(items) = v.as_array() else {
        return Err(format!("`{key}` must be an array of numbers"));
    };
    if items.len() > 10_000 {
        return Err(format!("`{key}` has too many entries (at most 10000)"));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item.as_f64().filter(|n| n.is_finite()) {
            Some(n) => out.push(n),
            None => return Err(format!("`{key}` must contain only finite numbers")),
        }
    }
    Ok(out)
}

/// A non-empty list of positive ordinates, sorted ascending.
fn positives(v: &Value, key: &str) -> Result<Vec<f64>, String> {
    let mut out = numbers(v, key)?;
    if out.is_empty() {
        return Err(format!("`{key}` needs at least one ordinate"));
    }
    if out.iter().any(|x| *x <= 0.0) {
        return Err(format!("`{key}` must be positive"));
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

// ------------------------------------------------------ tool definitions --

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn number(desc: &str) -> Value {
    json!({"type": "number", "description": desc})
}

fn integer(desc: &str) -> Value {
    json!({"type": "integer", "description": desc})
}

fn boolean(desc: &str) -> Value {
    json!({"type": "boolean", "description": desc})
}

fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

/// What a tool does to the world, which fixes its MCP annotation hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Measures or reports; changes nothing (`unfold_plate` writes a file only
    /// when asked, as a side product of the answer).
    ReadOnly,
    /// Changes the loaded model in place, additively or by small moves.
    Edits,
    /// Throws the loaded model away and substitutes another.
    Replaces,
    /// Writes a file on disk and leaves the model alone.
    Files,
}

impl Effect {
    /// Recover the effect from a tool definition's annotations.
    fn of(def: &Value) -> Effect {
        let a = &def["annotations"];
        if a["readOnlyHint"] == true {
            Effect::ReadOnly
        } else if a["destructiveHint"] == true {
            Effect::Replaces
        } else if a["idempotentHint"] == true {
            Effect::Files
        } else {
            Effect::Edits
        }
    }

    fn label(self) -> &'static str {
        match self {
            Effect::ReadOnly => "read-only",
            Effect::Edits => "edits the model",
            Effect::Replaces => "replaces the model",
            Effect::Files => "writes files",
        }
    }
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, effect: Effect) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {
            "title": title,
            "readOnlyHint": effect == Effect::ReadOnly,
            "destructiveHint": effect == Effect::Replaces,
            "idempotentHint": matches!(effect, Effect::ReadOnly | Effect::Replaces | Effect::Files),
            "openWorldHint": false,
        },
    })
}

/// Every tool, with its JSON Schema.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "hull_info",
            "Inspect the hull",
            "The model: name and principal dimensions, extents (x/y/z min and max, keel height), control point, \
             face and edge counts (crease, boundary, non-manifold), the face mix, the control-net area, every \
             layer with its symmetric and hydrostatics flags, and whether the model validates.",
            obj(json!({}), &[]),
            Effect::ReadOnly,
        ),
        tool(
            "load_hull",
            "Load a hull",
            "Replace the model: `path` reads a FREE!ship `.fef` file, or `demo: true` builds the built-in 12 m \
             demo hull (`symmetric` cuts it to the starboard half). Returns hull_info.",
            obj(
                json!({
                    "path": string("Path to a .fef file"),
                    "demo": boolean("Build the built-in demo hull instead of reading a file"),
                    "symmetric": boolean("For `demo`: model only the starboard half (default false)"),
                }),
                &[],
            ),
            Effect::Replaces,
        ),
        tool(
            "generate_hull",
            "Generate a parametric hull",
            "Replace the model with a parametric symmetric half-model control net built from principal dimensions \
             (`displacement`, `planing` or `yacht`), scaled so level-3 LWL, BWL and T match the specification and \
             optionally solved for target block coefficient `cb` and LCB fraction `lcb_frac`. Returns the generation report.",
            obj(
                json!({
                    "kind": {"type": "string", "enum": ["displacement", "planing", "yacht"], "description": "Hull family (default displacement)"},
                    "lwl": number("Waterline length LWL in metres"),
                    "beam": number("Waterline beam BWL in metres"),
                    "draft": number("Canoe-body draft T at midship in metres"),
                    "depth": number("Moulded depth from keel to sheer at midship in metres"),
                    "cb": number("Target block coefficient Cb (0.35..=0.85)"),
                    "lcb_frac": number("Target LCB as fraction of LWL from the aft perpendicular (0.35..=0.60)"),
                    "lcb": number("Alias for lcb_frac (0.35..=0.60)"),
                    "deadrise_deg": number("Bottom deadrise angle in degrees (planing only, default 15)"),
                    "deadrise": number("Alias for deadrise_deg"),
                    "transom_frac": number("Transom immersion / beam ratio (0..=1)"),
                    "transom": number("Alias for transom_frac"),
                    "stations": integer("Number of longitudinal control stations (default 9)"),
                    "points_per_station": integer("Number of control points per station (default 5)"),
                }),
                &[],
            ),
            Effect::Replaces,
        ),
        tool(
            "save_hull",
            "Save the hull",
            "Write the control net as a FREE!ship `.fef` file. Without `path` the model is written back to the \
             file it came from.",
            obj(json!({"path": string("Output .fef path")}), &[]),
            Effect::Files,
        ),
        tool(
            "project_get",
            "Get the project sidecar",
            "Return the design data sidecar (loading conditions, compartments, damage cases, windage, and report settings).",
            obj(json!({}), &[]),
            Effect::ReadOnly,
        ),
        tool(
            "project_set",
            "Set the project sidecar",
            "Replace the design data sidecar. `project` must be a valid seacraft-project JSON object with format and version 1.",
            obj(
                json!({
                    "project": {"type": "object", "description": "The complete project sidecar object"},
                }),
                &["project"],
            ),
            Effect::Edits,
        ),
        tool(
            "move_point",
            "Move a control point",
            "Set control point `index` to `xyz` in metres (x forward, y starboard, z up). On a symmetric layer y \
             is clamped to 0 or above, because only the starboard half exists and the port side is mirrored. \
             Returns the point before and after.",
            obj(
                json!({
                    "index": integer("Control point index (see hull_info for the count)"),
                    "xyz": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "New position [x, y, z] in metres"},
                }),
                &["index", "xyz"],
            ),
            Effect::Edits,
        ),
        tool(
            "set_crease",
            "Set an edge crease",
            "Mark the edge between control points `a` and `b` as a crease (a hard edge FREE!ship keeps sharp through \
             subdivision) or clear it. The two points must already share an edge.",
            obj(
                json!({
                    "a": integer("First control point index"),
                    "b": integer("Second control point index"),
                    "crease": boolean("true marks a crease, false clears it (default true)"),
                }),
                &["a", "b"],
            ),
            Effect::Edits,
        ),
        tool(
            "split_face",
            "Split a face",
            "Replace control face `face` with a triangle fan around a new point at its centroid. The face keeps its \
             index and layer, so a later `insert_edge` naming the same index refers to the first triangle of the \
             fan. Every new edge is interior to the face, so the net stays closed and manifold; on a planar face \
             the geometry is untouched, on a curved one the fan refines to a slightly different surface.",
            obj(json!({"face": integer("Control face index (see hull_info for the count)")}), &["face"]),
            Effect::Edits,
        ),
        tool(
            "insert_edge",
            "Insert an edge",
            "Join two non-adjacent points of control face `face` with a new edge, splitting the face in two. Both \
             points must already be on that face and must not be neighbours along it. The halves keep the original \
             face's layer and the surface is unchanged: only the topology is divided.",
            obj(
                json!({
                    "face": integer("Control face index"),
                    "a": integer("First control point index, on that face"),
                    "b": integer("Second control point index, on that face"),
                }),
                &["face", "a", "b"],
            ),
            Effect::Edits,
        ),
        tool(
            "hydro_compute",
            "Hydrostatics",
            "Measure one floating condition on the refined mesh: volume, displacement, the coefficients (Cb, Cp, Cm, \
             Cwp), LCB, KB, LCF, waterplane area and inertias, BM/KM, the midship section, wetted surface and the \
             section area curve, as the full Hydrostatics object plus `gm_m` for the given KG. `free_trim` (or `lcg`) \
             solves the trim that puts the LCB on the LCG.",
            obj(
                json!({
                    "draft": number("Design draft in metres (default: the model's)"),
                    "trim": number("Trim in metres, positive bow-up (default 0)"),
                    "heel_deg": number("Heel in degrees, positive to starboard (default 0)"),
                    "kg": number("Vertical centre of gravity above the keel, for GM (default: the server's)"),
                    "free_trim": boolean("Solve the trim to longitudinal equilibrium instead of holding `trim`"),
                    "lcg": number("Longitudinal centre of gravity in model x; implies free trim"),
                    "density": number("Water density in kg/m3 (default: the model's)"),
                    "stations": integer("Stations for the section area curve (default 21)"),
                    "mainframe_x": number("Station to measure the midship section and Cm/Cp at; 0 or omitted uses the LCF"),
                    "level": integer("Subdivision level 0-6 (default 3)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "stability_cross_curves",
            "Cross curves of stability",
            "Solve KN and GZ over the heel x displacement grid, with the IMO A.749 / 2008 IS Code Part A criteria \
             (areas 0-30, 0-40, 30-40, maximum GZ above 30 deg, its angle, GM0). `free_trim` (or `lcg`) lets the hull \
             trim to keep the LCB on the LCG while heeling; otherwise `trim` is held.",
            obj(
                json!({
                    "kg": number("Vertical centre of gravity above the keel (default: the server's)"),
                    "trim": number("Trim held while heeling, metres bow-up (default 0)"),
                    "draft": number("Draft used for the default displacements (default: the model's)"),
                    "free_trim": boolean("Trim freely to longitudinal equilibrium while heeling"),
                    "lcg": number("Longitudinal centre of gravity in model x; implies free trim"),
                    "heels": {"type": "array", "items": {"type": "number"}, "description": "Heel angles in degrees (default 0,10,...,90)"},
                    "displacements": {"type": "array", "items": {"type": "number"}, "description": "Displacements in tonnes (default: half, design, 1.25x)"},
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "loading_condition",
            "Loading condition",
            "Evaluate a ship loading condition (lightship, deadweight weight items, and partially filled tanks with \
             optional hull clipping): sums mass, LCG, TCG and solid KG, integrates each tank's liquid centroid and \
             free-surface moment FSM = rho * i_t, applies the free-surface correction FSC = FSM / Delta to obtain \
             KG_fluid, solves the free-trim floating condition (draft, trim, list angle), and evaluates GZ cross \
             curves and IMO A.749 intact-stability criteria at KG_fluid. Pass `case` (inline LoadingCondition \
             JSON object), `case_path` (path to a JSON file), or `name` (name of a condition in the project sidecar; \
             when neither case nor case_path is given, defaults to the project's named condition or the first condition).",
            obj(
                json!({
                    "case": {"type": "object", "description": "Inline LoadingCondition object {name, lightship, items, tanks}"},
                    "case_path": string("Path to a LoadingCondition JSON file"),
                    "name": string("Name of a loading condition in the project sidecar (used when case and case_path are omitted)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "weather_criterion",
            "Weather criterion (IMO IS Code §2.3)",
            "Evaluate the IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather) criterion on the refined \
             hull: measures the hull's lateral windage profile above the waterline and underwater lateral area, adds \
             any superstructure `windage_extra` items, computes steady and gust wind heeling levers lw1 and lw2, \
             interpolates Table 2.3.4-1..4 roll factors (X1, X2, k, r, s, T, C) and roll-back angle phi1, samples the \
             GZ curve at 2-deg steps from -30 to +60 deg, and integrates heeling energy area `a` and righting energy \
             area `b` to check `phi0 <= 16 deg` and `b >= a`.",
            obj(
                json!({
                    "draft": number("Design draft in metres (default: the model's)"),
                    "trim": number("Trim in metres, positive bow-up (default 0)"),
                    "kg": number("Vertical centre of gravity above the keel in metres (default: the server's)"),
                    "free_trim": boolean("Trim freely to longitudinal equilibrium while heeling"),
                    "lcg": number("Longitudinal centre of gravity in model x; implies free trim"),
                    "bilge": string("Bilge kind: 'round', 'sharp', or 'keels:<Ak>' (default 'round')"),
                    "bilge_keel_area_m2": number("Bilge/bar keel total lateral area Ak in m2 when bilge is 'keels'"),
                    "windage_extra": {"type": "array", "items": {"type": "object"}, "description": "Additional windage items [{name, area_m2, z_centroid}]"},
                    "downflooding_deg": number("Downflooding angle phi_f in degrees (optional)"),
                    "wind_pressure_pa": number("Steady wind pressure P in Pa (default 504)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "damage_stability",
            "Damage stability (lost-buoyancy method)",
            "Evaluate deterministic damage stability by the lost-buoyancy method for a damaged ship condition. \
             Flooded compartment volume is deducted from the buoyant hull, holding displacement and KG constant. \
             Solves damaged equilibrium (draft, trim, heel/list), checks freeboard to the deck edge, samples the \
             residual GZ curve (heels 0-60 deg, free trim), and evaluates SOLAS II-1 Reg. 7-2 indicative criteria \
             (positive range >= 15 deg, max GZ >= 0.10 m, area >= 0.015 m*rad). \
             Evaluate a case by `name` from the project sidecar, or pass an inline `case` or `case_path`.",
            obj(
                json!({
                    "name": string("Name of a damage case in the project (defaults to the first case when neither case nor case_path is given)"),
                    "case": {"type": "object", "description": "Inline DamageCase object {name, compartments, kg, displacement_t}"},
                    "case_path": string("Path to a DamageCase JSON file"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "seakeeping_raos",
            "Seakeeping RAOs (heave and pitch in regular head seas)",
            "Linear strip-theory heave and pitch motion Response Amplitude Operators (RAOs) in regular head seas \
             following Salvesen, Tuck & Faltinsen (1970). Computes 2D heave added mass and damping per section via \
             Frank close-fit panels, global radiation coefficients (A33..B55) with forward speed terms, hydrostatic \
             restoring (C33..C55), and wave excitation forces (Froude-Krylov + diffraction). Returns RAO points across \
             the lambda/L range, natural heave and pitch frequencies, and global coefficients.",
            obj(
                json!({
                    "draft": number("Design draft in metres (defaults to model's design draft)"),
                    "speed_kn": number("Ship forward speed in knots (default 0.0)"),
                    "kg": number("Vertical centre of gravity KG in metres above keel (default 0.0)"),
                    "kyy": number("Pitch radius of gyration in metres (default 0.25 * Lwl)"),
                    "stations": integer("Number of transverse stations along waterline (default 21)"),
                    "lambda_min": number("Minimum wavelength ratio lambda / L (default 0.3)"),
                    "lambda_max": number("Maximum wavelength ratio lambda / L (default 3.0)"),
                    "points": integer("Number of frequency points across lambda / L (default 30)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "seakeeping_irregular",
            "Seakeeping in irregular head seas (significant motions, accelerations, MSI)",
            "Heave, pitch, vertical motion, motion relative to the wave surface, vertical acceleration and the motion \
             sickness incidence in a long-crested irregular head sea, from the strip-theory RAOs of seakeeping_raos \
             integrated against a Bretschneider (default) or JONSWAP (give gamma) spectrum over the incident frequency. \
             Significant amplitude is 2*sqrt(m0); Tz is the mean zero-crossing encounter period. MSI is the share of \
             unadapted people expected to vomit within 2 hours (O'Hanlon & McCauley 1974) from the mean absolute \
             acceleration 0.798*sigma and its mean zero-crossing encounter frequency. Responses are reported at \
             `points` ([{name, x}] in model x; default LCG and Bow = forward end of the design waterline). \
             Heave in m, pitch in degrees (bow up), accelerations in m/s2 and g.",
            obj(
                json!({
                    "hs": number("Significant wave height in metres"),
                    "tp": number("Peak wave period in seconds"),
                    "gamma": number("JONSWAP peak enhancement factor 1-20 (omit for Bretschneider)"),
                    "draft": number("Design draft in metres (defaults to model's design draft)"),
                    "speed_kn": number("Ship forward speed in knots (default 0.0)"),
                    "kg": number("Vertical centre of gravity KG in metres above keel (default 0.0)"),
                    "kyy": number("Pitch radius of gyration in metres (default 0.25 * Lwl)"),
                    "stations": integer("Number of transverse stations along waterline, 3-200 (default 21)"),
                    "points": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "x": {"type": "number"}}, "required": ["x"]}, "description": "Evaluation points {name, x} with x in model coordinates, metres (default LCG and Bow)"},
                    "omegas": integer("Frequencies across 0.2..5 times the peak frequency, 8-2000 (default 60)"),
                }),
                &["hs", "tp"],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "bonjean_curves",
            "Bonjean curves",
            "Section area against draft at every longitudinal station, cut from the refined mesh at even keel: the \
             standard curve for grounding and damage-draft work. `drafts` fixes the ordinates in metres above the \
             keel; otherwise `draft_steps` drafts spread evenly from depth/steps to the hull's own depth are used. \
             Stations sit at slice midpoints, so no cut lands on an end face. Returns `drafts_m`, one section per \
             station (`x`, `areas_m2` in the same order), the maximum area and the hull depth.",
            obj(
                json!({
                    "stations": integer("Stations along the model length, 2-200 (default 21)"),
                    "drafts": {"type": "array", "items": {"type": "number"}, "description": "Draft ordinates in metres, positive and ascending after sorting (default: depth/steps ... depth)"},
                    "draft_steps": integer("Number of drafts in the default range, 2-50 (default 10)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "table_of_offsets",
            "Table of offsets",
            "The drawing-office table: half-breadths where each station section meets each waterline, plus the \
             height of the hull on each buttock plane. Stations are sampled at slice midpoints and every reading \
             comes from the refined mesh. `waterlines` are heights above the keel in metres and `buttocks` are \
             offsets to starboard from the centreline; omit either to get `waterline_steps`/`buttock_steps` \
             evenly spaced ordinates over the hull's own depth and half beam. A cell is null when the section does \
             not reach that ordinate (printed as `-`, which is not the same as 0.000 on the centreline).",
            obj(
                json!({
                    "stations": integer("Stations along the model length, 2-200 (default 21)"),
                    "waterlines": {"type": "array", "items": {"type": "number"}, "description": "Waterline heights above the keel, metres (default: depth/steps ... depth)"},
                    "waterline_steps": integer("Number of waterlines in the default range, 1-50 (default 6)"),
                    "buttocks": {"type": "array", "items": {"type": "number"}, "description": "Buttock offsets from the centreline, metres (default: spread over the half beam)"},
                    "buttock_steps": integer("Number of buttocks in the default range, 1-50 (default 4)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "resistance_curve",
            "Resistance curve",
            "Still-water resistance over a speed sweep: Holtrop-Mennen empirical method, Michell thin-ship \
             wave resistance, or Delft Systematic Yacht Hull Series (DSYHS) sailing yacht bare-hull resistance. Returns RT, components (RF, RW, 1+k), effective power PE, and warnings. Supports \
             catamarans via `cat_spacing` when `method: michell`. Give `propeller_diameter` to also get \
             propulsion factors.",
            obj(
                json!({
                    "method": string("Resistance method: 'holtrop' (default), 'michell', or 'dsyhs'"),
                    "cat_spacing": number("Demihull centreline separation in metres for catamarans (Michell method only)"),
                    "draft": number("Draft in metres (default: the model's design draft)"),
                    "speeds": {"type": "array", "items": {"type": "number"}, "description": "Speeds in knots (default 6,8,10,12,14)"},
                    "cstern": number("Holtrop stern parameter, -10 to +10 (default 0)"),
                    "bulb_area": number("Immersed bulbous-bow area in m2 (default 0)"),
                    "bulb_z": number("Height of the bulb centre above the keel, m"),
                    "propeller_diameter": number("Propeller diameter in metres; enables the power chain"),
                    "expanded_area": number("Propeller expanded area ratio AE/A0 (default 0.70)"),
                    "eta_o": number("Open-water efficiency from an open-water chart (default 0.60)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "planing_curve",
            "Planing curve",
            "Savitsky planing resistance over a speed sweep: trim, mean wetted length, wetted length in metres, \
             centre of pressure over beam, drag and effective power, with the trim solved so the pressure centre \
             sits under the CG. Weight comes from the hydrostatics at `draft`, the CG defaults to the LCB and \
             `deadrise` to the transom section's keel tangent. Speeds where the hull does not plane are omitted.",
            obj(
                json!({
                    "draft": number("Draft the displacement is taken at (default: the model's)"),
                    "speeds": {"type": "array", "items": {"type": "number"}, "description": "Speeds in knots (default 20,25,30)"},
                    "beam": number("Planing beam in metres (default: the waterline beam)"),
                    "lcg_x": number("Centre of gravity in model x (default: the LCB)"),
                    "deadrise": number("Deadrise at the transom in degrees (default: measured)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "transform_hull",
            "Transform the hull",
            "Scale the model to target main dimensions, apply an extra axis scale, translate it, or mirror it. \
             The scale is anchored at the aft end, the centreline and the keel, and a mirror reverses every \
             face's winding so the normals still point out. Order is fixed: fit, scale, translate, mirror. \
             The project's length, beam and draft move with it, and so does the measurement draft. Returns the \
             before/after dimensions and the cumulative scale.",
            obj(
                json!({
                    "lwl": number("Target waterline length in metres"),
                    "bwl": number("Target waterline beam in metres"),
                    "draft": number("Target draft in metres above the keel"),
                    "scale": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "Extra scale [x, y, z] applied after the fit"},
                    "translate": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "Translation [dx, dy, dz] in metres"},
                    "mirror": {"type": "string", "enum": ["x", "y", "z"], "description": "Reflect in this axis"},
                }),
                &[],
            ),
            Effect::Edits,
        ),
        tool(
            "unfold_plate",
            "Unfold a plate",
            "Develop a hull patch into the flat sheet a plate would be cut from. The patch is a simply \
             connected plate: a band across `axis` (x stations, y buttocks or z waterlines) between `from` and \
             `to`, further limited to the run `x_from`..`x_to` along the hull, and to the starboard side unless \
             `both_sides`. Returns the flat pattern size, the 3D and 2D areas, the seam gap (max, rms, and as a \
             fraction of the diagonal), the mean Gaussian curvature and a developable verdict at 0.5 % of the \
             diagonal. Give `path` to also write the pattern as SVG.",
            obj(
                json!({
                    "axis": {"type": "string", "enum": ["x", "y", "z"], "description": "Axis the band is cut across (default z, waterlines)"},
                    "from": number("Lower bound of the band in metres"),
                    "to": number("Upper bound of the band in metres"),
                    "x_from": number("Start of the plate's run along x (default: the model's minimum)"),
                    "x_to": number("End of the plate's run along x (default: the model's maximum)"),
                    "both_sides": boolean("Include the port side too (default false: one plate)"),
                    "path": string("Write the flat pattern SVG here"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "hydro_table",
            "Hydrostatics table",
            "Measure the hull once per draft and return the classic hydrostatics table: displacement, KB, BM, \
             BM_L, KM, LCB, LCF, the coefficients, tonnes per centimetre of immersion (TPC = A_w*rho/100) and \
             the moment to change trim one centimetre (MTC = rho*I_L/(100*L)). Rows are spread from just above \
             zero to `to` (default: the model's design draft) in `steps` equal increments, or at exactly the \
             drafts listed in `drafts`. The top row equals what `hydro_compute` reports at that draft.",
            obj(
                json!({
                    "to": number("Top draft in metres (default: the project draft)"),
                    "steps": integer("Number of drafts from just above zero to the top one (default 10)"),
                    "drafts": {"type": "array", "items": {"type": "number"}, "description": "Explicit drafts in metres (overrides `to` and `steps`)"},
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "trim_scan",
            "Trim scan",
            "Scan trim at a speed and report the cheapest, holding the displacement by solving the draft at each \
             trim (trimming about the centre of flotation changes the immersed volume by itself). Returns every \
             point (trim, draft, LWL, Fn, RT, PE), the cheapest one, whether that sits at the edge of the range \
             rather than at an interior minimum, and the model's warnings. Pass `hold_volume: false` to keep the \
             draft fixed instead and compare hulls of different weight.",
            obj(
                json!({
                    "speed_kn": number("Speed in knots (required)"),
                    "from": number("Aft limit of the trim range in metres (default -0.6)"),
                    "to": number("Forward limit of the trim range in metres (default 0.6)"),
                    "steps": integer("Number of trims to try (default 13)"),
                    "draft": number("Draft at which to hold the displacement (default: the server's)"),
                    "hold_volume": boolean("Solve the draft to hold the displacement (default true)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &["speed_kn"],
            ),
            Effect::ReadOnly,
        ),
        tool(
            "optimize_hull",
            "Optimise the hull for a speed",
            "Reshape the hull to cut calm-water resistance at `speed_kn` while holding displacement (and, with `kg` \
             and `min_gm`, stability). A Nelder-Mead search over up to three variables: `cp` (prismatic coefficient \
             change by a Lackenby station shift), `lcb` (LCB shift as a fraction of LWL, same shift) and `lb` (x \
             scaled by s and y by 1/s, so L/B changes by s squared); after each change the draft is re-solved so the \
             displacement holds. Scored by Holtrop-Mennen, or Savitsky for a planing hull (`method` auto picks \
             Savitsky at Fn_L >= 1), with penalties for displacement error, GM below `min_gm`, LCB outside \
             `lcb_band` and new validity warnings. EDITS THE MODEL: the session hull is replaced by the optimised \
             one and the measurement draft becomes the draft that holds the displacement; wrap the call in `batch` \
             to roll it back if a later step fails, or use the app's undo history when connected. Returns the \
             baseline and optimised R_T, P_E, displacement, Cp, Cb, LCB, L/B, GM and draft, the variables, \
             iterations and evaluations, why it stopped, warnings and the evaluation trajectory. `kg` defaults to \
             the session's.",
            obj(
                json!({
                    "speed_kn": number("Design speed in knots (required)"),
                    "method": {"type": "string", "enum": ["auto", "holtrop", "savitsky"], "description": "Resistance method (default auto)"},
                    "vars": {"type": "array", "items": {"type": "string", "enum": ["cp", "lcb", "lb"]}, "description": "Variables to optimise (default [\"cp\", \"lcb\"]; \"lb\" changes the length)"},
                    "bounds": {"type": "object", "description": "Bounds per variable as {\"cp\": [lo, hi]}; defaults cp +/-0.03, lcb +/-0.02, lb 0.92..1.08"},
                    "min_gm": number("Minimum GM = KM - KG in metres"),
                    "kg": number("KG above the keel in metres (default: the session's)"),
                    "lcb_band": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "Allowed LCB [lo, hi] as fractions of LWL from the aft end (0.5 is midships)"},
                    "hold_displacement_tol": number("Allowed relative displacement change (default 0.01)"),
                    "max_evaluations": integer("Cap on objective evaluations (default 60)"),
                    "draft": number("Draft the baseline is measured at (default: the server's)"),
                    "level": integer("Subdivision level 0-6 (default: the server's)"),
                }),
                &["speed_kn"],
            ),
            Effect::Edits,
        ),
        tool(
            "export_linesplan",
            "Export a linesplan",
            "Write the three-view linesplan. The extension picks the format: `.svg` (standalone drawing), `.dxf` \
             (AutoCAD 2000, one layer per view plus 3D copies) or `.igs` (B-spline curves).",
            obj(
                json!({
                    "path": string("Output path; .svg, .dxf or .igs"),
                    "stations": integer("Stations in the body plan (default 11)"),
                    "buttocks": integer("Buttocks in the profile view (default 5)"),
                    "waterlines": integer("Waterlines in the plan view (default 7)"),
                    "combs": boolean("Draw porcupine curvature combs (SVG and DXF)"),
                    "level": integer("Subdivision level 0-6 (default 3)"),
                }),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "render_png",
            "Render a preview",
            "Draw the hull with the CPU rasterizer and write a PNG: `shaded`, `zebra` (fairness stripes), \
             `curvature` or `draft` (mould draught angle), from `perspective`, `profile`, `plan` or `body`. Returns \
             the file size, the size drawn and the face count.",
            obj(
                json!({
                    "path": string("Output PNG path"),
                    "mode": {"type": "string", "enum": ["shaded", "zebra", "curvature", "draft"], "description": "Shading (default shaded)"},
                    "view": {"type": "string", "enum": ["perspective", "profile", "plan", "body"], "description": "Camera (default perspective)"},
                    "width": integer("Image width in pixels, 16-2048 (default 960)"),
                    "height": integer("Image height in pixels, 16-2048 (default 600)"),
                    "level": integer("Subdivision level 0-6 (default 3)"),
                }),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "design_report",
            "Generate a design dossier",
            "Produce a self-contained HTML5 design report of the hull (particulars, linesplan, hydrostatics table, \
             sectional area curve, GZ curve with IMO A.749 criteria, and Holtrop-Mennen resistance). \
             Returns path, bytes, sections present, and IMO pass/fail.",
            obj(
                json!({
                    "path": string("Output HTML path"),
                    "draft": number("Draft in metres (default: model's design draft)"),
                    "kg": number("Vertical centre of gravity above keel in metres"),
                    "lcg": number("Longitudinal centre of gravity in model x"),
                    "speeds_kn": {"type": "array", "items": {"type": "number"}, "description": "Speeds in knots for resistance evaluation"},
                }),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "batch",
            "Run several tool calls atomically",
            "Run tool calls in order, stopping at the first error. Each step is {name, arguments}. On an error earlier steps \
             are rolled back, so the batch is all-or-nothing (both headless and when connected to the GUI). Returns each \
             call's result on success, or {failed: {index, name, error}, completed: N} on failure. At most 100 calls; \
             nested batch is rejected.",
            obj(
                json!({
                    "calls": {
                        "type": "array",
                        "description": "Steps: [{\"name\": string, \"arguments\": {...}}, ...]",
                        "maxItems": 100,
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string", "description": "Tool name"},
                                "arguments": {"type": "object", "description": "Tool arguments"},
                            },
                            "required": ["name"],
                        },
                    },
                }),
                &["calls"],
            ),
            Effect::Edits,
        ),
    ]
}

/// The tools as a markdown list: name, title, first sentence, effect.
fn tool_catalogue() -> String {
    let mut out = String::from("# SeaCraft tools\n\n");
    for d in tool_definitions() {
        let desc = d["description"].as_str().unwrap_or_default();
        let first = desc.split_once(". ").map_or(desc, |(h, _)| h);
        out.push_str(&format!(
            "- `{}` ({}): {}. Effect: {}.\n",
            d["name"].as_str().unwrap_or_default(),
            d["title"].as_str().unwrap_or_default(),
            first.trim_end_matches('.'),
            Effect::of(&d).label(),
        ));
    }
    out
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    }
}

/// Check `args` against `name`'s schema: required keys present, no unknown
/// keys, each value of the declared type and inside its `enum`.
pub fn check_arguments(name: &str, args: &Value) -> Result<(), String> {
    #[cfg(test)]
    if name == "__test_panic" {
        return Ok(());
    }
    let defs = tool_definitions();
    let shown = clip(name, 100);
    let def = defs
        .iter()
        .find(|d| d["name"] == name)
        .ok_or_else(|| format!("unknown tool `{shown}`"))?;
    let schema = &def["inputSchema"];
    let Some(a) = args.as_object() else {
        return Err(format!("{name}: arguments must be an object"));
    };
    for r in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if a.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name}: missing required argument `{r}`"));
        }
    }
    let props = schema["properties"].as_object();
    for (k, v) in a {
        let Some(p) = props.and_then(|p| p.get(k)) else {
            return Err(format!("{name}: unknown argument `{}`", clip(k, 100)));
        };
        if v.is_null() {
            continue;
        }
        let ok = match &p["type"] {
            Value::String(t) => type_matches(t, v),
            Value::Array(ts) => ts
                .iter()
                .filter_map(Value::as_str)
                .any(|t| type_matches(t, v)),
            _ => true,
        };
        if !ok {
            return Err(format!(
                "{name}: argument `{k}` must be of type {}",
                p["type"]
            ));
        }
        if let Some(allowed) = p["enum"].as_array() {
            if !allowed.iter().any(|x| x == v) {
                return Err(format!(
                    "{name}: `{k}` must be one of {}",
                    Value::Array(allowed.clone())
                ));
            }
        }
    }
    if name == "batch" {
        if let Some(calls) = a.get("calls").and_then(Value::as_array) {
            if calls.len() > 100 {
                return Err("batch: `calls` has too many items (at most 100)".to_string());
            }
        }
    }
    Ok(())
}

/// A tool result: one text block and the error flag.
fn text_result(v: &Value, is_error: bool) -> Value {
    let text = match v {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

// ---------------------------------------------------------------- remote --

/// A running SeaCraft app reached through its loopback control channel
/// (`seacraft-gui --control HOST:PORT`).
///
/// The app speaks one JSON line per request, `{"method": …, "params": {…}}`,
/// and answers `{"ok": true, "result": …}` or `{"ok": false, "error": …}`
/// (`crates/seacraft-gui/src/control.rs`). SeaCraft's methods are not MCP
/// tools, so each tool is translated to the one that does the same thing:
/// `hull_info` -> `ui.inspect`, `load_hull` -> `model.open` / `model.load_demo`,
/// `save_hull` -> `model.save`, `move_point` -> `point.move`,
/// `set_crease` -> `edge.crease`, `split_face` -> `model.split_face`,
/// `insert_edge` -> `model.insert_edge`, `hydro_compute` -> `hydro.compute`,
/// `stability_cross_curves` -> `stability.cross_curves`,
/// `bonjean_curves` -> `curves.bonjean`, `table_of_offsets` -> `curves.offsets`,
/// `resistance_curve` -> `resistance.compute`, `planing_curve` -> `planing.compute`,
/// `transform_hull` -> `model.transform`, `unfold_plate` -> `curves.unfold`,
/// `hydro_table` -> `hydro.table`, `trim_scan` -> `hydro.trim`,
/// `optimize_hull` -> `model.optimize`,
/// `export_linesplan`
/// -> `model.export`, `render_png` -> `ui.render`. The app's reply is returned
/// as the tool's result (its own field names); a failure is a tool error.
pub struct Remote {
    addr: String,
    token: Option<String>,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
}

impl Remote {
    /// Connect to `addr` (`host:port`) with an optional authentication token,
    /// failing at once when nothing listens.
    pub fn connect_with_token(addr: &str, token: Option<String>) -> std::io::Result<Self> {
        let mut r = Self {
            addr: addr.to_string(),
            token,
            conn: None,
        };
        r.reconnect()?;
        Ok(r)
    }

    fn authenticate(&mut self, token: &str) -> std::io::Result<()> {
        let auth_cmd = json!({"method": "auth", "params": {"token": token}}).to_string();
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::other("not connected"));
        };
        writer.write_all(auth_cmd.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;

        let mut reply = String::new();
        let n = match reader.by_ref().take(MAX_REPLY).read_line(&mut reply) {
            Ok(n) => n,
            Err(e) => {
                self.conn = None;
                return Err(e);
            }
        };
        if n == 0 || !reply.ends_with('\n') {
            self.conn = None;
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed during authentication",
            ));
        }
        let v: Value = serde_json::from_str(&reply)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            self.conn = None;
            let err = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("authentication rejected");
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                err.to_string(),
            ))
        }
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("cannot resolve {}", self.addr),
        );
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // A measurement takes as long as it takes: the app answers
                    // when its job ends.
                    s.set_read_timeout(None).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    if let Some(token) = &self.token.clone() {
                        self.authenticate(token)?;
                    }
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        if self.conn.is_none() {
            self.reconnect()?;
        }
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::other("not connected"));
        };
        let sent = writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .and_then(|()| writer.flush());
        if let Err(e) = sent {
            self.conn = None;
            return Err(e);
        }
        let mut reply = String::new();
        let n = match reader.by_ref().take(MAX_REPLY).read_line(&mut reply) {
            Ok(n) => n,
            Err(e) => {
                self.conn = None;
                return Err(e);
            }
        };
        if n == 0 || !reply.ends_with('\n') {
            self.conn = None;
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed (or reply too large)",
            ));
        }
        Ok(reply)
    }

    /// One control-channel request, the app's `result`. A request is never
    /// retried: replaying an edit would apply it twice.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let line = json!({"method": method, "params": params}).to_string();
        let reply = self
            .roundtrip(&line)
            .map_err(|e| format!("SeaCraft at {}: {e}", self.addr))?;
        let v: Value = serde_json::from_str(&reply)
            .map_err(|e| format!("bad reply from SeaCraft at {}: {e}", self.addr))?;
        match v.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            Some(false) => Err(format!(
                "SeaCraft at {}: {}",
                self.addr,
                v.get("error").and_then(Value::as_str).unwrap_or("error")
            )),
            None => Err(format!("bad reply from SeaCraft at {}: no `ok`", self.addr)),
        }
    }

    /// Run one MCP tool on the app.
    pub fn tool(&mut self, name: &str, a: &Map<String, Value>) -> Result<Value, String> {
        match name {
            "hull_info" => self.request("ui.inspect", json!({})),
            "load_hull" => match str_arg(a, "path").filter(|p| !p.trim().is_empty()) {
                Some(path) => self.request("model.open", json!({"path": path})),
                None if bool_arg(a, "demo").unwrap_or(false) => self.request(
                    "model.load_demo",
                    json!({"symmetric": bool_arg(a, "symmetric").unwrap_or(false)}),
                ),
                None => Err("load_hull: give `path` or `demo: true`".to_string()),
            },
            "generate_hull" => self.request("model.generate", Value::Object(a.clone())),
            "save_hull" => match str_arg(a, "path").filter(|p| !p.trim().is_empty()) {
                Some(path) => self.request("model.save", json!({"path": path})),
                None => Err("save_hull: give `path` (the app saves to the named file)".to_string()),
            },
            "move_point" => {
                let index =
                    usize_arg(a, "index").ok_or("move_point: `index` must be a point index")?;
                let xyz = a
                    .get("xyz")
                    .and_then(Value::as_array)
                    .filter(|xyz| xyz.len() == 3)
                    .ok_or("move_point: `xyz` must be [x, y, z]")?;
                let mut p = Vec::new();
                for v in xyz {
                    p.push(
                        v.as_f64()
                            .filter(|n| n.is_finite() && n.abs() <= 1.0e6)
                            .ok_or("move_point: `xyz` must be three finite numbers")?,
                    );
                }
                self.request("point.move", json!({"index": index, "xyz": p}))
            }
            "split_face" => self.request("model.split_face", Value::Object(a.clone())),
            "insert_edge" => self.request("model.insert_edge", Value::Object(a.clone())),
            "set_crease" => {
                let a_idx = usize_arg(a, "a").ok_or("set_crease: `a` must be a point index")?;
                let b_idx = usize_arg(a, "b").ok_or("set_crease: `b` must be a point index")?;
                self.request(
                    "edge.crease",
                    json!({"edge": [a_idx, b_idx], "crease": bool_arg(a, "crease").unwrap_or(true)}),
                )
            }
            "hydro_compute" => {
                let r = self.request("hydro.compute", Value::Object(a.clone()))?;
                Ok(flatten_hydro(&r))
            }
            "stability_cross_curves" => {
                let r = self.request("stability.cross_curves", Value::Object(a.clone()))?;
                // The app solves its own heel/displacement grid and returns one
                // criteria row; present both the way the headless tool does.
                let imo = match r.get("imo") {
                    Some(Value::Object(_)) => json!([r.get("imo").cloned()]),
                    Some(Value::Array(list)) => json!(list),
                    _ => json!([]),
                };
                Ok(json!({
                    "condition": {
                        "kg": r.get("kg").cloned().unwrap_or(Value::Null),
                        "free_to_trim": r.get("free_trim").cloned().unwrap_or(Value::Null),
                        "lcg_x": r.get("lcg").cloned().unwrap_or(Value::Null),
                    },
                    "curves": r.get("curves").cloned().unwrap_or(Value::Null),
                    "imo_criteria": imo,
                    "backend": "app",
                }))
            }
            "loading_condition" if a.contains_key("case") || a.contains_key("case_path") => {
                self.request("stability.loading_condition", Value::Object(a.clone()))
            }
            "loading_condition" => {
                let mut p = json!({});
                if let Some(name) = str_arg(a, "name") {
                    p["name"] = json!(name);
                }
                self.request("loading.compute", p)
            }
            "project_get" => self.request("project.get", json!({})),
            "project_set" => {
                let p = a.get("project").ok_or("project_set: give `project`")?;
                self.request("project.set", json!({ "project": p }))
            }
            "weather_criterion" => self.request("stability.weather", Value::Object(a.clone())),
            "damage_stability" if a.contains_key("case") || a.contains_key("case_path") => {
                self.request("stability.damage_stability", Value::Object(a.clone()))
            }
            "damage_stability" => {
                let mut p = json!({});
                if let Some(name) = str_arg(a, "name") {
                    p["name"] = json!(name);
                }
                self.request("damage.compute", p)
            }
            "seakeeping_raos" => self.request("seakeeping.compute", Value::Object(a.clone())),
            "seakeeping_irregular" => {
                self.request("seakeeping.irregular", Value::Object(a.clone()))
            }
            "bonjean_curves" => self.request("curves.bonjean", Value::Object(a.clone())),
            "table_of_offsets" => self.request("curves.offsets", Value::Object(a.clone())),
            "resistance_curve" => self.request("resistance.compute", Value::Object(a.clone())),
            "planing_curve" => self.request("planing.compute", Value::Object(a.clone())),
            "transform_hull" => self.request("model.transform", Value::Object(a.clone())),
            "unfold_plate" => self.request("curves.unfold", Value::Object(a.clone())),
            "hydro_table" => self.request("hydro.table", Value::Object(a.clone())),
            "trim_scan" => self.request("hydro.trim", Value::Object(a.clone())),
            "optimize_hull" => self.request("model.optimize", Value::Object(a.clone())),
            "export_linesplan" => {
                let path = str_arg(a, "path")
                    .filter(|p| !p.trim().is_empty())
                    .ok_or("export_linesplan: give `path` (.svg, .dxf or .igs)")?;
                let format = match Path::new(path)
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default()
                    .as_str()
                {
                    "svg" | "" => "svg",
                    "dxf" => "dxf",
                    "igs" | "iges" => "igs",
                    other => {
                        return Err(format!(
                            "export_linesplan: unsupported extension `{}` (use .svg, .dxf or .igs)",
                            clip(other, 20)
                        ))
                    }
                };
                self.request("model.export", json!({"path": path, "format": format}))
            }
            "render_png" => {
                let path = str_arg(a, "path")
                    .filter(|p| !p.trim().is_empty())
                    .ok_or("render_png: give `path`")?;
                let view = match str_arg(a, "view")
                    .unwrap_or("perspective")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "perspective" => 0,
                    "profile" => 1,
                    "plan" => 2,
                    "body" => 3,
                    other => {
                        return Err(format!(
                            "render_png: unknown `view` `{}` (perspective, profile, plan, body)",
                            clip(other, 40)
                        ))
                    }
                };
                let mut p = json!({
                    "path": path,
                    "width": side_arg(a, "width", DEFAULT_SIZE.0),
                    "height": side_arg(a, "height", DEFAULT_SIZE.1),
                    "view": view,
                });
                if let Some(mode) = str_arg(a, "mode") {
                    p["mode"] = json!(mode.to_ascii_lowercase());
                }
                self.request("ui.render", p)
            }
            "design_report" => {
                let path = str_arg(a, "path")
                    .filter(|p| !p.trim().is_empty())
                    .ok_or("design_report: give `path`")?;
                let mut p = json!({ "path": path });
                if let Some(d) = number_arg(a, "draft") {
                    p["draft"] = json!(d);
                }
                if let Some(kg) = number_arg(a, "kg") {
                    p["kg"] = json!(kg);
                }
                if let Some(lcg) = number_arg(a, "lcg") {
                    p["lcg"] = json!(lcg);
                }
                if let Some(v) = a.get("speeds_kn").or_else(|| a.get("speeds")) {
                    p["speeds_kn"] = v.clone();
                }
                if let Some(nr) = bool_arg(a, "include_resistance") {
                    p["include_resistance"] = json!(nr);
                }
                if let Some(pl) = bool_arg(a, "include_planing") {
                    p["include_planing"] = json!(pl);
                }
                if a.get("level").is_some() {
                    p["level"] = json!(level_arg(a)?);
                }
                self.request("report.design", p)
            }
            other => Err(format!("unknown tool `{}`", clip(other, 100))),
        }
    }
}

/// The app's `hydro.compute` reply (`{draft, trim, heel_deg, kg, free_trim,
/// lcg, result}`) as the flat Hydrostatics object the tool promises.
fn flatten_hydro(reply: &Value) -> Value {
    let mut v = reply
        .get("result")
        .filter(|r| r.is_object())
        .cloned()
        .unwrap_or_else(|| reply.clone());
    let kg = reply.get("kg").and_then(Value::as_f64);
    if let Some(o) = v.as_object_mut() {
        if let Some(kg) = kg {
            if let Some(km) = o.get("km_transverse").and_then(Value::as_f64) {
                o.insert("gm_m".into(), json!(km - kg));
            }
            o.insert("kg".into(), json!(kg));
        }
        o.insert(
            "free_to_trim".into(),
            reply.get("free_trim").cloned().unwrap_or(json!(false)),
        );
        o.insert("backend".into(), json!("app"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `tool_name {…}` call in the agent skills names a real tool, and each skill's
    /// frontmatter is well formed.
    #[test]
    fn skills_name_only_real_tools() {
        let docs = [
            (
                "seacraft-hull-design",
                include_str!("../../../skills/seacraft-hull-design/SKILL.md"),
            ),
            (
                "seacraft-fairing",
                include_str!("../../../skills/seacraft-fairing/SKILL.md"),
            ),
            (
                "seacraft-stability-check",
                include_str!("../../../skills/seacraft-stability-check/SKILL.md"),
            ),
        ];
        let tools: Vec<String> = tool_definitions()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let mut checked = 0;
        for (folder, doc) in docs {
            let front = doc
                .strip_prefix("---\n")
                .and_then(|r| r.split_once("\n---\n"))
                .unwrap_or_else(|| panic!("{folder}: no frontmatter"))
                .0;
            let keys: Vec<&str> = front
                .lines()
                .filter_map(|l| l.split_once(':').map(|k| k.0))
                .collect();
            assert_eq!(keys, ["name", "description"], "{folder}: frontmatter keys");
            let field = |k: &str| {
                front
                    .lines()
                    .find_map(|l| l.strip_prefix(k))
                    .unwrap()
                    .trim()
            };
            assert_eq!(
                field("name:"),
                folder,
                "{folder}: name must equal the folder"
            );
            assert!(
                field("description:").starts_with("Use when "),
                "{folder}: description"
            );
            for span in doc.split('`').skip(1).step_by(2) {
                let name: String = span
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_lowercase()) {
                    continue;
                }
                if span[name.len()..].starts_with(" {") {
                    assert!(
                        tools.contains(&name),
                        "{folder} calls `{name}`, which is not a tool"
                    );
                    checked += 1;
                }
            }
        }
        assert!(
            checked >= 30,
            "only {checked} tool calls found in the skills"
        );
    }

    fn send(server: &mut McpServer, msg: Value) -> Value {
        let line = msg.to_string();
        match server.handle_line(&line) {
            Some(reply) => serde_json::from_str(&reply)
                .unwrap_or_else(|e| panic!("reply to {line} is not JSON ({e}): {reply}")),
            None => panic!("no reply to {line}"),
        }
    }

    fn request(server: &mut McpServer, id: u64, method: &str, params: Value) -> Value {
        send(
            server,
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
    }

    /// A tool call that must succeed; returns the parsed text block.
    fn ok(server: &mut McpServer, id: u64, name: &str, args: Value) -> Value {
        let r = request(
            server,
            id,
            "tools/call",
            json!({"name": name, "arguments": args}),
        );
        assert!(r.get("error").is_none(), "{name}: {r}");
        let result = &r["result"];
        assert_ne!(result["isError"], json!(true), "{name}: {result}");
        let text = result["content"][0]["text"].as_str().expect("a text block");
        serde_json::from_str(text)
            .unwrap_or_else(|e| panic!("{name}: reply is not JSON ({e}): {text}"))
    }

    /// A tool call that must fail as a tool error (not a protocol error).
    fn fails(server: &mut McpServer, id: u64, name: &str, args: Value) -> String {
        let r = request(
            server,
            id,
            "tools/call",
            json!({"name": name, "arguments": args}),
        );
        assert!(
            r.get("error").is_none(),
            "{name} should be a tool error, got {r}"
        );
        let result = &r["result"];
        assert_eq!(
            result["isError"],
            json!(true),
            "{name} should fail: {result}"
        );
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn temp_path(extension: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "seacraft-mcp-test-{}-{n}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn mcp_initialize_list_and_call_tools_headless() {
        let mut s = McpServer::new();
        assert!(!s.is_remote(), "no --connect: headless");

        // initialize: the protocol revision, the server's identity and tools.
        let init = request(
            &mut s,
            1,
            "initialize",
            json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": {}}),
        );
        assert_eq!(init["jsonrpc"], "2.0");
        assert_eq!(init["id"], 1);
        assert_eq!(init["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(init["result"]["serverInfo"]["name"], "seacraft-mcp");
        assert_eq!(
            init["result"]["serverInfo"]["version"],
            seacraft_core::VERSION
        );
        assert_eq!(
            init["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        // An unknown revision is answered with ours instead of failing.
        let v = request(
            &mut s,
            2,
            "initialize",
            json!({"protocolVersion": "1999-01-01"}),
        );
        assert_eq!(v["result"]["protocolVersion"], PROTOCOL_VERSION);

        // A notification is acted on and never answered.
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());

        // ping: an empty result.
        let pong = request(&mut s, 3, "ping", json!({}));
        assert_eq!(pong["result"], json!({}));

        // tools/list: every hull tool, each with a JSON Schema.
        let list = request(&mut s, 4, "tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().expect("a tool array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            vec![
                "hull_info",
                "load_hull",
                "generate_hull",
                "save_hull",
                "project_get",
                "project_set",
                "move_point",
                "set_crease",
                "split_face",
                "insert_edge",
                "hydro_compute",
                "stability_cross_curves",
                "loading_condition",
                "weather_criterion",
                "damage_stability",
                "seakeeping_raos",
                "seakeeping_irregular",
                "bonjean_curves",
                "table_of_offsets",
                "resistance_curve",
                "planing_curve",
                "transform_hull",
                "unfold_plate",
                "hydro_table",
                "trim_scan",
                "optimize_hull",
                "export_linesplan",
                "render_png",
                "design_report",
                "batch",
            ]
        );
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{t}");
            assert_eq!(t["inputSchema"]["additionalProperties"], false, "{t}");
        }

        // hull_info: the demo hull, measured on the control net's extents.
        let info = ok(&mut s, 5, "hull_info", json!({}));
        assert_eq!(info["name"], "SeaCraft 12");
        assert_eq!(info["backend"], "headless");
        assert!(info["points"].as_u64().unwrap_or(0) > 20, "{info}");
        assert!(info["faces"].as_u64().unwrap_or(0) > 20, "{info}");
        let length = info["extents"]["length"].as_f64().expect("a length");
        assert!((length - 12.0).abs() < 1e-9, "{info}");
        assert_eq!(info["validation"]["ok"], true, "{info}");
        assert!(info["layers"].as_array().is_some_and(|l| !l.is_empty()));

        // hydro_compute, free to trim: the AGENTS.md reference volume.
        let hydro = ok(
            &mut s,
            6,
            "hydro_compute",
            json!({"draft": 0.62, "kg": 2.2, "free_trim": true}),
        );
        let volume = hydro["volume_m3"].as_f64().expect("a volume");
        assert!(
            (volume - 11.851).abs() < 0.12,
            "volume {volume} left the reference 11.851 m3: {hydro}"
        );
        assert_eq!(hydro["free_to_trim"], true);
        assert!(
            (hydro["draft"].as_f64().unwrap_or(0.0) - 0.62).abs() < 0.05,
            "{hydro}"
        );
        assert!(
            hydro["trim_m"].as_f64().is_some_and(|t| t.is_finite()),
            "{hydro}"
        );
        let gm = hydro["gm_m"].as_f64().expect("a GM");
        let km = hydro["km_transverse"].as_f64().expect("a KM");
        assert!((gm - (km - 2.2)).abs() < 1e-9, "GM is KM - KG: {hydro}");
        // The fixed-trim path gives the same displacement at trim 0.
        let fixed = ok(&mut s, 7, "hydro_compute", json!({"draft": 0.62}));
        assert_eq!(fixed["free_to_trim"], false);
        let fixed_draft = fixed["draft"].as_f64().unwrap_or(0.0);
        assert!((fixed_draft - 0.62).abs() < 1e-12, "{fixed}");

        // bonjean_curves: the section area at the design draft has to agree
        // with the hydrostatic mainframe area of the same mesh, and the family
        // must grow with draft.
        let bj = ok(
            &mut s,
            60,
            "bonjean_curves",
            json!({"stations": 21, "drafts": [0.31, 0.62, 1.24]}),
        );
        assert_eq!(bj["backend"], "headless", "{bj}");
        assert_eq!(bj["drafts_m"], json!([0.31, 0.62, 1.24]));
        let sections = bj["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), 21, "{bj}");
        let mid = &sections[10];
        assert!(
            (mid["x"].as_f64().unwrap_or(0.0) - 6.0).abs() < 0.6,
            "{mid}"
        );
        let a: Vec<f64> = mid["areas_m2"]
            .as_array()
            .expect("areas")
            .iter()
            .filter_map(Value::as_f64)
            .collect();
        assert_eq!(a.len(), 3);
        assert!(a[0] < a[1] && a[1] < a[2], "area grows with draft: {a:?}");
        // The largest area at the design draft is the midship section, within
        // the demo hull's 1.467 m2 mainframe area (the peak sits a little
        // forward of the LCF, at ~1.49 m2).
        let at_design = sections
            .iter()
            .filter_map(|s| s["areas_m2"][1].as_f64())
            .fold(0.0_f64, f64::max);
        assert!(
            (at_design - 1.467).abs() < 0.06,
            "largest area at T = 0.62 was {at_design} (expected ~1.467 m2)"
        );
        assert!(
            bj["area_max_m2"].as_f64().unwrap_or(0.0) > a[2],
            "the deepest station is not the largest: {bj}"
        );
        // table_of_offsets: half-breadths and buttocks, spot-checked against
        // the hydrostatic beam and the hull's own depth.
        let ofs = ok(
            &mut s,
            62,
            "table_of_offsets",
            json!({"stations": 11, "waterlines": [0.62], "buttocks": [1.0]}),
        );
        assert_eq!(ofs["backend"], "headless", "{ofs}");
        assert_eq!(ofs["waterlines_m"], json!([0.62]));
        let rows = ofs["stations"].as_array().expect("stations");
        assert_eq!(rows.len(), 11, "{ofs}");
        let widest = rows
            .iter()
            .filter_map(|r| r["waterline_half_breadths"][0].as_f64())
            .fold(0.0_f64, f64::max);
        assert!(
            (widest - 1.771).abs() < 0.03,
            "half-breadth at T = 0.62 was {widest} (BWL/2 is 1.771)"
        );
        // A cell the section cannot reach is null, never zero.
        let bow = rows.last().expect("a bow station");
        assert!(
            rows.iter()
                .any(|r| r["waterline_half_breadths"][0].is_null()),
            "{ofs}"
        );
        assert!(
            bow["buttock_heights"][0].is_null()
                || bow["buttock_heights"][0].as_f64().unwrap_or(0.0) > 0.0
        );
        // Defaults cover the depth and the half beam.
        let dflt = ok(
            &mut s,
            63,
            "table_of_offsets",
            json!({"waterline_steps": 3, "buttock_steps": 2}),
        );
        let wls = dflt["waterlines_m"].as_array().expect("waterlines");
        assert_eq!(wls.len(), 3, "{dflt}");
        let top = wls[2].as_f64().unwrap_or(0.0);
        let depth = dflt["stations"]
            .as_array()
            .expect("stations")
            .iter()
            .filter_map(|r| r["deck_z"].as_f64())
            .fold(0.0_f64, f64::max)
            - dflt["keel_z"].as_f64().unwrap_or(0.0);
        // The tallest sampled station is a little lower than the model's own
        // maximum sheer (the stem is not a midpoint), so the top default
        // waterline sits at or just above the sampled depth.
        assert!(
            top >= depth && top < depth + 0.1,
            "top waterline {top} vs sampled depth {depth}"
        );

        // The default draft range covers the hull depth and stays in order.
        let dflt = ok(&mut s, 61, "bonjean_curves", json!({"draft_steps": 4}));
        let drafts: Vec<f64> = dflt["drafts_m"]
            .as_array()
            .expect("drafts")
            .iter()
            .filter_map(Value::as_f64)
            .collect();
        assert_eq!(drafts.len(), 4, "{dflt}");
        assert!(drafts.windows(2).all(|w| w[1] > w[0]), "{drafts:?}");
        assert!(
            (drafts[3] - dflt["depth_m"].as_f64().unwrap_or(0.0)).abs() < 1e-9,
            "{dflt}"
        );

        // move_point: an asymmetric layer keeps the y it is given ...
        let moved = ok(
            &mut s,
            8,
            "move_point",
            json!({"index": 0, "xyz": [0.0, -1.5, 0.2]}),
        );
        assert_eq!(moved["clamped"], false, "{moved}");
        assert_eq!(moved["after"]["y"], -1.5);
        assert_eq!(moved["before"]["y"], 0.0, "{moved}");
        let again = ok(&mut s, 9, "hull_info", json!({}));
        assert_eq!(again["points"], info["points"], "moving a point adds none");
        // ... a symmetric one clamps y to the centreline.
        let mut half = McpServer::from_surface(demo::demo_hull(true));
        let clamped = ok(
            &mut half,
            10,
            "move_point",
            json!({"index": 0, "xyz": [0.1, -2.0, 0.3]}),
        );
        assert_eq!(clamped["clamped"], true, "{clamped}");
        assert_eq!(clamped["after"]["y"], 0.0, "{clamped}");
        assert_eq!(clamped["after"]["x"], 0.1);
        // An index off the end is a tool error, not a panic.
        fails(
            &mut s,
            11,
            "move_point",
            json!({"index": 100_000, "xyz": [0.0, 0.0, 0.0]}),
        );

        // set_crease: the first edge the model has.
        let edge = s.surface.edges.first().map(|e| (e.a, e.b)).expect("edges");
        let creased = ok(
            &mut s,
            12,
            "set_crease",
            json!({"a": edge.0, "b": edge.1, "crease": true}),
        );
        assert_eq!(creased["crease"], true);
        assert_eq!(creased["a"], edge.0);
        fails(
            &mut s,
            13,
            "set_crease",
            json!({"a": 0, "b": 100_000, "crease": true}),
        );

        // split_face / insert_edge: the topology grows, the model stays valid,
        // and the hydrostatics barely move (a planar fan does not move them at
        // all; this net is curved, so allow a fraction of a percent).
        let before = ok(&mut s, 70, "hull_info", json!({}));
        let faces_before = before["faces"].as_u64().unwrap_or(0);
        let vol_before = ok(&mut s, 71, "hydro_compute", json!({"draft": 0.62}))["volume_m3"]
            .as_f64()
            .unwrap_or(0.0);
        let split = ok(&mut s, 72, "split_face", json!({"face": 0}));
        assert_eq!(
            split["pieces"], 4,
            "a quad fans into four triangles: {split}"
        );
        assert_eq!(split["faces"].as_u64(), Some(faces_before + 3), "{split}");
        let after = ok(&mut s, 73, "hull_info", json!({}));
        assert_eq!(after["validation"]["ok"], true, "{after}");
        let vol_after = ok(&mut s, 74, "hydro_compute", json!({"draft": 0.62}))["volume_m3"]
            .as_f64()
            .unwrap_or(0.0);
        assert!(
            (vol_after - vol_before).abs() / vol_before < 5e-3,
            "splitting a face moved the displacement from {vol_before} to {vol_after}"
        );
        // The fan's new point is addressable, and the fan edges are real edges.
        let new_point = split["point"].as_u64().unwrap_or(u64::MAX) as usize;
        let crease = ok(
            &mut s,
            75,
            "set_crease",
            json!({"a": new_point, "b": 0, "crease": true}),
        );
        assert_eq!(crease["crease"], true, "{crease}");
        // An edge cannot be inserted between neighbours that are already joined.
        fails(
            &mut s,
            76,
            "insert_edge",
            json!({"face": 0, "a": new_point, "b": 0}),
        );
        // The edited net is what the file round-trip below has to reproduce.
        let edited = ok(&mut s, 77, "hull_info", json!({}));

        // stability_cross_curves: KN and GZ, with the IMO criteria.
        let curves = ok(
            &mut s,
            14,
            "stability_cross_curves",
            json!({"kg": 2.2, "heels": [0.0, 30.0], "displacements": [6.0, 12.0]}),
        );
        let list = curves["curves"].as_array().expect("curves");
        assert_eq!(list.len(), 2, "one curve per heel: {curves}");
        for c in list {
            assert_eq!(c["points"].as_array().map(Vec::len), Some(2), "{c}");
        }
        let crit = curves["imo_criteria"].as_array().expect("criteria");
        assert_eq!(crit.len(), 2, "one criteria row per displacement: {curves}");
        assert!(crit.iter().all(|c| c["passed"].is_boolean()), "{curves}");

        // weather_criterion: IMO 2008 IS Code Part A §2.3 severe wind and rolling.
        let wc = ok(
            &mut s,
            150,
            "weather_criterion",
            json!({
                "draft": 0.62,
                "kg": 1.0,
                "free_trim": true,
                "bilge": "round",
                "windage_extra": [{"name": "Deckhouse", "area_m2": 4.0, "z_centroid": 1.8}],
                "level": 2
            }),
        );
        assert_eq!(wc["backend"], "headless", "{wc}");
        assert!(wc["area_m2"].as_f64().unwrap_or(0.0) > 4.0, "{wc}");
        assert!(wc["lw1_m"].as_f64().unwrap_or(0.0) > 0.0, "{wc}");
        assert!(wc["phi1_deg"].as_f64().unwrap_or(0.0) > 0.0, "{wc}");
        assert!(wc["passed"].is_boolean(), "{wc}");
        fails(&mut s, 151, "weather_criterion", json!({"bilge": "bogus"}));

        // render_png: a real PNG on disk, with its metadata.
        let png = temp_path("png");
        let rendered = ok(
            &mut s,
            15,
            "render_png",
            json!({"path": png, "mode": "zebra", "view": "profile", "width": 320, "height": 200, "level": 2}),
        );
        assert_eq!(rendered["mode"], "zebra");
        assert_eq!(rendered["view"], "profile");
        assert_eq!(rendered["width"], 320);
        assert!(rendered["bytes"].as_u64().unwrap_or(0) > 1000, "{rendered}");
        let bytes = std::fs::read(&png).expect("the PNG was written");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a real PNG");
        let _ = std::fs::remove_file(&png);

        // export_linesplan: the extension picks the format.
        let svg = temp_path("svg");
        let plan = ok(
            &mut s,
            16,
            "export_linesplan",
            json!({"path": svg, "stations": 5, "buttocks": 3, "waterlines": 3, "combs": true}),
        );
        assert_eq!(plan["format"], "svg");
        assert!(plan["bytes"].as_u64().unwrap_or(0) > 1000, "{plan}");
        let text = std::fs::read_to_string(&svg).expect("the SVG was written");
        assert!(text.starts_with("<?xml") || text.contains("<svg"), "an SVG");
        let _ = std::fs::remove_file(&svg);

        // mainframe_x: the midship section can be measured where the caller
        // wants it, and the default is the LCF (the `.fef` carries no mainframe).
        let default_hydro = ok(&mut s, 90, "hydro_compute", json!({"draft": 0.62}));
        let lcf = default_hydro["mainframe_x"].as_f64().unwrap_or(0.0);
        assert!(lcf > 3.0 && lcf < 6.0, "{default_hydro}");
        let at_lcf = default_hydro["mainframe_area_m2"].as_f64().unwrap_or(0.0);
        let moved = ok(
            &mut s,
            91,
            "hydro_compute",
            json!({"draft": 0.62, "mainframe_x": 5.2}),
        );
        assert!(
            (moved["mainframe_x"].as_f64().unwrap_or(0.0) - 5.2).abs() < 1e-9,
            "{moved}"
        );
        let at_52 = moved["mainframe_area_m2"].as_f64().unwrap_or(0.0);
        assert!(
            (at_lcf - 1.467).abs() < 0.01 && (at_52 - 1.441).abs() < 0.02,
            "mainframe areas {at_lcf} at the LCF and {at_52} at 5.2 m"
        );
        assert!(
            (moved["midship_coefficient"].as_f64().unwrap_or(0.0) - 0.6561).abs() < 0.005,
            "Cm at 5.2 m: {moved}"
        );

        // hydro_table: the top row must be the single-point measurement, and an
        // explicit draft list must be honoured exactly.
        let tbl = ok(&mut s, 92, "hydro_table", json!({"steps": 5}));
        assert_eq!(tbl["drafts"], json!(5), "{tbl}");
        let rows = tbl["rows"].as_array().expect("rows");
        let top = rows.last().expect("a top row");
        assert!(
            (top["draft"].as_f64().unwrap_or(0.0) - 0.62).abs() < 1e-9,
            "{top}"
        );
        let single = ok(&mut s, 93, "hydro_compute", json!({"draft": 0.62}));
        assert!(
            (top["volume_m3"].as_f64().unwrap_or(0.0)
                - single["volume_m3"].as_f64().unwrap_or(0.0))
            .abs()
                < 1e-9,
            "table row {top} vs single point {single}"
        );
        // Physical magnitudes: a 12 m hull is ~0.3 t/cm, not 300.
        let tpc = top["tpc"].as_f64().unwrap_or(0.0);
        assert!((0.05..1.5).contains(&tpc), "TPC {tpc} t/cm");
        let two = ok(&mut s, 94, "hydro_table", json!({"drafts": [0.3, 0.62]}));
        assert_eq!(two["drafts"], json!(2), "{two}");
        fails(&mut s, 95, "hydro_table", json!({"drafts": [0.0]}));
        fails(&mut s, 96, "hydro_table", json!({"steps": 0}));

        // trim_scan: the even-keel point is the resistance at that speed, the
        // displacement holds, and a range with no interior minimum says so.
        let t = ok(
            &mut s,
            97,
            "trim_scan",
            json!({"speed_kn": 10.0, "steps": 5}),
        );
        assert_eq!(t["backend"], "headless", "{t}");
        let pts = t["points"].as_array().expect("points");
        assert_eq!(pts.len(), 5, "{t}");
        let zero = pts
            .iter()
            .find(|p| p["trim_m"].as_f64().unwrap_or(1.0).abs() < 1e-12)
            .expect("an even-keel point");
        let target = t["target_volume_m3"].as_f64().unwrap_or(0.0);
        for p in pts {
            let v = p["volume_m3"].as_f64().unwrap_or(0.0);
            assert!((v - target).abs() / target < 1e-4, "volume {v} vs {target}");
        }
        // Same measurement path as `resistance_curve` at the same speed. (The
        // model has been transformed by the earlier test, so the absolute
        // numbers are not the demo's; the *agreement* is what is asserted.)
        let rc = ok(&mut s, 98, "resistance_curve", json!({"speeds": [10.0]}));
        let row = &rc["speeds"][0];
        // The scan solves the draft, so it lands a fraction of a micrometre away
        // from the command's 0.62 m and the resistances differ in the 6th digit;
        // a relative tolerance is the honest comparison.
        let a_rt = zero["rt_n"].as_f64().unwrap_or(0.0);
        let b_rt = row["rt_n"].as_f64().unwrap_or(0.0);
        assert!(
            (a_rt - b_rt).abs() / b_rt < 1e-4,
            "trim scan {a_rt} vs resistance {b_rt}"
        );
        assert!(t["best"].is_object(), "{t}");
        // Bad speed, empty range and a one-point scan are all refused.
        fails(&mut s, 99, "trim_scan", json!({"speed_kn": 0.0}));
        fails(
            &mut s,
            100,
            "trim_scan",
            json!({"speed_kn": 10.0, "from": 0.4, "to": 0.2}),
        );

        // resistance_curve: PE = RT·V, the branch is reported, and the power
        // chain closes when a propeller is given.
        let curve = ok(
            &mut s,
            78,
            "resistance_curve",
            json!({"draft": 0.62, "speeds": [8.0, 12.0], "propeller_diameter": 0.45}),
        );
        let points = curve["speeds"].as_array().expect("speeds");
        assert_eq!(points.len(), 2, "{curve}");
        for p in points {
            let rt = p["rt_n"].as_f64().unwrap_or(0.0);
            let pe = p["pe_w"].as_f64().unwrap_or(0.0);
            let v = p["speed_m_s"].as_f64().unwrap_or(0.0);
            assert!((pe - rt * v).abs() < 1e-6, "{p}");
            assert!(rt > 0.0 && p["wave_branch"].is_string(), "{p}");
        }
        assert_eq!(
            curve["propulsion"].as_array().map(Vec::len),
            Some(2),
            "{curve}"
        );
        assert!(
            curve["propulsion"][0]["wake_fraction"].as_f64().is_some(),
            "{curve}"
        );

        // resistance_curve with Michell thin-ship method and catamaran spacing
        let michell_curve = ok(
            &mut s,
            79,
            "resistance_curve",
            json!({"draft": 0.62, "speeds": [8.0, 12.0], "method": "michell", "cat_spacing": 10.0}),
        );
        assert_eq!(michell_curve["method"], "michell");
        assert_eq!(michell_curve["cat_spacing"], 10.0);
        let m_points = michell_curve["speeds"].as_array().expect("speeds");
        assert_eq!(m_points.len(), 2, "{michell_curve}");
        for p in m_points {
            let rt = p["rt_n"].as_f64().unwrap_or(0.0);
            let pe = p["pe_w"].as_f64().unwrap_or(0.0);
            let v = p["speed_m_s"].as_f64().unwrap_or(0.0);
            assert!((pe - rt * v).abs() < 1e-6, "{p}");
            assert!(rt > 0.0 && p["wave_branch"] == "michell", "{p}");
        }

        // resistance_curve with DSYHS method
        let dsyhs_curve = ok(
            &mut s,
            79,
            "resistance_curve",
            json!({"draft": 0.62, "speeds": [6.0, 8.0], "method": "dsyhs"}),
        );
        assert_eq!(dsyhs_curve["method"], "dsyhs");
        let d_points = dsyhs_curve["speeds"].as_array().expect("speeds");
        assert_eq!(d_points.len(), 2, "{dsyhs_curve}");
        for p in d_points {
            let rt = p["rt_n"].as_f64().unwrap_or(0.0);
            let pe = p["pe_w"].as_f64().unwrap_or(0.0);
            let v = p["speed_m_s"].as_f64().unwrap_or(0.0);
            assert!((pe - rt * v).abs() < 1e-6, "{p}");
            assert!(rt > 0.0 && p["wave_branch"] == "dsyhs", "{p}");
        }
        // planing_curve: the trim equilibrium is pinned by the pressure centre.
        let plan = ok(
            &mut s,
            79,
            "planing_curve",
            json!({"speeds": [20.0, 25.0], "deadrise": 15.0}),
        );
        let rows = plan["speeds"].as_array().expect("speeds");
        assert_eq!(rows.len(), 2, "{plan}");
        for r in rows {
            let lcp = r["lcp_over_b"].as_f64().unwrap_or(0.0);
            let lcg = plan["setup"]["hull"]["lcg_from_transom_m"]
                .as_f64()
                .unwrap_or(0.0)
                / plan["setup"]["hull"]["beam_m"].as_f64().unwrap_or(1.0);
            assert!(
                (lcp - lcg).abs() < 1e-3,
                "LCP/b {lcp} against LCG/b {lcg}: {r}"
            );
            assert!(r["trim_deg"].as_f64().unwrap_or(0.0) > 1.0, "{r}");
            assert!(r["resistance_n"].as_f64().unwrap_or(0.0) > 0.0);
        }

        // transform_hull: the model really is rescaled, and the measurement
        // draft moves with a draft target.
        let before_t = ok(&mut s, 80, "hull_info", json!({}));
        let faces_before_t = before_t["faces"].as_u64().unwrap_or(0);
        let t = ok(&mut s, 81, "transform_hull", json!({ "lwl": 18.0 }));
        assert_eq!(t["backend"], "headless", "{t}");
        assert!(
            (t["report"]["after"]["lwl"].as_f64().unwrap_or(0.0) - 18.0).abs() < 0.02,
            "{t}"
        );
        assert_eq!(
            t["faces"].as_u64(),
            Some(faces_before_t),
            "a transform keeps the topology"
        );
        // A draft target scales z (and with it the measurement draft).
        let d = ok(&mut s, 82, "transform_hull", json!({ "draft": 0.9 }));
        assert!(
            (d["report"]["after"]["draft"].as_f64().unwrap_or(0.0) - 0.9).abs() < 1e-3,
            "{d}"
        );
        assert!(
            (d["draft"].as_f64().unwrap_or(0.0) - 0.9).abs() < 1e-3,
            "{d}"
        );
        // A mirror keeps the model valid; bad input is refused.
        let m = ok(&mut s, 83, "transform_hull", json!({ "mirror": "x" }));
        assert_eq!(m["report"]["mirror"], "x", "{m}");
        fails(&mut s, 85, "transform_hull", json!({}));
        fails(&mut s, 86, "transform_hull", json!({ "scale": [1.0, 1.0] }));

        // unfold_plate: a strake develops with its areas preserved, and the SVG
        // is written where asked.
        let svg = temp_path("plate.svg");
        let plate = ok(
            &mut s,
            87,
            "unfold_plate",
            json!({"axis": "z", "from": 0.2, "to": 0.45, "x_from": 3.0, "x_to": 6.0, "path": svg}),
        );
        let p = &plate["plate"];
        let a2 = p["area_2d_m2"].as_f64().unwrap_or(0.0);
        let a3 = p["area_3d_m2"].as_f64().unwrap_or(0.0);
        assert!(a2 > 0.0, "{plate}");
        assert!((a2 - a3).abs() / a3 < 1e-9, "areas {a2} vs {a3}");
        assert!(p["developable"].as_bool().unwrap_or(false), "{plate}");
        assert!(p["max_gap_m"].as_f64().unwrap_or(1.0) < 0.01, "{plate}");
        assert_eq!(plate["written"], json!(svg.to_string_lossy()));
        let text = std::fs::read_to_string(&svg).expect("the SVG was written");
        assert!(
            text.contains("<svg") && text.contains("developable"),
            "an SVG"
        );
        let _ = std::fs::remove_file(&svg);
        // A band that hits nothing is a tool error, not an empty plate.
        fails(
            &mut s,
            88,
            "unfold_plate",
            json!({"axis": "z", "from": 50.0, "to": 60.0}),
        );

        // design_report writes the HTML dossier and returns summary.
        let rep = temp_path("html");
        let rep_res = ok(
            &mut s,
            89,
            "design_report",
            json!({"path": rep.to_string_lossy(), "draft": 0.62, "kg": 2.2}),
        );
        assert!(rep.exists(), "design_report must write file");
        assert!(rep_res["bytes"].as_u64().unwrap_or(0) > 1000);
        assert!(rep_res["sections"].as_array().unwrap().len() >= 6);
        assert_eq!(rep_res["imo_passed"], json!(false));
        let content = std::fs::read_to_string(&rep).unwrap();
        assert!(content.contains("Principal Particulars"));
        let _ = std::fs::remove_file(&rep);

        // save_hull then load_hull round-trips the control net.
        let fef = temp_path("fef");
        let saved = ok(&mut s, 17, "save_hull", json!({"path": fef}));
        assert_eq!(saved["points"], edited["points"]);
        let demoted = ok(&mut s, 18, "load_hull", json!({"path": fef}));
        assert_eq!(demoted["points"], edited["points"]);
        assert_eq!(demoted["faces"], edited["faces"]);
        assert_eq!(demoted["source"], json!(fef.to_string_lossy()));
        let _ = std::fs::remove_file(&fef);

        // load_hull {demo}: the half model, on request.
        let half_info = ok(
            &mut s,
            19,
            "load_hull",
            json!({"demo": true, "symmetric": true}),
        );
        assert!(half_info["layers"][0]["symmetric"]
            .as_bool()
            .unwrap_or(false));
        assert_eq!(half_info["name"], "SeaCraft 12");
    }

    #[test]
    fn mcp_rejects_malformed_and_hostile_inputs_without_panicking() {
        let mut s = McpServer::new();

        // Blank lines are not messages.
        assert!(s.handle_line("").is_none());
        assert!(s.handle_line("   \n").is_none());

        // Framing errors.
        let parse = send(&mut s, json!({"jsonrpc": "2.0", "id": 1}));
        assert!(parse.get("result").is_none());
        let bad = s.handle_line("this is not JSON").expect("a reply");
        let bad: Value = serde_json::from_str(&bad).expect("JSON");
        assert_eq!(bad["error"]["code"], PARSE_ERROR);
        assert_eq!(bad["id"], Value::Null);
        let deep = s.handle_line(&"[".repeat(400)).expect("a reply");
        let deep: Value = serde_json::from_str(&deep).expect("JSON");
        assert_eq!(deep["error"]["code"], PARSE_ERROR, "{deep}");

        // Wrong message shapes.
        for (line, code) in [
            ("[]", INVALID_REQUEST),                          // an empty batch
            ("42", INVALID_REQUEST),                          // not an object
            (r#"{"jsonrpc":"2.0","id":1}"#, INVALID_REQUEST), // no method
            (
                r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#,
                INVALID_REQUEST,
            ), // a bad id
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"no/such"}"#,
                METHOD_NOT_FOUND,
            ),
        ] {
            let reply = s
                .handle_line(line)
                .unwrap_or_else(|| panic!("no reply to {line}"));
            let v: Value = serde_json::from_str(&reply).expect("JSON");
            assert_eq!(v["error"]["code"], code, "{line} -> {v}");
        }
        // A batch over the cap is refused without processing anything.
        let big = Value::Array(
            (0..=MAX_BATCH_MESSAGES)
                .map(|i| json!({"jsonrpc": "2.0", "id": i, "method": "ping"}))
                .collect(),
        );
        let reply = s.handle_line(&big.to_string()).expect("a reply");
        let v: Value = serde_json::from_str(&reply).expect("JSON");
        assert_eq!(v["error"]["code"], INVALID_REQUEST, "{v}");
        // A short batch is answered in order.
        let batch = json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}]);
        let reply = s.handle_line(&batch.to_string()).expect("a reply");
        let v: Value = serde_json::from_str(&reply).expect("JSON");
        assert_eq!(v.as_array().map(Vec::len), Some(2), "{v}");

        // Notifications and stray responses get no reply.
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#)
            .is_none());
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","id":9,"result":{}}"#)
            .is_none());

        // Protocol errors for a bad `tools/call`.
        for params in [
            json!({}),                                               // no name
            json!({"name": 42}),                                     // name is not a string
            json!({"name": "no_such_tool", "arguments": {}}),        // unknown tool
            json!({"name": "hull_info", "arguments": []}),           // arguments not an object
            json!({"name": "hull_info", "arguments": {"bogus": 1}}), // unknown argument
            json!({"name": "move_point", "arguments": {"index": "one", "xyz": [0, 0, 0]}}),
            json!({"name": "render_png", "arguments": {"path": "x.png", "mode": "hologram"}}),
            json!({"name": "render_png", "arguments": {"path": "x.png", "view": "under"}}),
            json!({"name": "bonjean_curves", "arguments": {"drafts": "0.1,0.2"}}),
            json!({"name": "table_of_offsets", "arguments": {"waterlines": "0.1"}}),
            json!({"name": "split_face", "arguments": {"face": "first"}}),
            json!({"name": "transform_hull", "arguments": {"mirror": "w"}}),
            json!({"name": "insert_edge", "arguments": {"face": 0, "a": 1}}),
        ] {
            let v = request(&mut s, 20, "tools/call", params.clone());
            assert_eq!(v["error"]["code"], INVALID_PARAMS, "{params} -> {v}");
        }

        // Tool errors: hostile arguments are refused with a short message.
        let hostile: Vec<(&str, Value)> = vec![
            ("move_point", json!({"index": u64::MAX, "xyz": [0, 0, 0]})),
            ("move_point", json!({"index": 1e300, "xyz": [0, 0, 0]})),
            ("move_point", json!({"index": -1, "xyz": [0, 0, 0]})),
            ("move_point", json!({"index": 0, "xyz": [0, 0]})),
            ("move_point", json!({"index": 0, "xyz": [0, 0, 0, 0]})),
            ("move_point", json!({"index": 0, "xyz": ["a", 0, 0]})),
            ("move_point", json!({"index": 0, "xyz": [f64::MAX, 0, 0]})),
            ("set_crease", json!({"a": 0, "b": 0})),
            ("set_crease", json!({"a": u64::MAX, "b": 0})),
            ("load_hull", json!({})),
            ("load_hull", json!({"path": ""})),
            ("load_hull", json!({"path": "/no/such/hull.fef"})),
            ("load_hull", json!({"path": "/etc/passwd"})),
            ("save_hull", json!({"path": "/no/such/dir/hull.fef"})),
            ("project_set", json!({"project": {}})),
            ("hydro_compute", json!({"draft": 0.0})),
            ("hydro_compute", json!({"draft": -1.0})),
            ("hydro_compute", json!({"density": 0.0})),
            ("hydro_compute", json!({"heel_deg": 1e9})),
            ("stability_cross_curves", json!({"heels": []})),
            ("stability_cross_curves", json!({"heels": ["0"]})),
            ("stability_cross_curves", json!({"displacements": [0.0]})),
            ("bonjean_curves", json!({"drafts": []})),
            ("bonjean_curves", json!({"drafts": [0.0, 1.0]})),
            ("split_face", json!({"face": 999})),
            ("insert_edge", json!({"face": 999, "a": 0, "b": 1})),
            ("insert_edge", json!({"face": 0, "a": 0, "b": 1})),
            ("table_of_offsets", json!({"waterlines": []})),
            ("table_of_offsets", json!({"buttocks": [-0.5]})),
            ("export_linesplan", json!({"path": "/tmp/x.obj"})),
            ("export_linesplan", json!({"path": "/no/such/dir/x.svg"})),
            ("render_png", json!({"path": "/no/such/dir/x.png"})),
        ];
        for (i, (tool, args)) in hostile.into_iter().enumerate() {
            let message = fails(&mut s, 30 + i as u64, tool, args.clone());
            assert!(
                message.len() < 4000,
                "{tool} {args}: the error should be short, got {} bytes",
                message.len()
            );
        }

        // An enormous picture is clamped, not allocated or refused.
        let png = temp_path("png");
        let r = ok(
            &mut s,
            60,
            "render_png",
            json!({"path": png, "width": u64::MAX, "height": 16, "level": 0}),
        );
        assert_eq!(r["width"], MAX_SIDE, "{r}");
        assert!(std::fs::metadata(&png).is_ok());
        let _ = std::fs::remove_file(&png);

        // A line over the cap is answered with an error ...
        let long = "x".repeat(MAX_LINE + 1);
        let reply = s.handle_line(&long).expect("a reply");
        let v: Value = serde_json::from_str(&reply).expect("JSON");
        assert_eq!(v["error"]["code"], INVALID_REQUEST, "{v}");
        // ... and in a stream the rest of that line is dropped, so the next
        // message is still read.
        let mut input = Vec::new();
        input.extend(vec![b'x'; 100]);
        input.extend(b"\nshort\n");
        input.extend(vec![b'y'; 30]);
        let mut reader = std::io::BufReader::with_capacity(8, input.as_slice());
        assert_eq!(
            read_line(&mut reader, 10).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::InvalidData)
        );
        assert_eq!(
            read_line(&mut reader, 10).ok().flatten(),
            Some(b"short".to_vec())
        );
        assert!(
            read_line(&mut reader, 10).is_err(),
            "a last line without a newline can still overflow"
        );

        // After all of that the server still works and the model is unchanged.
        let pong = request(&mut s, 99, "ping", json!({}));
        assert_eq!(pong["result"], json!({}));
        let info = ok(&mut s, 100, "hull_info", json!({}));
        assert_eq!(info["name"], "SeaCraft 12");
        assert_eq!(info["points"], demo::demo_hull(false).points.len());
    }

    #[test]
    fn serve_writes_one_reply_per_message_and_nothing_for_notifications() {
        let mut s = McpServer::new();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "\n",
            "not json\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            "\n",
        );
        let mut out = Vec::new();
        s.serve(std::io::Cursor::new(input.as_bytes()), &mut out)
            .expect("a clean session");
        let lines: Vec<&str> = std::str::from_utf8(&out)
            .expect("UTF-8 replies")
            .lines()
            .collect();
        assert_eq!(lines.len(), 3, "{lines:?}");
        let first: Value = serde_json::from_str(lines[0]).expect("JSON");
        assert_eq!(first["id"], 1);
        assert_eq!(first["result"]["serverInfo"]["name"], "seacraft-mcp");
        let second: Value = serde_json::from_str(lines[1]).expect("JSON");
        assert_eq!(second["error"]["code"], PARSE_ERROR, "the unparseable line");
        let third: Value = serde_json::from_str(lines[2]).expect("JSON");
        assert_eq!(third["id"], 2, "the stream stayed in step");
        assert!(out.ends_with(b"\n"), "every reply is one line");
    }

    #[test]
    fn initialize_negotiates_the_protocol_version() {
        let mut s = McpServer::new();
        for v in SUPPORTED_VERSIONS {
            let r = request(&mut s, 1, "initialize", json!({"protocolVersion": v}));
            assert_eq!(r["result"]["protocolVersion"], v);
        }
        for v in [json!("1999-01-01"), json!(7), json!(null)] {
            let r = request(&mut s, 2, "initialize", json!({"protocolVersion": v}));
            assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        }
        let r = request(&mut s, 3, "initialize", json!({}));
        let res = &r["result"];
        assert_eq!(res["capabilities"]["resources"]["listChanged"], false);
        assert_eq!(res["serverInfo"]["title"], "SeaCraft");
        let text = res["instructions"].as_str().expect("instructions");
        assert!(text.contains("x forward"));
        assert!(text
            .trim_end()
            .ends_with("Backend: a headless in-process SeaCraft session."));
    }

    #[test]
    fn every_tool_carries_annotations_matching_its_effect() {
        let mut s = McpServer::new();
        let list = request(&mut s, 1, "tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().expect("tools");
        let mut read_only = Vec::new();
        for t in tools {
            let name = t["name"].as_str().expect("name");
            assert!(t["title"].is_string(), "{name}: title");
            let a = &t["annotations"];
            assert_eq!(a["title"], t["title"], "{name}");
            assert_eq!(a["openWorldHint"], false, "{name}");
            for k in ["readOnlyHint", "destructiveHint", "idempotentHint"] {
                assert!(a[k].is_boolean(), "{name}: {k}");
            }
            if a["readOnlyHint"] == true {
                assert_eq!(a["destructiveHint"], false, "{name}");
                read_only.push(name);
            }
        }
        for n in [
            "hull_info",
            "project_get",
            "hydro_compute",
            "stability_cross_curves",
            "trim_scan",
        ] {
            assert!(read_only.contains(&n), "{n} is read-only");
        }
        for n in [
            "load_hull",
            "save_hull",
            "project_set",
            "move_point",
            "transform_hull",
            "optimize_hull",
            "export_linesplan",
            "render_png",
        ] {
            assert!(!read_only.contains(&n), "{n} is not read-only");
        }
        let optimise = tools
            .iter()
            .find(|t| t["name"] == "optimize_hull")
            .expect("optimize_hull");
        assert_eq!(
            Effect::of(optimise),
            Effect::Edits,
            "it replaces the session hull in place"
        );
        assert!(
            optimise["description"]
                .as_str()
                .unwrap_or_default()
                .contains("EDITS THE MODEL"),
            "the description must say it edits the model"
        );
        let load = tools
            .iter()
            .find(|t| t["name"] == "load_hull")
            .expect("load_hull");
        assert_eq!(load["annotations"]["destructiveHint"], true);
        assert_eq!(Effect::of(load), Effect::Replaces);

        let gen = tools
            .iter()
            .find(|t| t["name"] == "generate_hull")
            .expect("generate_hull");
        assert_eq!(gen["annotations"]["destructiveHint"], true);
        assert_eq!(Effect::of(gen), Effect::Replaces);
    }

    #[test]
    fn optimize_hull_edits_the_session_hull_and_batch_rolls_it_back() {
        let mut s = McpServer::new();
        let v0 = ok(&mut s, 1, "hydro_compute", json!({}))["volume_m3"]
            .as_f64()
            .expect("volume");
        let args = json!({
            "speed_kn": 8.0, "vars": ["lb"], "kg": 1.0, "min_gm": 0.5,
            "max_evaluations": 8, "level": 2,
        });

        // A failing batch step rolls the optimisation back with it.
        let before = ok(&mut s, 2, "hull_info", json!({}));
        let msg = fails(
            &mut s,
            3,
            "batch",
            json!({"calls": [
                {"name": "optimize_hull", "arguments": args.clone()},
                {"name": "hull_info", "arguments": {"bogus": 1}},
            ]}),
        );
        assert!(
            msg.contains("bogus") || msg.contains("unknown argument"),
            "{msg}"
        );
        assert_eq!(
            ok(&mut s, 4, "hull_info", json!({}))["extents"],
            before["extents"]
        );

        // On its own it succeeds and the session hull is the optimised one.
        let r = ok(&mut s, 5, "optimize_hull", args);
        let rep = &r["report"];
        let (b, o) = (&rep["baseline"], &rep["optimised"]);
        assert!(
            o["rt_kn"].as_f64().unwrap_or(f64::MAX) <= b["rt_kn"].as_f64().unwrap_or(0.0) + 1e-12,
            "{r}"
        );
        assert_eq!(rep["variables"].as_array().map(Vec::len), Some(1), "{r}");
        assert_eq!(rep["constraints_met"], true, "{r}");
        assert!(
            rep["trajectory"]
                .as_array()
                .is_some_and(|t| !t.is_empty() && t.len() <= 8),
            "{r}"
        );
        assert_eq!(r["backend"], "headless");
        // The measurement draft followed the optimised hull, and the same
        // displacement floats at it.
        let h = ok(&mut s, 6, "hydro_compute", json!({}));
        assert!(
            (h["draft"].as_f64().unwrap_or(0.0) - r["draft"].as_f64().unwrap_or(-1.0)).abs()
                < 1e-12,
            "{h}"
        );
        let v = h["volume_m3"].as_f64().expect("volume");
        assert!((v - v0).abs() / v0 < 0.011, "{v} against {v0}");

        // Bad arguments are tool errors, not panics.
        fails(&mut s, 7, "optimize_hull", json!({"speed_kn": 0.0}));
        fails(
            &mut s,
            8,
            "optimize_hull",
            json!({"speed_kn": 8.0, "vars": ["wibble"]}),
        );
    }

    #[test]
    fn generate_hull_then_hull_info_succeeds() {
        let mut s = McpServer::new();
        let gen = ok(
            &mut s,
            1,
            "generate_hull",
            json!({
                "kind": "displacement",
                "lwl": 10.0,
                "beam": 3.0,
                "draft": 0.6,
                "depth": 1.6
            }),
        );
        assert_eq!(gen["kind"], "displacement");
        assert!((gen["lwl"].as_f64().unwrap() - 10.0).abs() <= 0.05);
        assert!((gen["bwl"].as_f64().unwrap() - 3.0).abs() <= 0.02);
        assert!((gen["t"].as_f64().unwrap() - 0.6).abs() <= 0.01);

        let info = ok(&mut s, 2, "hull_info", json!({}));
        assert_eq!(info["validation"]["ok"], true);
        assert_eq!(info["layers"][0]["symmetric"], true);
        assert!((info["principal"]["length"].as_f64().unwrap() - 10.0).abs() < 1e-6);
    }

    #[test]
    fn resources_list_and_read() {
        let mut s = McpServer::new();
        let list = request(&mut s, 1, "resources/list", json!({}));
        let uris: Vec<&str> = list["result"]["resources"]
            .as_array()
            .expect("resources")
            .iter()
            .filter_map(|r| r["uri"].as_str())
            .collect();
        assert_eq!(uris, vec!["seacraft://hull", "seacraft://tools"]);

        let hull = request(
            &mut s,
            2,
            "resources/read",
            json!({"uri": "seacraft://hull"}),
        );
        let c = &hull["result"]["contents"][0];
        assert_eq!(c["uri"], "seacraft://hull");
        assert_eq!(c["mimeType"], "application/json");
        let parsed: Value = serde_json::from_str(c["text"].as_str().expect("text")).expect("JSON");
        assert!(parsed.is_object());

        let cat = request(
            &mut s,
            3,
            "resources/read",
            json!({"uri": "seacraft://tools"}),
        );
        let text = cat["result"]["contents"][0]["text"].as_str().expect("text");
        for d in tool_definitions() {
            assert!(text.contains(d["name"].as_str().expect("name")));
        }
        assert!(text.contains("replaces the model"));

        let bad = request(
            &mut s,
            4,
            "resources/read",
            json!({"uri": "seacraft://nope"}),
        );
        assert_eq!(bad["error"]["code"], -32002);
        let none = request(&mut s, 5, "resources/read", json!({}));
        assert_eq!(none["error"]["code"], INVALID_PARAMS);

        let t = request(&mut s, 6, "resources/templates/list", json!({}));
        assert_eq!(t["result"]["resourceTemplates"], json!([]));
        let p = request(&mut s, 7, "prompts/list", json!({}));
        assert_eq!(p["result"]["prompts"], json!([]));
    }

    #[test]
    fn argument_validation_errors_name_the_field_and_failures_are_results() {
        let mut s = McpServer::new();
        let missing = request(
            &mut s,
            1,
            "tools/call",
            json!({"name": "move_point", "arguments": {"index": 0}}),
        );
        assert_eq!(missing["error"]["code"], INVALID_PARAMS);
        assert!(missing["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("xyz"));

        let wrong = request(
            &mut s,
            2,
            "tools/call",
            json!({"name": "move_point", "arguments": {"index": "zero", "xyz": [0, 0, 0]}}),
        );
        assert_eq!(wrong["error"]["code"], INVALID_PARAMS);
        assert!(wrong["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("index"));

        let unknown = request(&mut s, 3, "tools/call", json!({"name": "no_such_tool"}));
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);

        // A missing file is an execution failure: a result, not a protocol error.
        let msg = fails(
            &mut s,
            4,
            "load_hull",
            json!({"path": "/nonexistent/hull.fef"}),
        );
        assert!(!msg.is_empty());
    }

    #[test]
    fn loading_condition_returns_gm_fluid_below_gm_solid_with_slack_tank() {
        let mut s = McpServer::new();
        let inline = ok(
            &mut s,
            1,
            "loading_condition",
            json!({
                "case": {
                    "name": "Slack tank check",
                    "lightship": { "name": "Lightship", "mass_t": 8.2, "x": 4.2, "y": 0.0, "z": 1.1 },
                    "items": [
                        { "name": "Crew", "mass_t": 0.4, "x": 4.8, "y": 0.0, "z": 1.3 }
                    ],
                    "tanks": [
                        {
                            "name": "Fuel tank",
                            "x_min": 2.8, "x_max": 5.3,
                            "y_min": -0.8, "y_max": 0.8,
                            "z_min": 0.12, "z_max": 0.72,
                            "fill": 0.5,
                            "density_t_m3": 0.85,
                            "permeability": 0.98,
                            "clip_to_hull": false
                        }
                    ]
                }
            }),
        );
        let gm_solid = inline["gm_solid"].as_f64().expect("gm_solid");
        let gm_fluid = inline["gm_fluid"].as_f64().expect("gm_fluid");
        let fsc = inline["fsc_m"].as_f64().expect("fsc_m");
        assert!(fsc > 0.0, "slack tank must produce positive FSC: {inline}");
        assert!(
            gm_fluid < gm_solid,
            "GM_fluid {gm_fluid} must be < GM_solid {gm_solid}"
        );
        assert!((gm_solid - gm_fluid - fsc).abs() < 1e-12);

        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-loadcase.json");
        let from_file = ok(
            &mut s,
            2,
            "loading_condition",
            json!({ "case_path": path.to_string_lossy() }),
        );
        assert!(
            from_file["gm_fluid"].as_f64().unwrap() < from_file["gm_solid"].as_f64().unwrap(),
            "{from_file}"
        );
        assert!(
            from_file["imo_criteria"]["passed"].is_boolean(),
            "{from_file}"
        );

        fails(&mut s, 3, "loading_condition", json!({}));
    }

    #[test]
    fn project_set_then_project_get_round_trip() {
        let mut s = McpServer::new();
        let initial = ok(&mut s, 1, "project_get", json!({}));
        assert_eq!(initial["format"], "seacraft-project");
        assert_eq!(initial["version"], 1);

        let windage = json!([{ "name": "Deckhouse", "area_m2": 4.2, "z_centroid": 2.5 }]);
        let project_json = json!({
            "format": "seacraft-project",
            "version": 1,
            "windage": windage,
            "loading_conditions": [],
            "compartments": [],
            "damage_cases": [],
        });
        let set_res = ok(&mut s, 2, "project_set", json!({ "project": project_json }));
        assert_eq!(set_res["format"], "seacraft-project");

        let get_res = ok(&mut s, 3, "project_get", json!({}));
        assert_eq!(get_res["windage"], windage);

        // save_hull writes the sidecar next to the model; load_hull reads it back.
        let dir = std::env::temp_dir().join(format!("seacraft-mcp-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fef = dir.join("hull.fef");
        let saved = ok(
            &mut s,
            4,
            "save_hull",
            json!({ "path": fef.to_string_lossy() }),
        );
        assert!(
            saved["project"]
                .as_str()
                .unwrap()
                .ends_with("hull.seacraft.json"),
            "{saved}"
        );
        let mut t = McpServer::new();
        ok(
            &mut t,
            5,
            "load_hull",
            json!({ "path": fef.to_string_lossy() }),
        );
        assert_eq!(ok(&mut t, 6, "project_get", json!({}))["windage"], windage);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loading_condition_by_name_equals_inline_case() {
        let mut s = McpServer::new();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-loadcase.json");
        let text = std::fs::read_to_string(&path).expect("demo-loadcase.json");
        let cond: seacraft_core::loading::LoadingCondition =
            serde_json::from_str(&text).expect("parse demo-loadcase.json");

        let inline_res = ok(
            &mut s,
            1,
            "loading_condition",
            json!({ "case": serde_json::to_value(&cond).unwrap() }),
        );

        let project_json = json!({
            "format": "seacraft-project",
            "version": 1,
            "loading_conditions": [serde_json::to_value(&cond).unwrap()],
            "compartments": [],
            "damage_cases": [],
            "windage": [],
        });
        ok(&mut s, 2, "project_set", json!({ "project": project_json }));

        let name_res = ok(&mut s, 3, "loading_condition", json!({ "name": cond.name }));

        assert_eq!(inline_res["displacement_t"], name_res["displacement_t"]);
        assert_eq!(inline_res["draft"], name_res["draft"]);
        assert_eq!(inline_res["trim_m"], name_res["trim_m"]);
        assert_eq!(inline_res["kg_fluid"], name_res["kg_fluid"]);
        assert_eq!(inline_res["gm_fluid"], name_res["gm_fluid"]);

        let default_res = ok(&mut s, 4, "loading_condition", json!({}));
        assert_eq!(inline_res["displacement_t"], default_res["displacement_t"]);
        assert_eq!(inline_res["draft"], default_res["draft"]);
    }

    #[test]
    fn damage_stability_tool_finds_equilibrium_and_measures_residual_gz() {
        let mut s = McpServer::new();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-damage.json");
        let res = ok(
            &mut s,
            1,
            "damage_stability",
            json!({ "case_path": path.to_string_lossy() }),
        );
        assert_eq!(res["backend"], "headless");
        assert_eq!(res["equilibrium_found"], true);
        assert!(res["sinkage_m"].as_f64().unwrap() > 0.0);
        let crit = &res["criteria"];
        assert_eq!(crit["passed"], true);
        fails(&mut s, 2, "damage_stability", json!({}));
    }

    #[test]
    fn damage_stability_by_name_equals_inline_case() {
        let mut s = McpServer::new();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-damage.json");
        let text = std::fs::read_to_string(&path).expect("demo-damage.json");
        let case: seacraft_core::damage::DamageCase =
            serde_json::from_str(&text).expect("parse demo-damage.json");

        let inline_res = ok(
            &mut s,
            1,
            "damage_stability",
            json!({ "case": serde_json::to_value(&case).unwrap(), "level": 2 }),
        );

        let project_json = json!({
            "format": "seacraft-project",
            "version": 1,
            "loading_conditions": [],
            "compartments": [],
            "damage_cases": [serde_json::to_value(&case).unwrap()],
            "windage": [],
        });
        ok(&mut s, 2, "project_set", json!({ "project": project_json }));

        let name_res = ok(
            &mut s,
            3,
            "damage_stability",
            json!({ "name": case.name, "level": 2 }),
        );

        assert_eq!(inline_res["displacement_t"], name_res["displacement_t"]);
        assert_eq!(inline_res["draft"], name_res["draft"]);
        assert_eq!(inline_res["trim_m"], name_res["trim_m"]);
        assert_eq!(inline_res["heel_deg"], name_res["heel_deg"]);
        assert_eq!(
            inline_res["equilibrium_found"],
            name_res["equilibrium_found"]
        );

        let default_res = ok(&mut s, 4, "damage_stability", json!({ "level": 2 }));
        assert_eq!(inline_res["displacement_t"], default_res["displacement_t"]);
        assert_eq!(inline_res["draft"], default_res["draft"]);
    }

    #[test]
    fn seakeeping_raos_tool_evaluates_demo_hull_motions() {
        let mut s = McpServer::new();
        let res = ok(
            &mut s,
            1,
            "seakeeping_raos",
            json!({
                "draft": 0.62,
                "speed_kn": 6.0,
                "kg": 2.2,
                "points": 5,
                "stations": 11,
            }),
        );
        assert_eq!(res["backend"], "headless");
        let freqs = res["frequencies"].as_array().expect("frequencies array");
        assert_eq!(freqs.len(), 5);
        assert!(res["natural_heave_omega"].as_f64().unwrap() > 0.0);
        assert!(res["natural_pitch_omega"].as_f64().unwrap() > 0.0);
        fails(&mut s, 2, "seakeeping_raos", json!({ "draft": -1.0 }));
    }

    #[test]
    fn seakeeping_irregular_tool_reports_sea_state_responses() {
        let mut s = McpServer::new();
        let res = ok(
            &mut s,
            1,
            "seakeeping_irregular",
            json!({
                "draft": 0.62,
                "speed_kn": 6.0,
                "kg": 0.392,
                "stations": 11,
                "hs": 1.0,
                "tp": 4.0,
                "gamma": 3.3,
                "omegas": 24,
                "points": [{"name": "Mid", "x": 0.0}],
            }),
        );
        assert_eq!(res["backend"], "headless");
        assert_eq!(res["sea"]["kind"], "jonswap");
        assert_eq!(res["omega_count"], 24);
        assert!(res["heave"]["significant"].as_f64().unwrap() > 0.0);
        assert!(res["pitch_deg"]["significant"].as_f64().unwrap() > 0.0);
        let pts = res["points"].as_array().expect("points");
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0]["name"], "Mid");
        assert!(pts[0]["msi_percent"].as_f64().unwrap() >= 0.0);
        // Default points are LCG and Bow.
        let dflt = ok(
            &mut s,
            2,
            "seakeeping_irregular",
            json!({ "draft": 0.62, "hs": 1.0, "tp": 4.0, "stations": 11, "omegas": 16 }),
        );
        assert_eq!(dflt["sea"]["kind"], "bretschneider");
        assert_eq!(dflt["points"][0]["name"], "LCG");
        assert_eq!(dflt["points"][1]["name"], "Bow");
        // A missing required argument is a protocol error (-32602), not a tool error.
        let missing = request(
            &mut s,
            3,
            "tools/call",
            json!({"name": "seakeeping_irregular", "arguments": { "draft": 0.62 }}),
        );
        assert_eq!(missing["error"]["code"], -32602, "{missing}");
        fails(
            &mut s,
            4,
            "seakeeping_irregular",
            json!({ "draft": 0.62, "hs": -1.0, "tp": 4.0 }),
        );
        let wrong_type = request(
            &mut s,
            5,
            "tools/call",
            json!({"name": "seakeeping_irregular",
                   "arguments": { "draft": 0.62, "hs": 1.0, "tp": 4.0, "points": "bow" }}),
        );
        assert_eq!(wrong_type["error"]["code"], -32602, "{wrong_type}");
    }

    #[test]
    fn batch_tool_and_panic_safety_tests() {
        let mut s = McpServer::new();

        // 1. batch success (two calls -> two results)
        let success = ok(
            &mut s,
            1,
            "batch",
            json!({
                "calls": [
                    {
                        "name": "move_point",
                        "arguments": {"index": 0, "xyz": [0.0, 0.0, 0.5]}
                    },
                    {
                        "name": "hull_info",
                        "arguments": {}
                    }
                ]
            }),
        );
        let results = success["results"].as_array().expect("results array");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["index"], 0);
        assert_eq!(results[1]["name"], "SeaCraft 12");

        // 2. batch rollback (move_point then a failing call -> hull_info shows the point unmoved)
        let before_info = ok(&mut s, 2, "hull_info", json!({}));
        let err_text = fails(
            &mut s,
            3,
            "batch",
            json!({
                "calls": [
                    {
                        "name": "move_point",
                        "arguments": {"index": 0, "xyz": [0.0, 0.0, 5.0]}
                    },
                    {
                        "name": "move_point",
                        "arguments": {"index": 999999, "xyz": [0.0, 0.0, 0.0]}
                    }
                ]
            }),
        );
        let fail_json: Value = serde_json::from_str(&err_text).expect("fail json");
        assert_eq!(fail_json["completed"], 1);
        assert_eq!(fail_json["failed"]["index"], 1);
        assert_eq!(fail_json["failed"]["name"], "move_point");
        assert!(fail_json["failed"]["error"]
            .as_str()
            .unwrap()
            .contains("out of range"));

        // Verify point 0 was rolled back: z is still 0.5, not 5.0
        let after_info = ok(&mut s, 4, "hull_info", json!({}));
        assert_eq!(after_info["points"], before_info["points"]);
        let check_pt = ok(
            &mut s,
            5,
            "move_point",
            json!({"index": 0, "xyz": [0.0, 0.0, 0.5]}),
        );
        assert!((check_pt["before"]["z"].as_f64().unwrap() - 0.5).abs() < 1e-9);

        // 3. nested batch rejected
        let nested_err = fails(
            &mut s,
            6,
            "batch",
            json!({
                "calls": [
                    {
                        "name": "batch",
                        "arguments": {"calls": []}
                    }
                ]
            }),
        );
        let nested_json: Value = serde_json::from_str(&nested_err).expect("nested fail json");
        assert_eq!(nested_json["failed"]["name"], "batch");
        assert!(nested_json["failed"]["error"]
            .as_str()
            .unwrap()
            .contains("nested batch"));

        // 4. >100 calls rejected with -32602
        let calls_101: Vec<Value> = (0..101)
            .map(|_| json!({"name": "hull_info", "arguments": {}}))
            .collect();
        let too_many = request(
            &mut s,
            7,
            "tools/call",
            json!({"name": "batch", "arguments": {"calls": calls_101}}),
        );
        assert_eq!(too_many["error"]["code"], -32602);
        assert!(too_many["error"]["message"]
            .as_str()
            .unwrap()
            .contains("too many items"));

        // 5. deliberately panicking path does not kill the server — assert next ping still answers
        let panic_res = request(
            &mut s,
            8,
            "tools/call",
            json!({"name": "__test_panic", "arguments": {}}),
        );
        assert_eq!(panic_res["result"]["isError"], true);
        assert!(panic_res["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("internal error: deliberate test panic"));

        let ping = request(&mut s, 9, "ping", json!({}));
        assert_eq!(ping["result"], json!({}));
    }

    #[test]
    fn connected_batch_rollback_with_history_mark_restore() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();

            // 1. Expect history.mark
            line.clear();
            reader.read_line(&mut line).expect("read");
            let v: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v["method"], "history.mark");
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"mark\":42}}\n")
                .expect("write");

            // 2. Expect first tool call (point.move)
            line.clear();
            reader.read_line(&mut line).expect("read");
            let v: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v["method"], "point.move");
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"index\":0}}\n")
                .expect("write");

            // 3. Expect second tool call (model.split_face which fails)
            line.clear();
            reader.read_line(&mut line).expect("read");
            let v: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v["method"], "model.split_face");
            stream
                .write_all(b"{\"ok\":false,\"error\":\"split error\"}\n")
                .expect("write");

            // 4. Expect history.restore with mark 42
            line.clear();
            reader.read_line(&mut line).expect("read");
            let v: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v["method"], "history.restore");
            assert_eq!(v["params"]["mark"], 42);
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"restored\":true}}\n")
                .expect("write");
        });

        let mut s = McpServer::connect_no_auth(&addr.to_string()).expect("connect");
        assert!(s.is_remote());

        let err_text = fails(
            &mut s,
            1,
            "batch",
            json!({
                "calls": [
                    {
                        "name": "move_point",
                        "arguments": {"index": 0, "xyz": [0.0, 0.0, 1.0]}
                    },
                    {
                        "name": "split_face",
                        "arguments": {"face": 0}
                    }
                ]
            }),
        );
        let fail_json: Value = serde_json::from_str(&err_text).expect("fail json");
        assert_eq!(fail_json["completed"], 1);
        assert_eq!(fail_json["failed"]["index"], 1);
        assert_eq!(fail_json["failed"]["name"], "split_face");
        assert!(fail_json["failed"]["error"]
            .as_str()
            .unwrap()
            .contains("split error"));

        handle.join().expect("server thread finished");
    }

    #[test]
    fn mcp_connect_with_token_file_succeeds_end_to_end() {
        use std::io::Write;
        let token = seacraft_core::control_auth::generate_token().expect("generate token");
        let temp_dir = std::env::temp_dir();
        let token_path = temp_dir.join(format!("seacraft-test-mcp-{}.token", std::process::id()));
        seacraft_core::control_auth::write_token_file(&token_path, &token).expect("write token");
        let _guard = seacraft_core::control_auth::TokenFile(token_path.clone());

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let expected_token = token.clone();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();

            // 1. First line must be auth
            reader.read_line(&mut line).expect("read");
            let v: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v["method"], "auth");
            assert_eq!(v["params"]["token"], expected_token);
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"authenticated\":true}}\n")
                .expect("write");

            // 2. Next line is the tool request
            line.clear();
            reader.read_line(&mut line).expect("read");
            let v2: Value = serde_json::from_str(line.trim()).expect("json");
            assert_eq!(v2["method"], "ui.inspect");
            stream
                .write_all(b"{\"ok\":true,\"result\":{\"version\":\"0.1.0-beta.48\"}}\n")
                .expect("write");
        });

        // Test connecting with the token file
        let mut s = McpServer::connect(&addr.to_string(), Some(&token_path))
            .expect("connect with token file");
        assert!(s.is_remote());

        let info = s.call_tool("hull_info", &json!({}));
        assert_eq!(
            info["content"][0]["text"],
            "{\n  \"version\": \"0.1.0-beta.48\"\n}"
        );

        handle.join().expect("server thread finished");

        // Missing token file returns clear error
        let non_existent = temp_dir.join(format!("seacraft-missing-{}.token", std::process::id()));
        let err = match McpServer::connect(&addr.to_string(), Some(&non_existent)) {
            Err(e) => e,
            Ok(_) => panic!("expected missing token file to fail"),
        };
        assert!(err.contains("cannot read control token file"));
    }
}
