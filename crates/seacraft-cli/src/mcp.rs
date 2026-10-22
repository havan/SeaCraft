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
use seacraft_core::subdiv::SubdivisionMode;
use seacraft_core::{curves, demo, export, fef, linesplan, offsets, raster, topology};
use serde_json::{json, Map, Value};

/// The protocol revision this server speaks (`initialize` answers with it).
pub const PROTOCOL_VERSION: &str = "2024-11-05";

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

const INSTRUCTIONS: &str = "SeaCraft models a hull as a subdivision surface and measures it. Axes: x forward, \
y to starboard, z up, metres and kg; the keel is the lowest hydrostatics point, so KB/BM/KM/KG are above it. \
A symmetric layer models only y >= 0 and is mirrored on evaluation. Typical use: `hull_info` for the control \
net and its layers, extents and principal dimensions; `load_hull` {path} or {demo: true, symmetric}; \
`hydro_compute` {draft, trim, heel_deg, kg, free_trim, lcg} for one floating condition (free_trim solves the trim \
that puts the LCB on the LCG); `stability_cross_curves` for KN/GZ with the IMO A.749 criteria; `bonjean_curves` \
for the section area at each station against a range of drafts; `table_of_offsets` for the drawing-office \
half-breadth and buttock tables; `move_point` and \
`set_crease`, `split_face` and `insert_edge` to edit; `export_linesplan` (.svg/.dxf/.igs) and `render_png` (shaded, zebra, curvature, draft) to \
look at the result; `save_hull` to keep it. All measurement happens on the refined mesh.";

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
    remote: Option<Remote>,
    initialized: bool,
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer {
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
            remote: None,
            initialized: false,
        }
    }

    /// A server whose model is the `.fef` file at `path`.
    pub fn load_file(path: &Path) -> Result<Self, String> {
        let mut server = Self::from_surface(read_fef(path)?);
        server.source = Some(path.to_path_buf());
        Ok(server)
    }

    /// A server that forwards tool calls to `seacraft-gui --control addr`.
    pub fn connect(addr: &str) -> Result<Self, String> {
        let remote = Remote::connect(addr).map_err(|e| format!("cannot reach {addr}: {e}"))?;
        let mut server = Self::new();
        server.remote = Some(remote);
        Ok(server)
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
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                let version = if asked == PROTOCOL_VERSION {
                    asked
                } else {
                    PROTOCOL_VERSION
                };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "seacraft-mcp", "title": "SeaCraft", "version": seacraft_core::VERSION},
                    "instructions": format!("{INSTRUCTIONS} Backend: {}.", self.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
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
        let result = match &mut self.remote {
            Some(remote) => remote.tool(name, a),
            None => self.run_tool(name, a),
        };
        match result {
            Ok(v) => text_result(&v, false),
            Err(e) => text_result(&Value::String(e), true),
        }
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
            "save_hull" => self.save_hull(a),
            "move_point" => self.move_point(a),
            "set_crease" => self.set_crease(a),
            "split_face" => self.split_face(a),
            "insert_edge" => self.insert_edge(a),
            "hydro_compute" => self.hydro_compute(a),
            "stability_cross_curves" => self.stability_curves(a),
            "bonjean_curves" => self.bonjean_curves(a),
            "table_of_offsets" => self.table_of_offsets(a),
            "export_linesplan" => self.export_linesplan(a),
            "render_png" => self.render_png(a),
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
        match str_arg(a, "path") {
            Some(path) if !path.trim().is_empty() => {
                let surface = read_fef(Path::new(path))?;
                self.surface = surface;
                self.opts = HydroOptions::from_surface(
                    &self.surface,
                    seascape_subdiv(self.opts.subdivision.level),
                );
                self.source = Some(PathBuf::from(path));
            }
            _ if demo => {
                self.surface = demo::demo_hull(bool_arg(a, "symmetric").unwrap_or(false));
                self.opts = HydroOptions::from_surface(
                    &self.surface,
                    seascape_subdiv(self.opts.subdivision.level),
                );
                self.source = None;
            }
            _ => return Err("load_hull: give `path` or `demo: true`".to_string()),
        }
        Ok(self.hull_info())
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
        Ok(json!({
            "path": path.to_string_lossy(),
            "bytes": text.len(),
            "points": self.surface.points.len(),
            "faces": self.surface.faces.len(),
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
        let face = usize_arg(a, "face")
            .filter(|f| *f < faces)
            .ok_or_else(|| format!("split_face: `face` must be from 0 to {}", faces.saturating_sub(1)))?;
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
        let face = usize_arg(a, "face")
            .filter(|f| *f < faces)
            .ok_or_else(|| format!("insert_edge: `face` must be from 0 to {}", faces.saturating_sub(1)))?;
        let pick = |key: &str| -> Result<usize, String> {
            usize_arg(a, key)
                .filter(|v| *v < points)
                .ok_or_else(|| format!("insert_edge: `{key}` must be from 0 to {}", points.saturating_sub(1)))
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
                "bonjean_curves: the hull has no height above the keel (is a layer hidden?)".to_string(),
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

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "destructiveHint": !read_only,
            "idempotentHint": read_only,
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
            true,
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
            false,
        ),
        tool(
            "save_hull",
            "Save the hull",
            "Write the control net as a FREE!ship `.fef` file. Without `path` the model is written back to the \
             file it came from.",
            obj(json!({"path": string("Output .fef path")}), &[]),
            false,
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
            false,
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
            false,
        ),
        tool(
            "split_face",
            "Split a face",
            "Replace control face `face` with a triangle fan around a new point at its centroid. The face keeps its \
             index and layer, so a later `insert_edge` naming the same index refers to the first triangle of the \
             fan. Every new edge is interior to the face, so the net stays closed and manifold; on a planar face \
             the geometry is untouched, on a curved one the fan refines to a slightly different surface.",
            obj(json!({"face": integer("Control face index (see hull_info for the count)")}), &["face"]),
            false,
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
            false,
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
                    "level": integer("Subdivision level 0-6 (default 3)"),
                }),
                &[],
            ),
            true,
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
            true,
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
            true,
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
            true,
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
            false,
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
            false,
        ),
    ]
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
/// `export_linesplan`
/// -> `model.export`, `render_png` -> `ui.render`. The app's reply is returned
/// as the tool's result (its own field names); a failure is a tool error.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
}

impl Remote {
    /// Connect to `addr` (`host:port`), failing at once when nothing listens.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self {
            addr: addr.to_string(),
            conn: None,
        };
        r.reconnect()?;
        Ok(r)
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
            "bonjean_curves" => self.request("curves.bonjean", Value::Object(a.clone())),
            "table_of_offsets" => self.request("curves.offsets", Value::Object(a.clone())),
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

        // tools/list: the thirteen hull tools, each with a JSON Schema.
        let list = request(&mut s, 4, "tools/list", json!({}));
        let tools = list["result"]["tools"].as_array().expect("a tool array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            vec![
                "hull_info",
                "load_hull",
                "save_hull",
                "move_point",
                "set_crease",
                "split_face",
                "insert_edge",
                "hydro_compute",
                "stability_cross_curves",
                "bonjean_curves",
                "table_of_offsets",
                "export_linesplan",
                "render_png",
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
        assert!((mid["x"].as_f64().unwrap_or(0.0) - 6.0).abs() < 0.6, "{mid}");
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
        assert!(rows.iter().any(|r| r["waterline_half_breadths"][0].is_null()), "{ofs}");
        assert!(bow["buttock_heights"][0].is_null() || bow["buttock_heights"][0].as_f64().unwrap_or(0.0) > 0.0);
        // Defaults cover the depth and the half beam.
        let dflt = ok(&mut s, 63, "table_of_offsets", json!({"waterline_steps": 3, "buttock_steps": 2}));
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
        assert_eq!(split["pieces"], 4, "a quad fans into four triangles: {split}");
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
        fails(&mut s, 76, "insert_edge", json!({"face": 0, "a": new_point, "b": 0}));
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
}
