//! Programmatic control of the running app: a loopback JSON-lines socket for
//! agents and scripted tests.
//!
//! `--control HOST:PORT` binds the socket (see [`spawn_listener`]); every
//! accepted connection sends one request per JSON line and gets one
//! single-line JSON reply, `{"ok": true, "result": …}` or
//! `{"ok": false, "error": "…"}`. The app serves the queue once per frame
//! ([`crate::App::process_control`]), so a request reads and edits exactly the
//! state the next painted frame shows, and input methods (`ui.click`,
//! `ui.drag`, …) are injected into the next frame's [`egui::RawInput`].
//!
//! Adapted from Tangency Labs' `fieldcraft/crates/ui-egui/src/control.rs`
//! (MIT OR Apache-2.0): the request/reply shape, the synthetic-input queue and
//! the validate-then-apply style. Nothing is copied verbatim — the methods
//! below are SeaCraft's own. See `ATTRIBUTION.md`.
//!
//! Requests are untrusted input: every number is checked for finiteness and
//! range, every index against the model's size, and nothing here panics.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use seacraft_core::math::Vec3;
use seacraft_core::{AnalysisMode, SubdivisionMode};
use serde_json::{json, Value};

use crate::render::{self, Scene};
use crate::App;

/// Longest accepted request line. A hostile client cannot make the reader
/// allocate more than this per line.
const MAX_LINE: usize = 64 * 1024;
/// How long a connection waits for the window to answer one request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest image side `ui.render` accepts, in pixels.
const MAX_RENDER: usize = 4096;
/// The default `ui.render` image size.
const RENDER_DEFAULT: [usize; 2] = [1280, 800];
/// Most synthetic events that may wait for a frame.
pub const MAX_SYNTHETIC: usize = 512;
/// Largest absolute angle or length a camera request may give, in metres.
const MAX_CAMERA: f64 = 1e6;
/// Largest absolute position a synthetic pointer event may give, in points.
const MAX_POINT: f64 = 1e6;
/// Pitch stops just short of the pole, where the camera basis degenerates.
const MAX_PITCH: f64 = std::f64::consts::FRAC_PI_2 - 0.001;

/// One request from the socket, handed to the app thread with the channel that
/// carries the reply back.
pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

impl ControlRequest {
    /// A request whose reply goes to `reply`. The socket thread makes the
    /// channel and waits on it, so the window never blocks on a client.
    pub fn new(method: impl Into<String>, params: Value, reply: Sender<Value>) -> Self {
        Self {
            method: method.into(),
            params,
            reply,
        }
    }
}

/// Options for starting the loopback control socket.
#[derive(Clone, Debug, Default)]
pub struct ControlOptions {
    /// Explicit token file path (overrides default $XDG_RUNTIME_DIR/seacraft-control-<port>.token).
    pub token_file: Option<std::path::PathBuf>,
    /// Disable control channel authentication (warning printed).
    pub no_auth: bool,
    /// Explicit token string (used in testing).
    pub explicit_token: Option<String>,
}

/// The result of spawning a control listener.
pub struct ControlListener {
    pub addr: SocketAddr,
    pub token: Option<String>,
    pub token_file: Option<seacraft_core::control_auth::TokenFile>,
    pub receiver: Receiver<ControlRequest>,
}

/// Bind `addr` and serve JSON lines on a background thread.
///
/// Returns the listener information (bound address, token, token file RAII guard, and request receiver).
pub fn spawn_listener(addr: &str, opts: ControlOptions) -> std::io::Result<ControlListener> {
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    let (tx, rx) = mpsc::channel();

    let (token, token_file) = if opts.no_auth {
        eprintln!(
            "seacraft-gui: warning: control channel authentication is disabled (--control-no-auth)"
        );
        (None, None)
    } else {
        let token = match opts.explicit_token {
            Some(t) => t,
            None => seacraft_core::control_auth::generate_token()?,
        };
        let token_path = opts
            .token_file
            .unwrap_or_else(|| seacraft_core::control_auth::default_token_path(local.port()));
        seacraft_core::control_auth::write_token_file(&token_path, &token)?;
        eprintln!(
            "seacraft-gui: control token written to {}",
            token_path.display()
        );
        let token_file = Some(seacraft_core::control_auth::TokenFile(token_path));
        (Some(token), token_file)
    };

    let thread_token = token.clone();
    std::thread::Builder::new()
        .name("seacraft-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                let thread_token = thread_token.clone();
                let spawned = std::thread::Builder::new()
                    .name("seacraft-control-conn".into())
                    .spawn(move || serve(stream, &tx, thread_token));
                if spawned.is_err() {
                    // Out of threads: refusing the connection is the only
                    // safe answer, and it must not take the listener down.
                    continue;
                }
            }
        })?;
    Ok(ControlListener {
        addr: local,
        token,
        token_file,
        receiver: rx,
    })
}

/// Read one line, at most `max` bytes, from `reader`.
///
/// `Ok(None)` at end of stream. A line that does not fit is reported as
/// [`Line::TooLong`] and the rest of it is discarded, so the next line still
/// parses — the reader never holds more than `max` bytes.
fn read_line(reader: &mut impl BufRead, max: usize) -> std::io::Result<Option<Line>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            if buf.is_empty() && !too_long {
                return Ok(None);
            }
            break;
        }
        let (end, consume) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i, i + 1),
            None => (available.len(), available.len()),
        };
        if !too_long {
            if buf.len() + end > max {
                too_long = true;
                buf = Vec::new();
            } else {
                buf.extend_from_slice(&available[..end]);
            }
        }
        reader.consume(consume);
        if consume > end {
            break; // the newline was there
        }
    }
    if too_long {
        return Ok(Some(Line::TooLong));
    }
    match String::from_utf8(buf) {
        Ok(text) => Ok(Some(Line::Text(text))),
        Err(_) => Ok(Some(Line::NotUtf8)),
    }
}

enum Line {
    Text(String),
    TooLong,
    NotUtf8,
}

fn error(message: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "error": message.to_string() })
}

fn serve(stream: TcpStream, tx: &Sender<ControlRequest>, expected_token: Option<String>) {
    let _ = stream.set_nodelay(true);
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut writer = stream;
    let mut reader = BufReader::new(read_half);
    let mut authenticated = expected_token.is_none();
    loop {
        let line = match read_line(&mut reader, MAX_LINE) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(_) => break,
        };
        let text = match line {
            Line::TooLong => {
                let _ = write_reply(
                    &mut writer,
                    &error(format!("line longer than {} KiB", MAX_LINE / 1024)),
                );
                if !authenticated {
                    break;
                }
                continue;
            }
            Line::NotUtf8 => {
                let _ = write_reply(&mut writer, &error("a request line must be UTF-8"));
                if !authenticated {
                    break;
                }
                continue;
            }
            Line::Text(text) if text.trim().is_empty() => continue,
            Line::Text(text) => text,
        };
        let (method, params) = match parse_request(&text) {
            Ok(pair) => pair,
            Err(e) => {
                let _ = write_reply(&mut writer, &error(e));
                if !authenticated {
                    break;
                }
                continue;
            }
        };

        if !authenticated {
            if method == "auth" {
                let token_param = params.get("token").and_then(Value::as_str).unwrap_or("");
                let expected = expected_token.as_deref().unwrap_or("");
                if seacraft_core::control_auth::tokens_match(token_param, expected) {
                    authenticated = true;
                    if !write_reply(
                        &mut writer,
                        &json!({ "ok": true, "result": { "authenticated": true } }),
                    ) {
                        break;
                    }
                    continue;
                } else {
                    let _ = write_reply(&mut writer, &error("unauthorized: invalid token"));
                    break;
                }
            } else {
                let _ = write_reply(
                    &mut writer,
                    &error("unauthorized: first request must be auth"),
                );
                break;
            }
        }

        if method == "auth" {
            let token_param = params.get("token").and_then(Value::as_str).unwrap_or("");
            let expected = expected_token.as_deref().unwrap_or("");
            if expected_token.is_none()
                || seacraft_core::control_auth::tokens_match(token_param, expected)
            {
                if !write_reply(
                    &mut writer,
                    &json!({ "ok": true, "result": { "authenticated": true } }),
                ) {
                    break;
                }
            } else {
                let _ = write_reply(&mut writer, &error("unauthorized: invalid token"));
                break;
            }
            continue;
        }

        let (reply_tx, reply_rx) = mpsc::channel();
        if tx
            .send(ControlRequest::new(method, params, reply_tx))
            .is_err()
        {
            break; // the window is gone
        }
        let reply = match reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => error(format!(
                "the window did not answer within {} s",
                REPLY_TIMEOUT.as_secs()
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if !write_reply(&mut writer, &reply) {
            break;
        }
    }
}

/// Parse one request line into `(method, params)`. The params default to `{}`.
fn parse_request(text: &str) -> Result<(String, Value), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .ok_or("a request needs a string `method`")?
        .to_string();
    let params = match value.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p) => p.clone(),
    };
    Ok((method, params))
}

/// Write one reply line. A value serde_json cannot render (it cannot happen for
/// the replies produced here) still gets an answer rather than silence.
fn write_reply(stream: &mut TcpStream, reply: &Value) -> bool {
    let text = serde_json::to_string(reply).unwrap_or_else(|_| {
        "{\"ok\":false,\"error\":\"the reply could not be encoded\"}".to_string()
    });
    stream.write_all(text.as_bytes()).is_ok()
        && stream.write_all(b"\n").is_ok()
        && stream.flush().is_ok()
}

// ---- parameter helpers -------------------------------------------------

/// A finite `f64` from `key`, or an error naming the type problem.
fn number(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|x| x.is_finite())
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a finite number")),
    }
}

/// An optional array of finite numbers from `key`.
fn numbers(p: &Value, key: &str) -> Result<Option<Vec<f64>>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > 10_000 {
                return Err(format!("`{key}` has too many entries (at most 10000)"));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_f64().filter(|x| x.is_finite()) {
                    Some(n) => out.push(n),
                    None => return Err(format!("`{key}` must contain only finite numbers")),
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(format!("`{key}` must be an array of numbers")),
    }
}

/// A non-empty list of positive ordinates, sorted ascending.
fn positive_ordinates(mut v: Vec<f64>, key: &str) -> Result<Vec<f64>, String> {
    if v.is_empty() {
        return Err(format!(
            "curves.offsets: `{key}` needs at least one ordinate"
        ));
    }
    if v.iter().any(|x| *x <= 0.0) {
        return Err(format!("curves.offsets: `{key}` must be positive"));
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(v)
}

/// A finite `f64` from `key` inside `range`.
fn number_in(
    p: &Value,
    key: &str,
    range: std::ops::RangeInclusive<f64>,
) -> Result<Option<f64>, String> {
    match number(p, key)? {
        None => Ok(None),
        Some(x) if range.contains(&x) => Ok(Some(x)),
        Some(_) => Err(format!(
            "`{key}` must be from {} to {}",
            range.start(),
            range.end()
        )),
    }
}

fn boolean(p: &Value, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(v)) => Ok(Some(*v)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

fn text<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("`{key}` must be a string")),
    }
}

/// A whole number in `0..max` (exclusive `max`).
fn index_at(p: &Value, key: &str, max: usize) -> Result<Option<usize>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let x = v
                .as_f64()
                .filter(|x| x.is_finite() && x.fract() == 0.0 && *x >= 0.0 && *x < max as f64)
                .ok_or_else(|| {
                    format!(
                        "`{key}` must be a whole number from 0 to {}",
                        max.saturating_sub(1)
                    )
                })?;
            Ok(Some(x as usize))
        }
    }
}

fn u64_arg(p: &Value, key: &str) -> Result<Option<u64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .or_else(|| {
                v.as_f64()
                    .filter(|x| x.is_finite() && x.fract() == 0.0 && *x >= 0.0)
                    .map(|x| x as u64)
            })
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be an unsigned integer")),
    }
}

/// Exactly three finite numbers.
fn triple(v: &Value, key: &str) -> Result<Vec3, String> {
    let bad = || format!("`{key}` must be three finite numbers [x, y, z]");
    let list = v.as_array().filter(|a| a.len() == 3).ok_or_else(bad)?;
    let mut out = [0.0; 3];
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value.as_f64().filter(|x| x.is_finite()).ok_or_else(bad)?;
    }
    Ok(Vec3::new(out[0], out[1], out[2]))
}

/// A pointer position in window points.
fn position(p: &Value) -> Result<egui::Pos2, String> {
    let bad = || "`x` and `y` must be finite window coordinates".to_string();
    let x = p
        .get("x")
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && x.abs() <= MAX_POINT)
        .ok_or_else(bad)?;
    let y = p
        .get("y")
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && x.abs() <= MAX_POINT)
        .ok_or_else(bad)?;
    Ok(egui::pos2(x as f32, y as f32))
}

/// `[x, y]` as a pointer position, for the drag endpoints.
fn point_pair(p: &Value, key: &str) -> Result<Option<egui::Pos2>, String> {
    let Some(v) = p.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    let bad = || format!("`{key}` must be [x, y] in window points");
    let list = v.as_array().filter(|a| a.len() == 2).ok_or_else(bad)?;
    let mut out = [0.0f32; 2];
    for (slot, value) in out.iter_mut().zip(list) {
        *slot = value
            .as_f64()
            .filter(|x| x.is_finite() && x.abs() <= MAX_POINT)
            .ok_or_else(bad)? as f32;
    }
    Ok(Some(egui::pos2(out[0], out[1])))
}

fn modifiers(p: &Value) -> egui::Modifiers {
    let on = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(false);
    egui::Modifiers {
        shift: on("shift"),
        ctrl: on("ctrl"),
        command: on("ctrl") || on("cmd") || on("command"),
        alt: on("alt"),
        ..Default::default()
    }
}

fn pointer_button(name: Option<&str>) -> Result<egui::PointerButton, String> {
    match name {
        None | Some("left") | Some("primary") => Ok(egui::PointerButton::Primary),
        Some("right") | Some("secondary") => Ok(egui::PointerButton::Secondary),
        Some("middle") => Ok(egui::PointerButton::Middle),
        Some(other) => Err(format!(
            "`button` must be left, right or middle, not `{other}`"
        )),
    }
}

fn key_from(name: &str) -> Option<egui::Key> {
    let lower = name.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "enter" | "return" => Some(egui::Key::Enter),
        "esc" | "escape" => Some(egui::Key::Escape),
        "delete" | "del" => Some(egui::Key::Delete),
        "space" => Some(egui::Key::Space),
        "tab" => Some(egui::Key::Tab),
        _ => None,
    };
    alias.or_else(|| egui::Key::from_name(name)).or_else(|| {
        // `num1`…`num4` name the number keys the viewport shortcuts use.
        let digit = lower.strip_prefix("num")?.chars().next()?;
        egui::Key::from_name(&digit.to_string())
    })
}

fn analysis_from(name: &str) -> Option<AnalysisMode> {
    Some(match name {
        "shaded" => AnalysisMode::Shaded,
        "zebra" => AnalysisMode::Zebra { stripes: 14.0 },
        "curvature" => AnalysisMode::Curvature { radius: 1.0 },
        "draft" => AnalysisMode::Draft { angle_deg: 3.0 },
        _ => return None,
    })
}

fn button_event(
    pos: egui::Pos2,
    button: egui::PointerButton,
    pressed: bool,
    modifiers: egui::Modifiers,
) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers,
    }
}

fn key_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

fn mode_name(mode: SubdivisionMode) -> &'static str {
    match mode {
        SubdivisionMode::Loop => "loop",
        SubdivisionMode::CatmullClark => "catmull-clark",
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

// ---- the methods -------------------------------------------------------

/// Answer one request. Every arm validates its input before touching the app,
/// so a bad request leaves the window exactly as it was.
pub fn handle(app: &mut App, ctx: &egui::Context, req: &ControlRequest) -> Value {
    let empty = Value::Object(serde_json::Map::new());
    let p = if req.params.is_null() {
        &empty
    } else {
        &req.params
    };
    if !p.is_object() {
        return error("`params` must be a JSON object");
    }
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle_inner(app, ctx, req, p)
    }));
    match res {
        Ok(Ok(v)) => json!({ "ok": true, "result": v }),
        Ok(Err(e)) => error(e),
        Err(payload) => {
            let msg = panic_message(&*payload);
            error(format!("internal error: {msg}"))
        }
    }
}

fn handle_inner(
    app: &mut App,
    ctx: &egui::Context,
    req: &ControlRequest,
    p: &Value,
) -> Result<Value, String> {
    match req.method.as_str() {
        "ui.inspect" => Ok(inspect(app)),
        "ui.set" => {
            let object = p.as_object().ok_or("`params` must be a JSON object")?;
            // Read and validate everything first: a bad key must not leave a
            // half-applied state behind.
            let mut display = app.display().clone();
            let mut analysis: Option<AnalysisMode> = None;
            let mut level: Option<usize> = None;
            let mut active: Option<usize> = None;
            let mut loading_panel: Option<bool> = None;
            let mut damage_panel: Option<bool> = None;
            let mut windage_panel: Option<bool> = None;
            let mut performance_panel: Option<bool> = None;
            for (key, value) in object {
                match key.as_str() {
                    "shaded" => {
                        display.shaded = value.as_bool().ok_or("`shaded` must be true or false")?
                    }
                    "wireframe" => {
                        display.wireframe =
                            value.as_bool().ok_or("`wireframe` must be true or false")?
                    }
                    "control_net" => {
                        display.control_net = value
                            .as_bool()
                            .ok_or("`control_net` must be true or false")?
                    }
                    "points" => {
                        display.points = value.as_bool().ok_or("`points` must be true or false")?
                    }
                    "waterplane" => {
                        display.waterplane = value
                            .as_bool()
                            .ok_or("`waterplane` must be true or false")?
                    }
                    "volumes" => {
                        display.volumes =
                            value.as_bool().ok_or("`volumes` must be true or false")?
                    }
                    "windage" => {
                        display.windage =
                            value.as_bool().ok_or("`windage` must be true or false")?
                    }
                    "grid" => {
                        display.grid = value.as_bool().ok_or("`grid` must be true or false")?
                    }
                    "combs" => {
                        display.combs = value.as_bool().ok_or("`combs` must be true or false")?
                    }
                    "analysis" => {
                        let name = value.as_str().ok_or("`analysis` must be a string")?;
                        analysis = Some(analysis_from(name).ok_or_else(|| {
                            format!("`analysis` must be shaded, zebra, curvature or draft, not `{name}`")
                        })?);
                    }
                    "level" => {
                        level = Some(
                            value
                                .as_f64()
                                .filter(|x| {
                                    x.is_finite() && x.fract() == 0.0 && (0.0..=5.0).contains(x)
                                })
                                .ok_or("`level` must be a whole number from 0 to 5")?
                                as usize,
                        );
                    }
                    "active_view" => {
                        active = Some(
                            value
                                .as_f64()
                                .filter(|x| {
                                    x.is_finite() && x.fract() == 0.0 && (0.0..=3.0).contains(x)
                                })
                                .ok_or("`active_view` must be a whole number from 0 to 3")?
                                as usize,
                        );
                    }
                    "loading_panel" => {
                        loading_panel = Some(
                            value
                                .as_bool()
                                .ok_or("`loading_panel` must be true or false")?,
                        )
                    }
                    "damage_panel" => {
                        damage_panel = Some(
                            value
                                .as_bool()
                                .ok_or("`damage_panel` must be true or false")?,
                        )
                    }
                    "windage_panel" => {
                        windage_panel = Some(
                            value
                                .as_bool()
                                .ok_or("`windage_panel` must be true or false")?,
                        )
                    }
                    "performance_panel" => {
                        performance_panel = Some(
                            value
                                .as_bool()
                                .ok_or("`performance_panel` must be true or false")?,
                        )
                    }
                    other => return Err(format!("unknown setting `{other}`")),
                }
            }
            *app.display_mut() = display;
            if let Some(show) = loading_panel {
                app.set_show_loading(show);
            }
            if let Some(show) = damage_panel {
                app.set_show_damage(show);
            }
            if let Some(show) = windage_panel {
                app.set_show_windage(show);
            }
            if let Some(show) = performance_panel {
                app.set_show_performance(show);
            }
            if let Some(mode) = analysis {
                app.set_analysis(mode);
            }
            if let Some(level) = level {
                app.set_subdivision_level(level as u8);
            }
            if let Some(view) = active {
                app.set_active_view(view);
            }
            Ok(inspect(app))
        }
        "ui.view" => {
            let what = text(p, "view")?.ok_or("missing `view`")?;
            match what {
                "fit" | "reset" => app.reset_cameras(),
                "perspective" => app.set_active_view(0),
                "profile" => app.set_active_view(1),
                "plan" => app.set_active_view(2),
                "body" => app.set_active_view(3),
                other => {
                    return Err(format!(
                        "unknown view `{other}` (fit, reset, perspective, profile, plan, body)"
                    ))
                }
            }
            Ok(json!({
                "active_view": app.active_view(),
                "camera": camera_json(app, app.active_view()),
            }))
        }
        "ui.camera" => {
            let view = index_at(p, "view", 4)?.unwrap_or_else(|| app.active_view());
            let yaw = number(p, "yaw")?;
            let pitch = number(p, "pitch")?;
            let distance = number_in(p, "distance", 0.0..=MAX_CAMERA)?;
            let ortho = number_in(p, "ortho_height", 0.0..=MAX_CAMERA)?;
            if distance == Some(0.0) || ortho == Some(0.0) {
                return Err("`distance` and `ortho_height` must be greater than 0".into());
            }
            let target = match p.get("target") {
                None | Some(Value::Null) => None,
                Some(v) => Some(triple(v, "target")?),
            };
            let camera = &mut app.cameras_mut()[view];
            if let Some(yaw) = yaw {
                camera.yaw = yaw.to_radians();
            }
            if let Some(pitch) = pitch {
                // A perspective eye exactly over the target degenerates the
                // basis, so stop just short of the pole.
                camera.pitch = pitch.to_radians().clamp(-MAX_PITCH, MAX_PITCH);
            }
            if let Some(distance) = distance {
                camera.distance = distance;
            }
            if let Some(ortho) = ortho {
                camera.ortho_height = ortho;
            }
            if let Some(target) = target {
                camera.target = target;
            }
            Ok(camera_json(app, view))
        }
        "ui.move" => {
            let pos = position(p)?;
            let queued = app.queue_events([egui::Event::PointerMoved(pos)])?;
            Ok(json!({ "queued": queued, "pending": app.synthetic_len() }))
        }
        "ui.click" => {
            let pos = position(p)?;
            let button = pointer_button(text(p, "button")?)?;
            let modifiers = modifiers(p);
            let clicks = if boolean(p, "double")?.unwrap_or(false) {
                2
            } else {
                1
            };
            let mut events = vec![egui::Event::PointerMoved(pos)];
            for _ in 0..clicks {
                events.push(button_event(pos, button, true, modifiers));
                events.push(button_event(pos, button, false, modifiers));
            }
            let queued = app.queue_events(events)?;
            Ok(json!({ "queued": queued, "pending": app.synthetic_len() }))
        }
        "ui.drag" => {
            // `from`/`to` pairs, or the x0/y0/x1/y1 scalars.
            let pair = |p: &Value, key: &str, x: &str, y: &str| -> Result<egui::Pos2, String> {
                if let Some(pos) = point_pair(p, key)? {
                    return Ok(pos);
                }
                let bad = || format!("missing `{key}` (or `{x}` and `{y}`)");
                let get = |k: &str| {
                    p.get(k)
                        .and_then(Value::as_f64)
                        .filter(|v| v.is_finite() && v.abs() <= MAX_POINT)
                };
                Ok(egui::pos2(
                    get(x).ok_or_else(bad)? as f32,
                    get(y).ok_or_else(bad)? as f32,
                ))
            };
            let from = pair(p, "from", "x0", "y0")?;
            let to = pair(p, "to", "x1", "y1")?;
            let button = pointer_button(text(p, "button")?)?;
            let modifiers = modifiers(p);
            let steps = number_in(p, "steps", 1.0..=64.0)?.unwrap_or(8.0) as usize;
            let mut events = vec![
                egui::Event::PointerMoved(from),
                button_event(from, button, true, modifiers),
            ];
            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                events.push(egui::Event::PointerMoved(from + (to - from) * t));
            }
            events.push(button_event(to, button, false, modifiers));
            let queued = app.queue_events(events)?;
            Ok(json!({ "queued": queued, "pending": app.synthetic_len() }))
        }
        "ui.scroll" => {
            let lines = number_in(p, "delta", -100.0..=100.0)?.ok_or("missing `delta`")?;
            let mut events = Vec::new();
            if p.get("x").is_some() || p.get("y").is_some() {
                events.push(egui::Event::PointerMoved(position(p)?));
            }
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, lines as f32),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: modifiers(p),
            });
            let queued = app.queue_events(events)?;
            Ok(json!({ "queued": queued, "pending": app.synthetic_len() }))
        }
        "ui.key" => {
            let name = text(p, "key")?.ok_or("missing `key`")?;
            let key = key_from(name).ok_or_else(|| format!("unknown key `{name}`"))?;
            let modifiers = modifiers(p);
            let pressed = boolean(p, "pressed")?.unwrap_or(true);
            let mut events = vec![key_event(key, pressed, modifiers)];
            if pressed {
                // A tap: egui reads the release as the end of the press.
                events.push(key_event(key, false, modifiers));
            }
            let queued = app.queue_events(events)?;
            Ok(json!({ "queued": queued, "pending": app.synthetic_len() }))
        }
        "ui.render" => {
            let path = text(p, "path")?
                .filter(|s| !s.trim().is_empty())
                .ok_or("missing `path`")?;
            let validated = app.check_control_write_path(std::path::Path::new(path.trim()))?;
            let width = index_at(p, "width", MAX_RENDER + 1)?.unwrap_or(RENDER_DEFAULT[0]);
            let height = index_at(p, "height", MAX_RENDER + 1)?.unwrap_or(RENDER_DEFAULT[1]);
            if width == 0 || height == 0 {
                return Err(format!(
                    "`width` and `height` must be from 1 to {MAX_RENDER}"
                ));
            }
            let view = index_at(p, "view", 4)?.unwrap_or_else(|| app.active_view());
            let mode = match text(p, "mode")? {
                None => app.display().analysis,
                Some(name) => analysis_from(name).ok_or_else(|| {
                    format!("`mode` must be shaded, zebra, curvature or draft, not `{name}`")
                })?,
            };
            let png = render_png(app, width, height, view, mode)?;
            let bytes = png.len();
            seacraft_core::io::write_atomic(&validated, &png)
                .map_err(|e| format!("{}: {e}", validated.display()))?;
            app.status_note(format!(
                "rendered {view} to {} ({width}×{height})",
                validated.display()
            ));
            Ok(
                json!({ "path": validated.to_string_lossy(), "width": width, "height": height, "bytes": bytes }),
            )
        }
        "model.load_demo" => {
            let symmetric = boolean(p, "symmetric")?.unwrap_or(false);
            app.load_demo_shape(symmetric);
            Ok(json!({
                "symmetric": symmetric,
                "points": app.control_point_count(),
                "faces": app.face_count(),
                "layers": app.layer_count(),
            }))
        }
        "model.generate" => {
            use seacraft_core::parametric::{self, HullKind, HullSpec};
            let kind = match text(p, "kind")? {
                Some(k) => k.parse::<HullKind>()?,
                None => HullKind::Displacement,
            };
            let mut spec = HullSpec::default_for(kind);
            if let Some(v) = number(p, "lwl")? {
                spec.lwl = v;
            }
            if let Some(v) = number(p, "beam")? {
                spec.beam = v;
            }
            if let Some(v) = number(p, "draft")? {
                spec.draft = v;
            }
            if let Some(v) = number(p, "depth")? {
                spec.depth = v;
            }
            if let Some(v) = number(p, "cb")? {
                spec.cb = Some(v);
            }
            if let Some(v) = number(p, "lcb_frac")?.or(number(p, "lcb")?) {
                spec.lcb_frac = Some(v);
            }
            if let Some(v) = number(p, "deadrise_deg")?.or(number(p, "deadrise")?) {
                spec.deadrise_deg = v;
            }
            if let Some(v) = number(p, "transom_frac")?.or(number(p, "transom")?) {
                spec.transom_frac = v;
            }
            if let Some(v) = index_at(p, "stations", 66)? {
                spec.stations = v;
            }
            if let Some(v) = index_at(p, "points_per_station", 34)? {
                spec.points_per_station = v;
            }
            let (surface, rep) = parametric::generate(&spec)?;
            app.load_generated(surface);
            let mut out = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
            if let Some(obj) = out.as_object_mut() {
                obj.insert("points".into(), json!(app.control_point_count()));
                obj.insert("faces".into(), json!(app.face_count()));
                obj.insert("layers".into(), json!(app.layer_count()));
            }
            Ok(out)
        }
        "model.open" => {
            let path = text(p, "path")?
                .filter(|s| !s.trim().is_empty())
                .ok_or("missing `path`")?;
            let validated = app.check_control_read_path(std::path::Path::new(path.trim()))?;
            app.open_model(&validated)?;
            Ok(json!({
                "path": app.path().map(|p| p.to_string_lossy().to_string()),
                "points": app.control_point_count(),
                "faces": app.face_count(),
            }))
        }
        "model.save" => {
            let path = text(p, "path")?
                .filter(|s| !s.trim().is_empty())
                .ok_or("missing `path`")?;
            let path = std::path::PathBuf::from(path.trim());
            let validated = app.check_control_write_path(&path)?;
            if !app.save_to(&validated) {
                return Err(app.status().to_string());
            }
            Ok(json!({ "path": validated.to_string_lossy(), "dirty": app.is_dirty() }))
        }
        "project.get" => serde_json::to_value(app.project()).map_err(|e| e.to_string()),
        "project.set" => {
            let value = p.get("project").ok_or("missing `project`")?;
            let project = seacraft_core::project::Project::from_json(&value.to_string())?;
            app.set_project(project);
            Ok(json!({ "dirty": app.is_dirty(), "empty": app.project().is_empty() }))
        }
        "model.export" => {
            let path = text(p, "path")?
                .filter(|s| !s.trim().is_empty())
                .ok_or("missing `path`")?;
            let format = text(p, "format")?.ok_or("missing `format`")?;
            let kind = export_kind(format).ok_or_else(|| {
                format!(
                    "`format` must be obj, stl, 3mf, ply, html, svg, dxf or igs, not `{format}`"
                )
            })?;
            let path = std::path::PathBuf::from(path.trim());
            let validated = app.check_control_write_path(&path)?;
            if !app.export_to(kind, &validated) {
                return Err(app.status().to_string());
            }
            let bytes = std::fs::metadata(&validated).map(|m| m.len()).unwrap_or(0);
            Ok(json!({
                "path": validated.to_string_lossy(),
                "format": format.to_ascii_lowercase(),
                "bytes": bytes,
            }))
        }
        "point.move" => {
            if app.surface().is_none() {
                return Err("no model loaded".into());
            }
            let count = app.control_point_count();
            let index = index_at(p, "index", count.max(1))?.ok_or("missing `index`")?;
            if index >= count {
                return Err(format!(
                    "`index` must be from 0 to {}",
                    count.saturating_sub(1)
                ));
            }
            let current = app
                .surface()
                .map(|s| s.points[index].p)
                .unwrap_or(Vec3::ZERO);
            let target = match (p.get("xyz"), p.get("delta")) {
                (Some(v), None) if !v.is_null() => triple(v, "xyz")?,
                (None | Some(Value::Null), Some(v)) if !v.is_null() => {
                    current + triple(v, "delta")?
                }
                _ => return Err("give `xyz` [x, y, z] or `delta` [dx, dy, dz]".into()),
            };
            if !target.is_finite() {
                return Err("the new position must be finite".into());
            }
            // A symmetric layer models only y >= 0: the mirror supplies the
            // other half, so a point dragged across the centreline is clamped.
            let symmetric = app
                .surface()
                .map(|s| on_symmetric_layer(s, index))
                .unwrap_or(false);
            let target = if symmetric && target.y < 0.0 {
                Vec3::new(target.x, 0.0, target.z)
            } else {
                target
            };
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                surface.points[index].p = target;
            }
            app.select_point(Some(index));
            app.mark_geometry_changed();
            app.status_note(format!("control point {index} moved"));
            Ok(json!({
                "index": index,
                "point": [target.x, target.y, target.z],
                "clamped": symmetric && target.y == 0.0,
                "undo_len": app.undo_len(),
            }))
        }
        "edge.crease" => {
            let surface = app.surface().ok_or("no model loaded")?;
            let count = surface.points.len();
            let list = p
                .get("edge")
                .and_then(Value::as_array)
                .filter(|a| a.len() == 2)
                .ok_or("`edge` must be [a, b] control point indices")?;
            let mut ends = [0usize; 2];
            for (slot, value) in ends.iter_mut().zip(list) {
                *slot = value
                    .as_f64()
                    .filter(|x| x.is_finite() && x.fract() == 0.0 && *x >= 0.0 && *x < count as f64)
                    .ok_or_else(|| {
                        format!(
                            "an `edge` index must be from 0 to {}",
                            count.saturating_sub(1)
                        )
                    })? as usize;
            }
            let edge = seacraft_core::mesh::ordered(ends[0], ends[1]);
            if !surface.edges.iter().any(|e| e.key() == edge) {
                return Err(format!("no control edge between {} and {}", edge.0, edge.1));
            }
            let crease = boolean(p, "crease")?.unwrap_or(true);
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                for e in &mut surface.edges {
                    if e.key() == edge {
                        e.crease = crease;
                    }
                }
            }
            app.mark_geometry_changed();
            app.status_note(format!("edge {}-{} crease {crease}", edge.0, edge.1));
            Ok(json!({
                "edge": [edge.0, edge.1],
                "crease": crease,
                "undo_len": app.undo_len(),
            }))
        }
        "hydro.compute" => {
            apply_hydro_inputs(app, p)?;
            app.recompute_hydrostatics();
            let result = serde_json::to_value(app.hydrostatics()).unwrap_or(Value::Null);
            Ok(json!({
                "draft": app.hydro_options().draft,
                "trim": app.hydro_options().trim,
                "heel_deg": app.hydro_options().heel_deg,
                "kg": app.kg(),
                "free_trim": app.free_trim(),
                "lcg": app.lcg(),
                "result": result,
            }))
        }
        "hydro.trim" => {
            // Read-only: a scan is a report, not an edit.
            let mut opts = app.hydro_options().clone();
            if let Some(d) = number_in(p, "draft", 0.001..=1.0e3)? {
                opts.draft = d;
            }
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let subdiv = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let speed = number(p, "speed_kn")?.ok_or("hydro.trim: give `speed_kn`")?;
            let from = number_in(p, "from", -5.0..=5.0)?.unwrap_or(-0.6);
            let to = number_in(p, "to", -5.0..=5.0)?.unwrap_or(0.6);
            if to <= from {
                return Err("hydro.trim: `to` must be greater than `from`".into());
            }
            let steps = match p.get("steps") {
                None | Some(Value::Null) => 13,
                Some(_) => number_in(p, "steps", 2.0..=200.0)?
                    .map(|n| n.round() as usize)
                    .unwrap_or(13),
            };
            let hold = boolean(p, "hold_volume")?.unwrap_or(true);
            let surface = app
                .surface()
                .ok_or_else(|| "hydro.trim: no model is loaded".to_string())?;
            let trims = seacraft_core::trimopt::trim_range(from, to, steps);
            let scan = seacraft_core::trimopt::scan(surface, subdiv, &opts, speed, &trims, hold)
                .map_err(|e| format!("hydro.trim: {e}"))?;
            Ok(json!({
                "backend": "app",
                "level": level,
                "speed_kn": scan.speed_kn,
                "target_volume_m3": scan.target_volume_m3,
                "at_boundary": scan.at_boundary,
                "best": scan.best,
                "warnings": scan.warnings,
                "points": scan.points,
            }))
        }
        "hydro.table" => {
            // Read-only: a table is a report, not an edit.
            let mut opts = app.hydro_options().clone();
            if let Some(to) = number_in(p, "to", 0.001..=1.0e3)? {
                opts.draft = to;
            }
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            opts.subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let drafts = match numbers(p, "drafts")? {
                Some(list) => {
                    if list.is_empty() {
                        return Err("hydro.table: `drafts` is empty".into());
                    }
                    if list.iter().any(|d| *d <= 0.0) {
                        return Err("hydro.table: `drafts` must be positive".into());
                    }
                    list
                }
                None => {
                    let steps = match p.get("steps") {
                        None | Some(Value::Null) => 10,
                        Some(_) => number_in(p, "steps", 1.0..=200.0)?
                            .map(|n| n.round() as usize)
                            .unwrap_or(10),
                    };
                    seacraft_core::table::draft_range(opts.draft, steps)
                }
            };
            let surface = app
                .surface()
                .ok_or_else(|| "hydro.table: no model is loaded".to_string())?;
            let t = seacraft_core::table::table(surface, &opts, &drafts);
            Ok(json!({
                "backend": "app",
                "level": level,
                "name": t.name,
                "water_density": t.water_density,
                "drafts": t.rows.len(),
                "rows": serde_json::to_value(&t.rows).unwrap_or(Value::Null),
            }))
        }
        "stability.cross_curves"
            if ["heels", "displacements", "draft", "trim", "level"]
                .iter()
                .any(|k| p.get(k).is_some_and(|v| !v.is_null())) =>
        {
            explicit_cross_curves(app, p)
        }
        "stability.cross_curves" => {
            apply_stability_inputs(app, p)?;
            app.compute_cross_curves();
            let curves = app
                .cross_curves()
                .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            let longest = app
                .cross_curves()
                .map(|c| c.iter().map(|x| x.points.len()).max().unwrap_or(0))
                .unwrap_or(0);
            let imo = match app.cross_curves() {
                Some(c) if longest > 0 => serde_json::to_value(
                    seacraft_core::stability::evaluate_imo_criteria(c, 2.min(longest - 1)),
                )
                .unwrap_or(Value::Null),
                _ => Value::Null,
            };
            Ok(json!({
                "kg": app.kg(),
                "free_trim": app.free_trim(),
                "lcg": app.lcg(),
                "curves": curves,
                "imo": imo,
            }))
        }
        "stability.loading_condition" | "loading_condition" => {
            let cond: seacraft_core::loading::LoadingCondition = if let Some(case_val) =
                p.get("case").filter(|v| !v.is_null())
            {
                serde_json::from_value(case_val.clone())
                    .map_err(|e| format!("stability.loading_condition: invalid `case`: {e}"))?
            } else if let Some(path) = text(p, "case_path")?.filter(|s| !s.trim().is_empty()) {
                let validated = app.check_control_read_path(std::path::Path::new(path.trim()))?;
                let contents = std::fs::read_to_string(&validated).map_err(|e| {
                    format!(
                        "stability.loading_condition: cannot read {}: {e}",
                        validated.display()
                    )
                })?;
                serde_json::from_str(&contents).map_err(|e| {
                    format!(
                        "stability.loading_condition: invalid JSON in {}: {e}",
                        validated.display()
                    )
                })?
            } else {
                return Err("stability.loading_condition: give `case` or `case_path`".into());
            };
            seacraft_core::loading::validate(&cond)
                .map_err(|e| format!("stability.loading_condition: {e}"))?;
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let surface = app
                .surface()
                .ok_or_else(|| "stability.loading_condition: no model is loaded".to_string())?;
            let mut opts = app.hydro_options().clone();
            opts.subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let res = seacraft_core::loading::evaluate(surface, &opts, &cond);
            let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }
            Ok(v)
        }
        "loading.compute" => {
            let name = text(p, "name")?;
            let res = app.compute_loading_blocking(name)?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }
        "damage.compute" => {
            let name = text(p, "name")?;
            let res = app.compute_damage_blocking(name)?;
            serde_json::to_value(res).map_err(|e| e.to_string())
        }
        "stability.weather" | "weather_criterion" => {
            let mut opts = app.hydro_options().clone();
            if let Some(d) = number_in(p, "draft", 0.001..=1.0e3)? {
                opts.draft = d;
            }
            if let Some(t) = number_in(p, "trim", -100.0..=100.0)? {
                opts.trim = t;
            }
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            opts.subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let kg = match p.get("kg") {
                None | Some(Value::Null) => app.kg(),
                Some(_) => number_in(p, "kg", 0.0..=1.0e3)?
                    .ok_or_else(|| "stability.weather: `kg` must be >= 0".to_string())?,
            };
            let lcg = number_in(p, "lcg", -1.0e4..=1.0e4)?;
            let free = boolean(p, "free_trim")?.unwrap_or_else(|| app.free_trim()) || lcg.is_some();
            let bilge = match text(p, "bilge")? {
                None => {
                    if let Some(ak) = number_in(p, "bilge_keel_area_m2", 0.0..=1.0e4)? {
                        seacraft_core::weather::BilgeKind::Keels { area_m2: ak }
                    } else {
                        seacraft_core::weather::BilgeKind::Round
                    }
                }
                Some("keels") => {
                    let ak = number_in(p, "bilge_keel_area_m2", 0.0..=1.0e4)?.unwrap_or(0.0);
                    seacraft_core::weather::BilgeKind::Keels { area_m2: ak }
                }
                Some(s) => seacraft_core::weather::BilgeKind::parse(s)
                    .map_err(|e| format!("stability.weather: {e}"))?,
            };
            let windage_extra: Vec<seacraft_core::weather::WindageItem> =
                match p.get("windage_extra") {
                    // As a loading condition does: the project's windage.
                    None | Some(Value::Null) => app.project().windage.clone(),
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|e| format!("stability.weather: invalid `windage_extra`: {e}"))?,
                };
            for w in &windage_extra {
                if !w.area_m2.is_finite() || w.area_m2 < 0.0 || !w.z_centroid.is_finite() {
                    return Err(format!(
                        "stability.weather: windage item `{}` must have finite `area_m2 >= 0` and `z_centroid`",
                        w.name
                    ));
                }
            }
            let downflooding_deg = match p.get("downflooding_deg") {
                None | Some(Value::Null) => None,
                Some(_) => Some(number_in(p, "downflooding_deg", 0.001..=180.0)?.ok_or_else(
                    || "stability.weather: `downflooding_deg` must be in (0, 180]".to_string(),
                )?),
            };
            let wind_pressure_pa = match p.get("wind_pressure_pa") {
                None | Some(Value::Null) => seacraft_core::weather::DEFAULT_WIND_PRESSURE_PA,
                Some(_) => number_in(p, "wind_pressure_pa", 0.0..=1.0e5)?.ok_or_else(|| {
                    "stability.weather: `wind_pressure_pa` must be >= 0".to_string()
                })?,
            };
            let surface = app
                .surface()
                .ok_or_else(|| "stability.weather: no model is loaded".to_string())?;
            let hull = seacraft_core::hydro::Hull::new(surface, opts.subdivision);
            let lcg_x =
                if free {
                    Some(lcg.or_else(|| app.lcg()).unwrap_or_else(|| {
                        seacraft_core::stability::upright_lcb(&hull, opts.draft)
                    }))
                } else {
                    None
                };
            let cond = seacraft_core::stability::StabilityCondition {
                kg,
                trim: opts.trim,
                lcg_x,
            };
            let w_input = seacraft_core::weather::WeatherInput {
                windage_extra,
                bilge,
                downflooding_deg,
                wind_pressure_pa,
            };
            let res = seacraft_core::weather::evaluate_hull_weather(&hull, &opts, &cond, &w_input);
            let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }
            Ok(v)
        }
        "stability.damage_stability" | "damage_stability" => {
            let case: seacraft_core::damage::DamageCase = if let Some(case_val) =
                p.get("case").filter(|v| !v.is_null())
            {
                serde_json::from_value(case_val.clone())
                    .map_err(|e| format!("stability.damage_stability: invalid `case`: {e}"))?
            } else if let Some(path) = text(p, "case_path")?.filter(|s| !s.trim().is_empty()) {
                let validated = app.check_control_read_path(std::path::Path::new(path.trim()))?;
                let contents = std::fs::read_to_string(&validated).map_err(|e| {
                    format!(
                        "stability.damage_stability: cannot read {}: {e}",
                        validated.display()
                    )
                })?;
                serde_json::from_str(&contents).map_err(|e| {
                    format!(
                        "stability.damage_stability: invalid JSON in {}: {e}",
                        validated.display()
                    )
                })?
            } else {
                return Err("stability.damage_stability: give `case` or `case_path`".into());
            };
            seacraft_core::damage::validate(&case)
                .map_err(|e| format!("stability.damage_stability: {e}"))?;
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let surface = app
                .surface()
                .ok_or_else(|| "stability.damage_stability: no model is loaded".to_string())?;
            let mut opts = app.hydro_options().clone();
            opts.subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let res = seacraft_core::damage::evaluate(surface, &opts, &case)
                .map_err(|e| format!("stability.damage_stability: {e}"))?;
            let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }
            Ok(v)
        }
        "seakeeping.compute" | "seakeeping_raos" => {
            let surface = app
                .surface()
                .ok_or_else(|| "seakeeping.compute: no model is loaded".to_string())?;
            let draft =
                match number_in(p, "draft", 0.001..=1.0e4)? {
                    Some(d) => d,
                    None if surface.settings.draft > 0.0 => surface.settings.draft,
                    None => return Err(
                        "seakeeping.compute: draft must be specified or set in project settings"
                            .into(),
                    ),
                };
            let speed_kn = number(p, "speed_kn")?
                .or(number(p, "speed")?)
                .unwrap_or(0.0);
            let kg = number(p, "kg")?.unwrap_or_else(|| app.kg());
            let kyy = number(p, "kyy")?;
            let stations = number_in(p, "stations", 3.0..=200.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(21);
            let lambda_min = number(p, "lambda_min")?;
            let lambda_max = number(p, "lambda_max")?;
            let points = number_in(p, "points", 1.0..=500.0)?.map(|n| n.round() as usize);
            let transom_terms = boolean(p, "transom_terms")?.unwrap_or(true);

            let spec = seacraft_core::seakeeping::SeakeepingSpec {
                draft,
                speed_kn,
                kg,
                kyy,
                stations,
                lambda_min,
                lambda_max,
                points,
                transom_terms,
                ..Default::default()
            };

            let rep = seacraft_core::seakeeping::head_sea_raos(surface, &spec)
                .map_err(|e| format!("seakeeping.compute: {e}"))?;

            let mut v = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }
            Ok(v)
        }
        "seakeeping.irregular" | "seakeeping_irregular" => {
            let surface = app
                .surface()
                .ok_or_else(|| "seakeeping.irregular: no model is loaded".to_string())?;
            let spec =
                seacraft_core::seastate::IrregularSpec::from_params(p, surface.settings.draft)
                    .map_err(|e| format!("seakeeping.irregular: {e}"))?;
            let rep = seacraft_core::seastate::irregular_head_sea(surface, &spec)
                .map_err(|e| format!("seakeeping.irregular: {e}"))?;
            let mut v = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
            if let Some(o) = v.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }
            Ok(v)
        }
        "model.split_face" => {
            // A topology edit: snapshot first, so Ctrl+Z in the window undoes it
            // exactly like a drag does.
            let faces = app.surface().map(|s| s.faces.len()).unwrap_or(0);
            let face = index_at(p, "face", faces)?.ok_or("model.split_face: give `face`")?;
            // Edit a copy: a refused split must leave no undo entry behind.
            let mut edited = app.surface().ok_or("no model loaded")?.clone();
            let report = seacraft_core::topology::split_face(&mut edited, face)
                .map_err(|e| format!("model.split_face: {e}"))?;
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                *surface = edited;
            }
            app.mark_geometry_changed();
            app.status_note(format!(
                "face {face} split into {} triangles",
                report.pieces
            ));
            Ok(json!({
                "face": report.face,
                "point": report.point,
                "pieces": report.pieces,
                "faces": report.faces_after,
                "points": report.points_after,
                "undo_len": app.undo_len(),
            }))
        }
        "model.insert_edge" => {
            let faces_total = app.surface().map(|s| s.faces.len()).unwrap_or(0);
            let f = index_at(p, "face", faces_total)?.ok_or("model.insert_edge: give `face`")?;
            let count = app.surface().map(|s| s.points.len()).unwrap_or(0);
            let a = index_at(p, "a", count)?.ok_or("model.insert_edge: give `a`")?;
            let b = index_at(p, "b", count)?.ok_or("model.insert_edge: give `b`")?;
            let mut edited = app.surface().ok_or("no model loaded")?.clone();
            seacraft_core::topology::insert_edge(&mut edited, f, a, b)
                .map_err(|e| format!("model.insert_edge: {e}"))?;
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                *surface = edited;
            }
            app.mark_geometry_changed();
            app.status_note(format!("edge {a}-{b} inserted on face {f}"));
            let faces = app.surface().map(|s| s.faces.len()).unwrap_or(0);
            Ok(json!({
                "face": f,
                "a": a,
                "b": b,
                "faces": faces,
                "undo_len": app.undo_len(),
            }))
        }
        "curves.bonjean" => {
            // Read-only measurement of the app's own model, so an agent can
            // draw the curves the way `seacraft bonjean` does. Nothing is
            // changed: no snapshot, no dirty flag.
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let stations = number_in(p, "stations", 2.0..=200.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(21);
            let steps = number_in(p, "draft_steps", 2.0..=50.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(10);
            let surface = app
                .surface()
                .ok_or_else(|| "curves.bonjean: no model is loaded".to_string())?;
            let hull = seacraft_core::hydro::Hull::new(
                surface,
                seacraft_core::subdiv::SubdivisionOptions {
                    level,
                    mode: app.subdivision_mode(),
                },
            );
            let depth = hull.model_max.z - hull.keel;
            if depth <= 0.0 {
                return Err("curves.bonjean: the hull has no height above the keel".into());
            }
            let drafts = match numbers(p, "drafts")? {
                Some(mut given) => {
                    if given.len() < 2 {
                        return Err("curves.bonjean: `drafts` needs at least two ordinates".into());
                    }
                    if given.iter().any(|d| *d <= 0.0) {
                        return Err("curves.bonjean: `drafts` must be positive".into());
                    }
                    given.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    given
                }
                None => seacraft_core::curves::draft_range(depth / steps as f64, depth, steps),
            };
            let bj = seacraft_core::curves::bonjean(&hull, stations, &drafts);
            Ok(json!({
                "backend": "app",
                "level": level,
                "depth_m": depth,
                "design_draft_m": app.hydro_options().draft,
                "drafts_m": bj.drafts_m,
                "sections": bj.sections,
                "area_max_m2": bj.max_area(),
            }))
        }
        "curves.offsets" => {
            // Read-only, like curves.bonjean: no snapshot, no dirty flag.
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let stations = number_in(p, "stations", 2.0..=200.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(21);
            let wl_steps = number_in(p, "waterline_steps", 1.0..=50.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(6);
            let b_steps = number_in(p, "buttock_steps", 1.0..=50.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(4);
            let surface = app
                .surface()
                .ok_or_else(|| "curves.offsets: no model is loaded".to_string())?;
            let hull = seacraft_core::hydro::Hull::new(
                surface,
                seacraft_core::subdiv::SubdivisionOptions {
                    level,
                    mode: app.subdivision_mode(),
                },
            );
            let waterlines = match numbers(p, "waterlines")? {
                Some(v) => positive_ordinates(v, "waterlines")?,
                None => seacraft_core::offsets::default_waterlines(&hull, wl_steps),
            };
            let buttocks = match numbers(p, "buttocks")? {
                Some(v) => positive_ordinates(v, "buttocks")?,
                None => seacraft_core::offsets::default_buttocks(&hull, b_steps),
            };
            let table =
                seacraft_core::offsets::table_of_offsets(&hull, stations, &waterlines, &buttocks);
            Ok(json!({
                "backend": "app",
                "level": level,
                "keel_z": table.keel_z,
                "waterlines_m": table.waterlines_m,
                "buttocks_m": table.buttocks_m,
                "stations": table.stations,
                "max_half_breadth_m": table.max_half_breadth(),
            }))
        }
        "performance.compute" => {
            let surface = app
                .surface()
                .ok_or_else(|| "performance.compute: no model is loaded".to_string())?;
            let mut opts = app.hydro_options().clone();
            if let Some(d) = number_in(p, "draft", 0.001..=1.0e4)? {
                opts.draft = d;
            }
            opts.subdivision.level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=5.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            opts.subdivision.mode = app.subdivision_mode();

            let method = match text(p, "method")? {
                Some(m) if m.eq_ignore_ascii_case("dsyhs") => crate::app::ResistanceMethod::Dsyhs,
                Some(m)
                    if m.eq_ignore_ascii_case("savitsky") || m.eq_ignore_ascii_case("planing") =>
                {
                    crate::app::ResistanceMethod::Savitsky
                }
                Some(m) if m.eq_ignore_ascii_case("michell") => {
                    crate::app::ResistanceMethod::Michell
                }
                Some(m) if m.eq_ignore_ascii_case("holtrop") => {
                    crate::app::ResistanceMethod::Holtrop
                }
                Some(other) => return Err(format!("unknown resistance method `{other}`")),
                None => app.performance_state().method,
            };

            let speeds = match numbers(p, "speeds")? {
                Some(v) if !v.is_empty() => v,
                _ => app.performance_speeds(),
            };

            let resistance = app.compute_resistance_for(method, &speeds, &opts)?;

            let speed_kn = number(p, "speed_kn")?
                .or(number(p, "speed")?)
                .unwrap_or(app.performance_state().sk_speed_kn);
            let kg = match number(p, "kg")? {
                Some(k) => k,
                None => app.effective_performance_kg(),
            };
            let kyy = number(p, "kyy")?;
            let stations = number_in(p, "stations", 3.0..=200.0)?
                .map(|n| n.round() as usize)
                .unwrap_or(21);
            let lambda_min = number(p, "lambda_min")?;
            let lambda_max = number(p, "lambda_max")?;
            let points = number_in(p, "points", 1.0..=500.0)?
                .map(|n| n.round() as usize)
                .or(Some(app.performance_state().points));

            let spec = seacraft_core::seakeeping::SeakeepingSpec {
                draft: opts.draft,
                speed_kn,
                kg,
                kyy,
                stations,
                lambda_min,
                lambda_max,
                points,
                ..Default::default()
            };

            let rep = seacraft_core::seakeeping::head_sea_raos(surface, &spec)
                .map_err(|e| format!("performance.compute: {e}"))?;

            app.set_performance_results(resistance.clone(), rep.clone());

            let resistance_json = json!({
                "method": resistance.method,
                "rows": resistance.rows.iter().map(|r| json!({
                    "speed_kn": r.speed_kn,
                    "fn_": r.fn_,
                    "rt_kn": r.rt_kn,
                    "pe_kw": r.pe_kw,
                    "note": r.note,
                })).collect::<Vec<_>>(),
                "warnings": resistance.warnings,
                "rt_label": resistance.rt_label,
            });

            let mut rep_json = serde_json::to_value(&rep).map_err(|e| e.to_string())?;
            if let Some(o) = rep_json.as_object_mut() {
                o.insert("backend".into(), json!("app"));
            }

            Ok(json!({
                "backend": "app",
                "resistance": resistance_json,
                "raos": rep_json,
            }))
        }
        "resistance.compute" => {
            // Read-only: the resistance model measures, it does not edit.
            let mut opts = app.hydro_options().clone();
            if let Some(d) = number_in(p, "draft", 0.01..=1.0e4)? {
                opts.draft = d;
            }
            if let Some(level) = number_in(p, "level", 0.0..=6.0)? {
                opts.subdivision.level = level as u8;
            }
            let surface = app
                .surface()
                .ok_or_else(|| "resistance.compute: no model is loaded".to_string())?;
            let hull = seacraft_core::hydro::Hull::new(surface, opts.subdivision);
            let speeds = match numbers(p, "speeds")? {
                Some(v) if !v.is_empty() => v,
                _ => vec![6.0, 8.0, 10.0, 12.0, 14.0],
            };
            let method = text(p, "method")?.unwrap_or("holtrop");
            let cat_spacing = number(p, "cat_spacing")?;
            if method.eq_ignore_ascii_case("michell") {
                let grid = seacraft_core::michell::michell_grid_from_surface(
                    surface,
                    opts.draft,
                    65,
                    25,
                    opts.subdivision,
                )
                .ok_or(
                    "resistance.compute: cannot extract Michell offsets grid from hull surface",
                )?;
                let holtrop_in = seacraft_core::resistance::HoltropInput::measure(&hull, &opts);
                let form_factor = if let Some(ref h_in) = holtrop_in {
                    let r = seacraft_core::resistance::holtrop(h_in, 1.0);
                    if h_in.warnings().is_empty() {
                        r.form_factor
                    } else {
                        1.0
                    }
                } else {
                    1.0
                };
                let viscosity = if (opts.water_density - 1000.0).abs() < 25.0 {
                    seacraft_core::resistance::VISCOSITY_FRESH_WATER
                } else {
                    seacraft_core::resistance::VISCOSITY_SEA_WATER
                };
                let mopts = seacraft_core::michell::MichellOptions {
                    nx: 65,
                    nz: 25,
                    ntheta: 128,
                    catamaran_spacing: cat_spacing,
                };
                let rows = seacraft_core::michell::michell_resistance_curve(
                    &grid,
                    form_factor,
                    viscosity,
                    opts.water_density,
                    seacraft_core::resistance::GRAVITY,
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
                    "backend": "app",
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

            if method.eq_ignore_ascii_case("dsyhs") {
                let dsyhs_in = seacraft_core::resistance::DsyhsInput::measure(&hull, &opts)
                    .ok_or("resistance.compute: cannot measure the hull for DSYHS")?;
                let mut warnings = dsyhs_in.warnings();
                let rows = seacraft_core::resistance::dsyhs_curve(&dsyhs_in, &speeds);
                for r in &rows {
                    if !(0.15..=0.75).contains(&r.fn_) {
                        warnings.push(format!(
                            "Fn {:.3} at {:.2} kn is outside DSYHS valid range (0.15-0.75, clamped)",
                            r.fn_, r.speed_kn
                        ));
                    }
                }
                return Ok(json!({
                    "backend": "app",
                    "method": "dsyhs",
                    "input": dsyhs_in,
                    "warnings": warnings,
                    "speeds": rows,
                }));
            }

            let mut input = seacraft_core::resistance::HoltropInput::measure(&hull, &opts)
                .ok_or("resistance.compute: cannot measure the hull")?;
            input.c_stern = number(p, "cstern")?.unwrap_or(0.0);
            input.bulb_area_m2 = number(p, "bulb_area")?.unwrap_or(0.0).max(0.0);
            input.bulb_centre_z = number(p, "bulb_z")?.unwrap_or(0.0).max(0.0);
            let rows = seacraft_core::resistance::curve(&input, &speeds);
            let diameter = number(p, "propeller_diameter")?.unwrap_or(0.0).max(0.0);
            let propulsion: Vec<seacraft_core::resistance::Propulsion> = if diameter > 0.0 {
                let propeller = seacraft_core::resistance::Propeller {
                    diameter_m: diameter,
                    expanded_area_ratio: number(p, "expanded_area")?.unwrap_or(0.70),
                    open_water_efficiency: number(p, "eta_o")?.unwrap_or(0.60),
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
                "backend": "app",
                "input": input,
                "warnings": input.warnings(),
                "speeds": rows,
                "propulsion": propulsion,
            }))
        }
        "planing.compute" => {
            let mut opts = app.hydro_options().clone();
            if let Some(d) = number_in(p, "draft", 0.01..=1.0e4)? {
                opts.draft = d;
            }
            if let Some(level) = number_in(p, "level", 0.0..=6.0)? {
                opts.subdivision.level = level as u8;
            }
            let surface = app
                .surface()
                .ok_or_else(|| "planing.compute: no model is loaded".to_string())?;
            let setup = seacraft_core::planing::setup(
                surface,
                opts.subdivision,
                &opts,
                number(p, "beam")?,
                number(p, "lcg_x")?,
                number(p, "deadrise")?,
            )
            .map_err(|e| format!("planing.compute: {e}"))?;
            let speeds = match numbers(p, "speeds")? {
                Some(v) if !v.is_empty() => v,
                _ => vec![20.0, 25.0, 30.0],
            };
            let rows = seacraft_core::planing::planing_curve(&setup.hull, &speeds);
            if rows.is_empty() {
                return Err("planing.compute: no planing equilibrium at those speeds".into());
            }
            Ok(json!({ "backend": "app", "setup": setup, "speeds": rows }))
        }
        "model.transform" => {
            let subdiv = seacraft_core::subdiv::SubdivisionOptions {
                level: app.subdivision_level(),
                mode: app.subdivision_mode(),
            };
            let mirror = match text(p, "mirror")? {
                None => None,
                Some(name) => Some(
                    seacraft_core::transform::Axis::parse(name)
                        .ok_or_else(|| format!("`mirror` must be x, y or z (got `{name}`)"))?,
                ),
            };
            let vec3 = |key: &str| -> Result<Option<Vec3>, String> {
                match p.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(v) => Ok(Some(triple(v, key)?)),
                }
            };
            let request = seacraft_core::transform::TransformRequest {
                lwl: number_in(p, "lwl", 0.01..=1.0e4)?,
                bwl: number_in(p, "bwl", 0.01..=1.0e4)?,
                draft: number_in(p, "draft", 0.001..=1.0e3)?,
                scale: vec3("scale")?,
                translate: vec3("translate")?.unwrap_or(Vec3::ZERO),
                mirror,
            };
            let surface = app
                .surface()
                .ok_or_else(|| "model.transform: no model is loaded".to_string())?;
            let (out, report) = seacraft_core::transform::run(surface, &request, subdiv)
                .map_err(|e| format!("model.transform: {e}"))?;
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                *surface = out;
            }
            app.select_point(None);
            // The fit rescales the model's own design draft; the app measures at
            // its hydro option, so carry the new draft across or every number
            // after this would be read at the old waterline.
            let new_draft = app.surface().map(|s| s.settings.draft).unwrap_or(0.0);
            if new_draft > 0.0 {
                app.hydro_options_mut().draft = new_draft;
            }
            app.mark_geometry_changed();
            app.status_note(format!(
                "hull transformed: scale {:.3} x {:.3} x {:.3}",
                report.scale.x, report.scale.y, report.scale.z
            ));
            Ok(json!({
                "report": report,
                "faces": app.face_count(),
                "points": app.control_point_count(),
                "draft": app.hydro_options().draft,
                "undo_len": app.undo_len(),
            }))
        }
        "model.optimize" => {
            // Reshape the hull for lower resistance at a speed. It is an edit:
            // one undo snapshot, the whole optimised hull replaces the model.
            let params = p.as_object().cloned().unwrap_or_default();
            let spec = seacraft_core::optimize::OptimizeSpec::from_params(&params, Some(app.kg()))
                .map_err(|e| format!("model.optimize: {e}"))?;
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let mut opts = app.hydro_options().clone();
            opts.subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level,
                mode: app.subdivision_mode(),
            };
            let surface = app
                .surface()
                .ok_or_else(|| "model.optimize: no model is loaded".to_string())?;
            let (out, report) = seacraft_core::optimize::optimize_hull(surface, &opts, &spec)
                .map_err(|e| format!("model.optimize: {e}"))?;
            app.snapshot_undo();
            if let Some(surface) = app.surface_mut() {
                *surface = out;
            }
            app.select_point(None);
            // The optimised hull is measured at the draft that holds the
            // original displacement; the app must read it there too.
            app.hydro_options_mut().draft = report.optimised.draft_m;
            app.mark_geometry_changed();
            app.status_note(format!(
                "hull optimised at {:.1} kn: RT {:+.1} %",
                report.speed_kn, report.rt_change_pct
            ));
            Ok(json!({
                "backend": "app",
                "level": level,
                "kg": spec.kg,
                "report": report,
                "faces": app.face_count(),
                "points": app.control_point_count(),
                "draft": app.hydro_options().draft,
                "undo_len": app.undo_len(),
            }))
        }
        "curves.unfold" => {
            // Read-only plate development: no snapshot, no dirty flag.
            let axis = match text(p, "axis")? {
                None => seacraft_core::develop::BandAxis::Z,
                Some(name) => seacraft_core::develop::BandAxis::parse(name)
                    .ok_or_else(|| format!("`axis` must be x, y or z (got `{name}`)"))?,
            };
            let level = match p.get("level") {
                None | Some(Value::Null) => app.subdivision_level(),
                Some(_) => number_in(p, "level", 0.0..=6.0)?
                    .map(|l| l.round() as u8)
                    .unwrap_or_else(|| app.subdivision_level()),
            };
            let lo = number(p, "from")?;
            let hi = number(p, "to")?;
            let x_lo = number(p, "x_from")?;
            let x_hi = number(p, "x_to")?;
            let both = boolean(p, "both_sides")?.unwrap_or(false);
            let surface = app
                .surface()
                .ok_or_else(|| "curves.unfold: no model is loaded".to_string())?;
            let hull = seacraft_core::hydro::Hull::new(
                surface,
                seacraft_core::subdiv::SubdivisionOptions {
                    level,
                    mode: app.subdivision_mode(),
                },
            );
            let pick = |axis: seacraft_core::develop::BandAxis, given: Option<f64>, low: bool| {
                given.unwrap_or(match (axis, low) {
                    (seacraft_core::develop::BandAxis::X, true) => hull.model_min.x,
                    (seacraft_core::develop::BandAxis::X, false) => hull.model_max.x,
                    (seacraft_core::develop::BandAxis::Y, true) => hull.model_min.y,
                    (seacraft_core::develop::BandAxis::Y, false) => hull.model_max.y,
                    (seacraft_core::develop::BandAxis::Z, true) => hull.keel,
                    (seacraft_core::develop::BandAxis::Z, false) => hull.model_max.z,
                })
            };
            let selection = seacraft_core::develop::PatchSelection {
                slabs: vec![
                    (axis, pick(axis, lo, true), pick(axis, hi, false)),
                    (
                        seacraft_core::develop::BandAxis::X,
                        pick(seacraft_core::develop::BandAxis::X, x_lo, true),
                        pick(seacraft_core::develop::BandAxis::X, x_hi, false),
                    ),
                ],
                starboard_only: !both,
            };
            let faces = seacraft_core::develop::select(&hull.verts, &hull.tris, &selection);
            if faces.is_empty() {
                return Err("curves.unfold: the band selected no faces; widen the range".into());
            }
            let unfolded = seacraft_core::develop::unfold(&hull.verts, &faces);
            let mut written = None;
            if let Some(path) = text(p, "path")?.filter(|s| !s.trim().is_empty()) {
                let validated = app.check_control_write_path(std::path::Path::new(path.trim()))?;
                let title = format!(
                    "{} - plate on {:?} {:.2}..{:.2} m",
                    surface.settings.name, axis, selection.slabs[0].1, selection.slabs[0].2
                );
                seacraft_core::io::write_atomic(
                    &validated,
                    seacraft_core::develop::svg(&unfolded, &title),
                )
                .map_err(|e| format!("curves.unfold: cannot write {}: {e}", validated.display()))?;
                written = Some(validated.to_string_lossy().to_string());
            }
            Ok(json!({
                "backend": "app",
                "level": level,
                "faces": faces.len(),
                "plate": unfolded,
                "written": written,
            }))
        }
        "report.design" => {
            let path_str = text(p, "path")?
                .filter(|s| !s.trim().is_empty())
                .ok_or("report.design: `path` is required")?;
            let path = std::path::PathBuf::from(path_str.trim());
            let validated = app.check_control_write_path(&path)?;
            let surface = app
                .surface()
                .ok_or_else(|| "report.design: no model loaded".to_string())?;

            let project = app.project();
            let mut opts = seacraft_core::report::ReportOptions::from_surface(surface);
            opts = project.report.apply(opts);
            if let Some(d) = number_in(p, "draft", 0.001..=1.0e3)? {
                opts.draft = d;
            }
            if let Some(kg) = number_in(p, "kg", 0.0..=1.0e3)? {
                opts.kg = kg;
            }
            if let Some(lcg) = number_in(p, "lcg", -1.0e6..=1.0e6)? {
                opts.lcg_x = Some(lcg);
            }
            if let Some(speeds) = numbers(p, "speeds_kn")?.or(numbers(p, "speeds")?) {
                if !speeds.is_empty() {
                    opts.speeds_kn = speeds;
                }
            }
            if let Some(nr) = boolean(p, "include_resistance")? {
                opts.include_resistance = nr;
            }
            if let Some(pl) = boolean(p, "include_planing")? {
                opts.include_planing = pl;
            }
            if let Some(lvl) = number_in(p, "level", 0.0..=6.0)? {
                opts.level = lvl.round() as u8;
            } else {
                opts.level = app.subdivision_level();
            }
            opts.mode = app.subdivision_mode();

            let html = seacraft_core::report::design_report_with_project(surface, &opts, project)
                .map_err(|e| format!("report.design: {e}"))?;
            seacraft_core::io::write_atomic(&validated, &html)
                .map_err(|e| format!("report.design: cannot write {}: {e}", validated.display()))?;

            let sections = seacraft_core::report::report_sections_with_project(&opts, project);

            let hull = seacraft_core::hydro::Hull::new(
                surface,
                seacraft_core::subdiv::SubdivisionOptions {
                    level: opts.level,
                    mode: opts.mode,
                },
            );
            let h_opts = seacraft_core::hydro::HydroOptions::from_surface(
                surface,
                seacraft_core::subdiv::SubdivisionOptions {
                    level: opts.level,
                    mode: opts.mode,
                },
            );
            let cond = seacraft_core::stability::StabilityCondition {
                kg: opts.kg,
                trim: 0.0,
                lcg_x: opts.lcg_x.or(Some(seacraft_core::stability::upright_lcb(
                    &hull, opts.draft,
                ))),
            };
            let target_disp =
                hull.displaced_volume(opts.draft, 0.0, 0.0) * h_opts.water_density / 1000.0;
            let curves = seacraft_core::stability::cross_curves(
                &hull,
                &opts.heels,
                &[target_disp],
                &cond,
                &h_opts,
            );
            let imo_pass = seacraft_core::stability::evaluate_imo_criteria(&curves, 0)
                .map(|c| c.passed)
                .unwrap_or(false);

            Ok(json!({
                "path": validated.to_string_lossy(),
                "bytes": html.len(),
                "sections": sections,
                "imo_pass": imo_pass,
                "imo_passed": imo_pass,
            }))
        }
        "history.mark" => {
            let mark = app.mark();
            Ok(json!({ "mark": mark }))
        }
        "history.restore" => {
            let mark = u64_arg(p, "mark")?.ok_or("missing `mark`")?;
            app.restore_mark(mark)?;
            Ok(json!({ "mark": mark, "restored": true }))
        }
        "history.forget" => {
            if let Some(mark) = u64_arg(p, "mark")? {
                app.forget_mark(mark);
            }
            Ok(json!({ "forgotten": true }))
        }
        "history.undo" => {
            app.undo();
            Ok(
                json!({ "undo_len": app.undo_len(), "can_undo": app.can_undo(), "can_redo": app.can_redo() }),
            )
        }
        "history.redo" => {
            app.redo();
            Ok(
                json!({ "undo_len": app.undo_len(), "can_undo": app.can_undo(), "can_redo": app.can_redo() }),
            )
        }
        #[cfg(test)]
        "test.panic" => panic!("deliberate control panic"),
        "app.quit" => {
            app.request_quit(ctx);
            Ok(json!({ "quitting": true }))
        }
        other => Err(format!("unknown method `{other}`")),
    }
}

/// Everything `ui.inspect` reports.
fn inspect(app: &App) -> Value {
    let d = app.display();
    let h = app.hydro_options();
    json!({
        "version": seacraft_core::VERSION,
        "status": app.status(),
        "dirty": app.is_dirty(),
        "path": app.path().map(|p| p.to_string_lossy().to_string()),
        "level": app.subdivision_level(),
        "mode": mode_name(app.subdivision_mode()),
        "active_view": app.active_view(),
        "selected_point": app.selected(),
        "selected_volume": match app.selected_volume_info() {
            Some((kind, index, name)) => json!({
                "kind": kind,
                "index": index,
                "name": name,
            }),
            None => Value::Null,
        },
        "selected_windage": match app.selected_windage_info() {
            Some((index, name)) => json!({
                "index": index,
                "name": name,
            }),
            None => Value::Null,
        },
        "undo_len": app.undo_len(),
        "points": app.control_point_count(),
        "faces": app.face_count(),
        "layers": app.layer_count(),
        "display": {
            "shaded": d.shaded,
            "wireframe": d.wireframe,
            "control_net": d.control_net,
            "points": d.points,
            "waterplane": d.waterplane,
            "volumes": d.volumes,
            "windage": d.windage,
            "grid": d.grid,
            "combs": d.combs,
            "analysis": d.analysis.label(),
        },
        "hydro": {
            "draft": h.draft,
            "trim": h.trim,
            "heel_deg": h.heel_deg,
            "kg": app.kg(),
            "free_trim": app.free_trim(),
            "lcg": app.lcg(),
            "result": serde_json::to_value(app.hydrostatics()).unwrap_or(Value::Null),
        },
    })
}

fn camera_json(app: &App, view: usize) -> Value {
    let camera = &app.cameras()[view];
    json!({
        "view": view,
        "kind": camera.kind.label(),
        "yaw_deg": camera.yaw.to_degrees(),
        "pitch_deg": camera.pitch.to_degrees(),
        "distance": camera.distance,
        "ortho_height": camera.ortho_height,
        "target": [camera.target.x, camera.target.y, camera.target.z],
    })
}

/// Update the floating condition from `draft`, `trim` and `heel_deg`, all
/// checked before any of them is applied.
fn apply_hydro_inputs(app: &mut App, p: &Value) -> Result<(), String> {
    let draft = number_in(p, "draft", -1_000.0..=1_000.0)?;
    let trim = number_in(p, "trim", -1_000.0..=1_000.0)?;
    let heel = number_in(p, "heel_deg", -180.0..=180.0)?;
    let kg = number_in(p, "kg", 0.0..=10_000.0)?;
    let opts = app.hydro_options_mut();
    if let Some(v) = draft {
        opts.draft = v;
    }
    if let Some(v) = trim {
        opts.trim = v;
    }
    if let Some(v) = heel {
        opts.heel_deg = v;
    }
    if let Some(v) = kg {
        app.set_kg(v);
    }
    apply_stability_inputs(app, p)
}

/// Update `kg`, `free_trim` and `lcg` (the GZ solve's condition).
fn apply_stability_inputs(app: &mut App, p: &Value) -> Result<(), String> {
    let kg = number_in(p, "kg", 0.0..=10_000.0)?;
    let free = boolean(p, "free_trim")?;
    let lcg = match p.get("lcg") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => Some(Some(
            v.as_f64()
                .filter(|x| x.is_finite() && x.abs() <= MAX_CAMERA)
                .ok_or("`lcg` must be a finite number or null")?,
        )),
    };
    if let Some(kg) = kg {
        app.set_kg(kg);
    }
    if free.is_some() || lcg.is_some() {
        let free = free.unwrap_or_else(|| app.free_trim());
        let lcg = match lcg {
            Some(lcg) => lcg,
            None => app.lcg(),
        };
        app.set_free_trim(free, lcg);
    }
    Ok(())
}

/// `stability.cross_curves` with an explicit grid or condition: computed on
/// the current model exactly as the headless MCP `stability_cross_curves`
/// tool does (same defaults, same reply shape), without touching the dock.
fn explicit_cross_curves(app: &mut App, p: &Value) -> Result<Value, String> {
    use seacraft_core::stability::{
        cross_curves, default_displacements, evaluate_imo_criteria, upright_lcb, StabilityCondition,
    };
    let surface = app.surface().ok_or("no model loaded")?.clone();
    let mut base = app.hydro_options().clone();
    if let Some(d) = number(p, "draft")? {
        base.draft = d;
    }
    if let Some(t) = number(p, "trim")? {
        base.trim = t;
    }
    if let Some(level) = number_in(p, "level", 0.0..=6.0)? {
        base.subdivision.level = level as u8;
    }
    if !(base.draft.is_finite() && base.draft > 0.0) {
        return Err("`draft` must be positive".into());
    }
    let kg = number_in(p, "kg", 0.0..=10_000.0)?.unwrap_or_else(|| app.kg());
    let lcg = number(p, "lcg")?;
    let free = boolean(p, "free_trim")?.unwrap_or(false) || lcg.is_some();
    let hull = seacraft_core::hydro::Hull::new(&surface, base.subdivision);
    let heels = numbers(p, "heels")?
        .unwrap_or_else(|| vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0]);
    if heels.is_empty() || heels.len() > 200 {
        return Err("`heels` must have 1 to 200 angles".into());
    }
    let displacements = match numbers(p, "displacements")? {
        Some(d) => d,
        None => default_displacements(
            hull.displaced_volume(base.draft, base.trim, 0.0) * base.water_density / 1000.0,
        ),
    };
    if displacements.is_empty()
        || displacements.len() > 50
        || displacements.iter().any(|d| *d <= 0.0)
    {
        return Err("`displacements` must be 1 to 50 positive tonnes".into());
    }
    let lcg_x = free.then(|| lcg.unwrap_or_else(|| upright_lcb(&hull, base.draft)));
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

/// A control point's layer: symmetric layers model only `y >= 0`.
fn on_symmetric_layer(surface: &seacraft_core::Surface, point: usize) -> bool {
    surface.faces.iter().enumerate().any(|(fi, f)| {
        f.contains(&point)
            && surface
                .face_layer
                .get(fi)
                .and_then(|&li| surface.layers.get(li))
                .is_some_and(|l| l.symmetric)
    })
}

fn export_kind(format: &str) -> Option<crate::app::ExportKind> {
    use crate::app::ExportKind;
    Some(match format.to_ascii_lowercase().as_str() {
        "obj" => ExportKind::Obj,
        "stl" => ExportKind::Stl,
        "3mf" => ExportKind::ThreeMf,
        "ply" => ExportKind::Ply,
        "igs_surface" | "iges_surface" => ExportKind::HullIges,
        "html" => ExportKind::Html,
        "svg" => ExportKind::Linesplan,
        "dxf" => ExportKind::LinesplanDxf,
        "igs" | "iges" => ExportKind::LinesplanIges,
        _ => return None,
    })
}

/// Render one viewport to PNG pixels with the CPU rasterizer.
fn render_png(
    app: &mut App,
    width: usize,
    height: usize,
    view: usize,
    mode: AnalysisMode,
) -> Result<Vec<u8>, String> {
    app.refresh_cache();
    let surface = app.surface().ok_or("no model loaded")?;
    let mesh = app.render_cache().ok_or("the model could not be refined")?;
    let (min, max) = surface.extents();
    let (draft, trim, heel_deg) = {
        let options = app.hydro_options();
        (options.draft, options.trim, options.heel_deg)
    };
    let scene = Scene {
        surface,
        mesh,
        curves: None,
        volumes: &[],
        windage: &[],
        min,
        max,
        draft,
        trim,
        heel_deg,
    };
    let transform = app.cameras()[view].transform();
    let waterplane = app.display().waterplane;
    let (w, h, rgba) = render::raster_frame(&transform, &scene, mode, waterplane, width, height);
    seacraft_core::raster::png(w, h, &rgba).ok_or_else(|| "could not encode the PNG".to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Write};

    use super::*;

    /// A demo model at a cheap subdivision level: these tests drive the
    /// protocol, not the refinement.
    fn demo() -> (App, egui::Context) {
        let mut app = App::headless_for_test(None);
        app.set_subdivision_level(2);
        (app, egui::Context::default())
    }

    fn call(app: &mut App, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (tx, _rx) = mpsc::channel();
        handle(app, ctx, &ControlRequest::new(method, params, tx))
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "seacraft-control-{name}-{}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn project_get_and_set_over_control() {
        let (mut app, ctx) = demo();
        let r = call(&mut app, &ctx, "project.get", json!({}));
        assert_eq!(r["result"]["format"], "seacraft-project", "{r}");
        let windage = json!([{ "name": "Deckhouse", "area_m2": 3.5, "z_centroid": 2.4 }]);
        let project = json!({ "format": "seacraft-project", "version": 1, "windage": windage });
        let r = call(&mut app, &ctx, "project.set", json!({ "project": project }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["empty"], false);
        let r = call(&mut app, &ctx, "project.get", json!({}));
        assert_eq!(r["result"]["windage"], windage);
        // A bare loading condition is not a project, and nothing changes.
        let r = call(
            &mut app,
            &ctx,
            "project.set",
            json!({ "project": { "name": "x" } }),
        );
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(app.project().windage.len(), 1);
    }

    #[test]
    fn loading_compute_over_control() {
        let (mut app, ctx) = demo();
        let cond_text = std::fs::read_to_string(format!(
            "{}/../../examples/demo-loadcase.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let cond: seacraft_core::loading::LoadingCondition =
            serde_json::from_str(&cond_text).unwrap();
        let project = seacraft_core::project::Project {
            loading_conditions: vec![cond],
            ..Default::default()
        };
        app.set_project(project);

        let r = call(&mut app, &ctx, "loading.compute", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        let disp = r["result"]["displacement_t"]
            .as_f64()
            .expect("displacement");
        assert!((disp - 12.147).abs() < 0.05, "disp {disp}");
        let draft = r["result"]["draft"].as_f64().expect("draft");
        assert!((draft - 0.620).abs() < 0.01, "draft {draft}");
    }

    #[test]
    fn damage_compute_over_control() {
        let (mut app, ctx) = demo();
        let dc_text = std::fs::read_to_string(format!(
            "{}/../../examples/demo-damage.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let dc: seacraft_core::damage::DamageCase = serde_json::from_str(&dc_text).unwrap();
        let project = seacraft_core::project::Project {
            damage_cases: vec![dc],
            ..Default::default()
        };
        app.set_project(project);

        let r = call(&mut app, &ctx, "damage.compute", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["equilibrium_found"], true);
        let draft = r["result"]["draft"].as_f64().expect("draft");
        assert!(draft > 0.60, "draft {draft}");
    }

    #[test]
    fn inspect_set_move_hydro_and_render_over_control() {
        let (mut app, ctx) = demo();

        // ---- inspect on the freshly loaded demo ------------------------
        let r = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        let v = &r["result"];
        assert_eq!(v["version"], seacraft_core::VERSION);
        assert_eq!(v["dirty"], true, "a built-in demo is unsaved work: {v}");
        assert_eq!(v["level"], 2);
        assert_eq!(v["mode"], "loop");
        assert_eq!(v["active_view"], 0);
        assert_eq!(v["undo_len"], 0);
        assert!(v["points"].as_u64().unwrap() > 10, "{v}");
        assert!(v["faces"].as_u64().unwrap() > 5, "{v}");
        assert_eq!(v["layers"], 1);
        assert_eq!(v["selected_point"], Value::Null);
        assert_eq!(v["display"]["shaded"], true);
        assert_eq!(v["display"]["analysis"], "shaded");
        assert_eq!(v["hydro"]["draft"], 0.62);
        assert_eq!(v["hydro"]["kg"], 2.2);
        assert_eq!(v["hydro"]["free_trim"], false);
        assert!(v["hydro"]["result"].is_null(), "nothing measured yet: {v}");

        // ---- ui.set changes display and model settings ------------------
        let r = call(
            &mut app,
            &ctx,
            "ui.set",
            json!({ "shaded": false, "analysis": "zebra", "level": 1, "active_view": 2 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["display"]["shaded"], false);
        assert_eq!(r["result"]["display"]["analysis"], "zebra");
        assert_eq!(r["result"]["level"], 1);
        assert_eq!(r["result"]["active_view"], 2);
        assert_eq!(
            app.display().analysis,
            AnalysisMode::Zebra { stripes: 14.0 }
        );
        for bad in [
            json!({ "nope": 1 }),
            json!({ "level": 9 }),
            json!({ "level": 1.5 }),
            json!({ "level": "high" }),
            json!({ "active_view": -1 }),
            json!({ "analysis": "hologram" }),
            json!({ "shaded": "yes" }),
            // A batch with one bad key applies nothing at all.
            json!({ "shaded": true, "waterplane": "maybe" }),
        ] {
            let r = call(&mut app, &ctx, "ui.set", bad.clone());
            assert_eq!(r["ok"], false, "{bad}: {r}");
        }
        assert!(!app.display().shaded, "a refused batch changed nothing");
        assert!(app.display().waterplane);

        // ---- ui.view and ui.camera -------------------------------------
        let r = call(&mut app, &ctx, "ui.view", json!({ "view": "body" }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.active_view(), 3);
        assert_eq!(r["result"]["camera"]["kind"], "Body plan");
        assert_eq!(
            call(&mut app, &ctx, "ui.view", json!({ "view": "fit" }))["ok"],
            true
        );
        assert_eq!(
            call(&mut app, &ctx, "ui.view", json!({ "view": "sideways" }))["ok"],
            false
        );
        assert_eq!(call(&mut app, &ctx, "ui.view", json!({}))["ok"], false);

        let r = call(
            &mut app,
            &ctx,
            "ui.camera",
            json!({ "view": 1, "yaw": 90, "pitch": 10, "ortho_height": 6.0 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(
            (app.cameras()[1].yaw - std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "{r}"
        );
        assert!((app.cameras()[1].pitch - 10f64.to_radians()).abs() < 1e-9);
        assert_eq!(r["result"]["ortho_height"], 6.0);
        // A perspective camera can be aimed and moved; the pitch stops short
        // of the pole where the basis degenerates.
        let r = call(
            &mut app,
            &ctx,
            "ui.camera",
            json!({ "view": 0, "yaw": -30, "pitch": 90, "distance": 40, "target": [1, 2, 3] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!((app.cameras()[0].target - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-12);
        assert!(app.cameras()[0].pitch < std::f64::consts::FRAC_PI_2);
        for bad in [
            json!({ "distance": 0 }),
            json!({ "distance": -1 }),
            json!({ "ortho_height": 0 }),
            json!({ "distance": "far" }),
            json!({ "view": 7 }),
            json!({ "target": [1, 2] }),
            json!({ "target": [1, 2, "x"] }),
            json!({ "yaw": "left" }),
        ] {
            assert_eq!(
                call(&mut app, &ctx, "ui.camera", bad.clone())["ok"],
                false,
                "{bad}"
            );
        }

        // ---- point.move: undoable, and the next frame sees it -----------
        let before = app.surface().unwrap().points[3].p;
        let r = call(
            &mut app,
            &ctx,
            "point.move",
            json!({ "index": 3, "delta": [0.25, 0.0, 0.1] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let after = app.surface().unwrap().points[3].p;
        assert!(
            (after - before - Vec3::new(0.25, 0.0, 0.1)).length() < 1e-12,
            "{after:?} vs {before:?}"
        );
        assert_eq!(app.undo_len(), 1, "the move is undoable");
        assert_eq!(app.selected(), Some(3));
        assert_eq!(r["result"]["undo_len"], 1);
        let r = call(
            &mut app,
            &ctx,
            "point.move",
            json!({ "index": 3, "xyz": [-1.0, -2.0, 0.5] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let moved = app.surface().unwrap().points[3].p;
        assert!((moved - Vec3::new(-1.0, -2.0, 0.5)).length() < 1e-12, "{r}");
        assert_eq!(app.undo_len(), 2);
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "point.move",
                json!({ "index": 99999, "xyz": [0, 0, 0] })
            )["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "point.move",
                json!({ "index": -3, "xyz": [0, 0, 0] })
            )["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "point.move", json!({ "index": 3 }))["ok"],
            false,
            "needs xyz or delta"
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "point.move",
                json!({ "index": 3, "xyz": [1, 2] })
            )["ok"],
            false
        );
        let r = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(r["result"]["selected_point"], 3);
        assert_eq!(r["result"]["undo_len"], 2);

        // ---- edge.crease -------------------------------------------------
        let edge = app.surface().unwrap().edges[0].key();
        let r = call(
            &mut app,
            &ctx,
            "edge.crease",
            json!({ "edge": [edge.0, edge.1], "crease": true }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.surface().unwrap().edges[0].crease);
        assert_eq!(
            call(&mut app, &ctx, "edge.crease", json!({ "edge": [0, 99999] }))["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "edge.crease", json!({ "edge": [0] }))["ok"],
            false
        );

        // ---- synthetic input waits for the next frame --------------------
        let r = call(&mut app, &ctx, "ui.click", json!({ "x": 700, "y": 400 }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.synthetic_len(), 3, "move, press, release");
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&mut raw);
        assert_eq!(raw.events.len(), 3);
        assert_eq!(app.synthetic_len(), 0, "the queue is drained by the frame");
        assert_eq!(
            call(&mut app, &ctx, "ui.key", json!({ "key": "Escape" }))["ok"],
            true
        );
        assert_eq!(
            call(&mut app, &ctx, "ui.key", json!({ "key": "Num2" }))["ok"],
            true,
            "num aliases name the number keys"
        );
        assert_eq!(
            call(&mut app, &ctx, "ui.key", json!({ "key": "NoSuchKey" }))["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "ui.drag",
                json!({ "from": [10, 10], "to": [80, 60], "steps": 4 })
            )["ok"],
            true
        );
        assert_eq!(
            call(&mut app, &ctx, "ui.drag", json!({ "from": [10] }))["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "ui.scroll",
                json!({ "x": 700, "y": 400, "delta": -2 })
            )["ok"],
            true
        );
        assert_eq!(call(&mut app, &ctx, "ui.scroll", json!({}))["ok"], false);
        assert_eq!(
            call(&mut app, &ctx, "ui.move", json!({ "x": 5, "y": 5 }))["ok"],
            true
        );

        // ---- hydro.compute reports the measurement -----------------------
        let r = call(
            &mut app,
            &ctx,
            "hydro.compute",
            json!({ "draft": 0.5, "kg": 2.0 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["draft"], 0.5);
        assert_eq!(r["result"]["kg"], 2.0);
        let volume = r["result"]["result"]["volume_m3"].as_f64().expect("volume");
        assert!(volume > 5.0 && volume < 30.0, "volume {volume}");
        assert_eq!(r["result"]["result"]["draft"], 0.5);
        assert_eq!(
            call(&mut app, &ctx, "hydro.compute", json!({ "draft": 1e9 }))["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "hydro.compute", json!({ "heel_deg": 400 }))["ok"],
            false
        );

        // ---- stability.cross_curves --------------------------------------
        let r = call(
            &mut app,
            &ctx,
            "stability.cross_curves",
            json!({ "kg": 2.2, "free_trim": false }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let curves = r["result"]["curves"].as_array().expect("curves");
        assert_eq!(curves.len(), 9, "0..80 deg in 10 deg steps");
        assert_eq!(curves[0]["points"].as_array().unwrap().len(), 5);
        assert!(
            r["result"]["imo"].is_object(),
            "IMO criteria for the design displacement: {r}"
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "stability.cross_curves",
                json!({ "lcg": "forward" })
            )["ok"],
            false
        );

        // ---- curves.bonjean: read-only, and it does not dirty the model ---
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "curves.bonjean",
            json!({ "stations": 11, "drafts": [0.3, 0.6, 1.2] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["drafts_m"], json!([0.3, 0.6, 1.2]));
        let sections = r["result"]["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), 11, "{r}");
        let area = sections[5]["areas_m2"][1].as_f64().expect("an area");
        assert!(area > 0.5 && area < 3.0, "mid station at T = 0.6: {area}");
        assert!(
            r["result"]["area_max_m2"].as_f64().unwrap_or(0.0) >= area,
            "the family maximum covers the mid station: {r}"
        );
        assert_eq!(
            app.is_dirty(),
            dirty_before,
            "a measurement must not dirty the model"
        );
        assert_eq!(
            app.undo_len(),
            undo_before,
            "a measurement must not push undo"
        );
        // The default draft range comes from the hull's own depth.
        let d = call(
            &mut app,
            &ctx,
            "curves.bonjean",
            json!({ "draft_steps": 3 }),
        );
        assert_eq!(d["ok"], true, "{d}");
        let top = d["result"]["drafts_m"][2].as_f64().unwrap_or(0.0);
        assert!(
            (top - d["result"]["depth_m"].as_f64().unwrap_or(-1.0)).abs() < 1e-9,
            "{d}"
        );
        for bad in [
            json!({ "drafts": [0.5] }),
            json!({ "drafts": [0.0, 0.5] }),
            json!({ "drafts": "0.5,1.0" }),
            json!({ "stations": 500 }),
            json!({ "level": 9 }),
        ] {
            assert_eq!(
                call(&mut app, &ctx, "curves.bonjean", bad.clone())["ok"],
                false,
                "{bad}"
            );
        }

        // ---- model.split_face / model.insert_edge: edits with undo ----------
        let faces_before = app.face_count();
        let undo_before = app.undo_len();
        let r = call(&mut app, &ctx, "model.split_face", json!({ "face": 0 }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["pieces"], 4, "a quad fans into four: {r}");
        assert_eq!(app.face_count(), faces_before + 3, "{r}");
        assert_eq!(
            app.undo_len(),
            undo_before + 1,
            "a topology edit is undoable"
        );
        assert_eq!(r["result"]["undo_len"], json!(app.undo_len()));
        // The fan's point is a real control point and the fan edges exist.
        let new_point = r["result"]["point"].as_u64().unwrap_or(u64::MAX);
        let r = call(
            &mut app,
            &ctx,
            "edge.crease",
            json!({ "edge": [new_point, 0], "crease": true }),
        );
        assert_eq!(r["ok"], true, "{r}");
        // The edit is one snapshot, so one undo (Ctrl+Z in the window, the
        // history unit test covers the key path) takes the whole fan away.
        // `ui.key` only queues synthetic input; the frame applies it, so the
        // count is checked through the queue length here instead.
        assert!(app.undo_len() > undo_before, "no snapshot was taken");
        let r = call(
            &mut app,
            &ctx,
            "ui.key",
            json!({ "key": "Z", "ctrl": true }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.synthetic_len() >= 2, "press and release are queued");
        // insert_edge shares the same validation path as the core and CLI tests
        // (which exercise the positive case); here it is the refusals and the
        // snapshot that matter for the control channel.
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.insert_edge",
                json!({ "face": 0, "a": 0, "b": 1 })
            )["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.insert_edge",
                json!({ "face": 0, "a": 0, "b": 100_000 })
            )["ok"],
            false
        );
        assert_eq!(
            app.face_count(),
            faces_before + 3,
            "a refused insert_edge changes nothing (the split above stands)"
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.split_face",
                json!({ "face": 10_000 })
            )["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "model.split_face", json!({}))["ok"],
            false
        );

        // ---- resistance.compute / planing.compute: read-only power ---------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "resistance.compute",
            json!({ "draft": 0.62, "speeds": [8.0, 12.0], "propeller_diameter": 0.45 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let pts = r["result"]["speeds"].as_array().expect("speeds");
        assert_eq!(pts.len(), 2, "{r}");
        assert!(
            (pts[0]["pe_w"].as_f64().unwrap_or(0.0)
                - pts[0]["rt_n"].as_f64().unwrap_or(0.0)
                    * pts[0]["speed_m_s"].as_f64().unwrap_or(0.0))
            .abs()
                < 1e-6
        );
        assert_eq!(
            r["result"]["propulsion"].as_array().map(Vec::len),
            Some(2),
            "{r}"
        );
        let rm = call(
            &mut app,
            &ctx,
            "resistance.compute",
            json!({ "draft": 0.62, "speeds": [8.0, 12.0], "method": "michell", "cat_spacing": 15.0 }),
        );
        assert_eq!(rm["ok"], true, "{rm}");
        assert_eq!(rm["result"]["method"], "michell");
        assert_eq!(rm["result"]["speeds"].as_array().map(Vec::len), Some(2));
        let rd = call(
            &mut app,
            &ctx,
            "resistance.compute",
            json!({ "draft": 0.62, "speeds": [6.0, 8.0], "method": "dsyhs" }),
        );
        assert_eq!(rd["ok"], true, "{rd}");
        assert_eq!(rd["result"]["method"], "dsyhs");
        assert_eq!(rd["result"]["speeds"].as_array().map(Vec::len), Some(2));
        let p = call(
            &mut app,
            &ctx,
            "planing.compute",
            json!({ "speeds": [20.0], "deadrise": 15.0 }),
        );
        assert_eq!(p["ok"], true, "{p}");
        let row = &p["result"]["speeds"][0];
        let lcp = row["lcp_over_b"].as_f64().unwrap_or(0.0);
        let lcg = p["result"]["setup"]["hull"]["lcg_from_transom_m"]
            .as_f64()
            .unwrap_or(0.0)
            / p["result"]["setup"]["hull"]["beam_m"]
                .as_f64()
                .unwrap_or(1.0);
        assert!((lcp - lcg).abs() < 1e-3, "LCP/b {lcp} against LCG/b {lcg}");
        assert_eq!(
            app.is_dirty(),
            dirty_before,
            "a measurement must not dirty the model"
        );
        assert_eq!(
            app.undo_len(),
            undo_before,
            "a measurement must not push undo"
        );
        // Unknown keys are ignored by the control channel; a bad *value* is not.
        assert_eq!(
            call(&mut app, &ctx, "planing.compute", json!({ "draft": -1.0 }))["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "resistance.compute",
                json!({ "draft": 0.0 })
            )["ok"],
            false
        );

        // ---- seakeeping.compute: strip-theory motions ----------------------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "seakeeping.compute",
            json!({ "draft": 0.62, "speed_kn": 6.0, "kg": 2.2, "points": 5, "stations": 11 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["backend"], "app");
        let freqs = r["result"]["frequencies"]
            .as_array()
            .expect("frequencies array");
        assert_eq!(freqs.len(), 5);
        assert!(r["result"]["natural_heave_omega"].as_f64().unwrap() > 0.0);
        assert_eq!(app.is_dirty(), dirty_before);
        assert_eq!(app.undo_len(), undo_before);
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "seakeeping.compute",
                json!({ "draft": -1.0 })
            )["ok"],
            false
        );

        // ---- seakeeping.irregular: sea-state responses ---------------------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "seakeeping.irregular",
            json!({
                "draft": 0.62, "speed_kn": 6.0, "kg": 0.392, "stations": 11,
                "hs": 1.0, "tp": 4.0, "omegas": 20,
                "points": [{"name": "Deck", "x": 0.0}]
            }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["backend"], "app");
        assert_eq!(r["result"]["points"][0]["name"], "Deck");
        assert!(r["result"]["heave"]["significant"].as_f64().unwrap() > 0.0);
        assert_eq!(app.is_dirty(), dirty_before);
        assert_eq!(app.undo_len(), undo_before);
        for bad in [
            json!({ "draft": 0.62 }),
            json!({ "draft": 0.62, "hs": 1.0, "tp": 0.0 }),
            json!({ "draft": 0.62, "hs": 1.0, "tp": 4.0, "gamma": 0.1 }),
            json!({ "draft": 0.62, "hs": 1.0, "tp": 4.0, "points": [{"x": "a"}] }),
        ] {
            assert_eq!(
                call(&mut app, &ctx, "seakeeping.irregular", bad.clone())["ok"],
                false,
                "{bad}"
            );
        }

        // ---- curves.offsets: the same table from the app's model ----------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "curves.offsets",
            json!({ "stations": 9, "waterlines": [0.62], "buttocks": [1.0] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["waterlines_m"], json!([0.62]));
        let rows = r["result"]["stations"].as_array().expect("stations");
        assert_eq!(rows.len(), 9, "{r}");
        let widest = rows
            .iter()
            .filter_map(|row| row["waterline_half_breadths"][0].as_f64())
            .fold(0.0_f64, f64::max);
        assert!(
            (widest - 1.771).abs() < 0.04,
            "half-breadth at T = 0.62: {widest}"
        );
        assert_eq!(
            app.is_dirty(),
            dirty_before,
            "a measurement must not dirty the model"
        );
        assert_eq!(
            app.undo_len(),
            undo_before,
            "a measurement must not push undo"
        );
        for bad in [
            json!({ "waterlines": [] }),
            json!({ "buttocks": [0.0] }),
            json!({ "stations": 1e9 }),
        ] {
            assert_eq!(
                call(&mut app, &ctx, "curves.offsets", bad.clone())["ok"],
                false,
                "{bad}"
            );
        }

        // ---- ui.render writes a PNG ---------------------------------------
        let png_path = scratch("render.png");
        let path_text = png_path.to_string_lossy().to_string();
        let r = call(
            &mut app,
            &ctx,
            "ui.render",
            json!({ "path": path_text, "width": 64, "height": 48, "view": 1, "mode": "zebra" }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["width"], 64);
        assert_eq!(r["result"]["height"], 48);
        let bytes = std::fs::read(&png_path).expect("the PNG was written");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
        assert_eq!(r["result"]["bytes"], bytes.len());
        let _ = std::fs::remove_file(&png_path);
        for bad in [
            json!({ "width": 10, "height": 10 }),
            json!({ "path": path_text, "width": 0 }),
            json!({ "path": path_text, "width": MAX_RENDER + 1 }),
            json!({ "path": path_text, "height": 10.5 }),
            json!({ "path": path_text, "mode": "hologram" }),
            json!({ "path": path_text, "view": 4 }),
        ] {
            assert_eq!(
                call(&mut app, &ctx, "ui.render", bad.clone())["ok"],
                false,
                "{bad}"
            );
        }
        assert!(
            std::fs::metadata(&png_path).is_err(),
            "a refused render writes nothing"
        );

        // ---- hydro.trim: read-only trim scan --------------------------------
        let dirty_before_r = app.is_dirty();
        let undo_before_r = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "hydro.trim",
            json!({ "speed_kn": 10.0, "steps": 5 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let pts = r["result"]["points"].as_array().expect("points");
        assert_eq!(pts.len(), 5, "{r}");
        let zero = pts
            .iter()
            .find(|p| p["trim_m"].as_f64().unwrap_or(1.0).abs() < 1e-12)
            .expect("an even-keel point");
        assert!(zero["rt_n"].as_f64().unwrap_or(0.0) > 0.0, "{zero}");
        assert_eq!(
            app.is_dirty(),
            dirty_before_r,
            "a scan must not dirty the model"
        );
        assert_eq!(app.undo_len(), undo_before_r, "a scan must not push undo");
        assert_eq!(
            call(&mut app, &ctx, "hydro.trim", json!({ "speed_kn": 0.0 }))["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "hydro.trim",
                json!({ "speed_kn": 10.0, "from": 0.4, "to": 0.2 })
            )["ok"],
            false
        );

        // ---- hydro.table: read-only, and the top row is the dock's number ---
        let dirty_before_t = app.is_dirty();
        let undo_before_t = app.undo_len();
        let r = call(&mut app, &ctx, "hydro.table", json!({ "steps": 4 }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["drafts"], json!(4), "{r}");
        let rows = r["result"]["rows"].as_array().expect("rows");
        let top = rows.last().expect("a top row");
        assert!(
            (top["draft"].as_f64().unwrap_or(0.0) - app.hydro_options().draft).abs() < 1e-9,
            "{top}"
        );
        assert!(top["volume_m3"].as_f64().unwrap_or(0.0) > 0.0, "{top}");
        assert_eq!(
            app.is_dirty(),
            dirty_before_t,
            "a table must not dirty the model"
        );
        assert_eq!(app.undo_len(), undo_before_t, "a table must not push undo");
        let two = call(
            &mut app,
            &ctx,
            "hydro.table",
            json!({ "drafts": [0.4, 0.62] }),
        );
        assert_eq!(two["result"]["drafts"], json!(2), "{two}");
        assert_eq!(
            call(&mut app, &ctx, "hydro.table", json!({ "drafts": [0.0] }))["ok"],
            false
        );

        // ---- model.optimize: an edit, one undo step, the draft follows ------
        let undo_before = app.undo_len();
        let draft_before = app.hydro_options().draft;
        let points_before = app.control_point_count();
        let r = call(
            &mut app,
            &ctx,
            "model.optimize",
            json!({ "speed_kn": 8.0, "vars": ["lb"], "max_evaluations": 6, "level": 1 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let rep = &r["result"]["report"];
        assert!(
            rep["optimised"]["rt_kn"].as_f64().unwrap_or(f64::MAX)
                <= rep["baseline"]["rt_kn"].as_f64().unwrap_or(0.0) + 1e-12,
            "{r}"
        );
        assert_eq!(
            app.undo_len(),
            undo_before + 1,
            "an optimisation is one snapshot"
        );
        assert!(app.is_dirty(), "the model changed");
        assert_eq!(
            app.control_point_count(),
            points_before,
            "the topology is kept"
        );
        let opt_draft = rep["optimised"]["draft_m"].as_f64().unwrap_or(0.0);
        assert!(
            (app.hydro_options().draft - opt_draft).abs() < 1e-12,
            "the app measures at the new draft"
        );
        // Undo puts the hull back (the draft is the app's own setting).
        assert_eq!(call(&mut app, &ctx, "history.undo", json!({}))["ok"], true);
        assert_eq!(app.undo_len(), undo_before);
        let _ = draft_before;
        assert_eq!(
            call(&mut app, &ctx, "model.optimize", json!({}))["ok"],
            false,
            "speed_kn is required"
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.optimize",
                json!({ "speed_kn": 8.0, "vars": ["wibble"] })
            )["ok"],
            false
        );
        assert_eq!(
            app.undo_len(),
            undo_before,
            "a refused optimisation leaves no snapshot"
        );

        // ---- model.transform: a real edit, with undo ------------------------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "model.transform",
            json!({ "lwl": 13.0, "draft": 0.9 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let rep = &r["result"]["report"];
        assert!(
            (rep["after"]["lwl"].as_f64().unwrap_or(0.0) - 13.0).abs() < 0.05,
            "{r}"
        );
        assert!(
            (rep["after"]["draft"].as_f64().unwrap_or(0.0) - 0.9).abs() < 1e-3,
            "{r}"
        );
        // The app measures at the new draft, and the edit is undoable.
        assert!(
            (app.hydro_options().draft - 0.9).abs() < 1e-3,
            "{}",
            app.hydro_options().draft
        );
        assert_eq!(
            app.undo_len(),
            undo_before + 1,
            "a transform is one snapshot"
        );
        assert!(app.is_dirty(), "the model changed");
        let faces_after = app.face_count();
        assert_eq!(
            faces_after,
            faces_before + 3,
            "a transform keeps the topology: {r}"
        );
        // A mirror is a valid transform; a bad axis and an empty request are not.
        assert_eq!(
            call(&mut app, &ctx, "model.transform", json!({ "mirror": "x" }))["ok"],
            true
        );
        assert_eq!(
            call(&mut app, &ctx, "model.transform", json!({ "mirror": "w" }))["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "model.transform", json!({}))["ok"],
            false
        );
        let _ = dirty_before;

        // ---- curves.unfold: read-only plate development ---------------------
        let dirty_before = app.is_dirty();
        let undo_before = app.undo_len();
        let r = call(
            &mut app,
            &ctx,
            "curves.unfold",
            json!({ "axis": "z", "from": 0.2, "to": 0.45, "x_from": 3.0, "x_to": 6.0 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        let plate = &r["result"]["plate"];
        assert!(plate["area_2d_m2"].as_f64().unwrap_or(0.0) > 0.0, "{r}");
        assert!(plate["developable"].as_bool().unwrap_or(false), "{r}");
        assert_eq!(
            app.is_dirty(),
            dirty_before,
            "a measurement must not dirty the model"
        );
        assert_eq!(
            app.undo_len(),
            undo_before,
            "a measurement must not push undo"
        );
        assert_eq!(
            call(&mut app, &ctx, "curves.unfold", json!({ "axis": "w" }))["ok"],
            false
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "curves.unfold",
                json!({ "from": 50.0, "to": 60.0 })
            )["ok"],
            false
        );

        // ---- model.save / model.open / model.export ------------------------
        let fef_path = scratch("hull.fef");
        let fef_text = fef_path.to_string_lossy().to_string();
        let r = call(&mut app, &ctx, "model.save", json!({ "path": fef_text }));
        assert_eq!(r["ok"], true, "{r}");
        assert!(!app.is_dirty(), "saving clears the dirty flag");
        let points = app.control_point_count();
        let r = call(&mut app, &ctx, "model.open", json!({ "path": fef_text }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["points"], points);
        assert_eq!(app.path().map(|p| p.to_path_buf()), Some(fef_path.clone()));
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.open",
                json!({ "path": "/nonexistent/hull.fef" })
            )["ok"],
            false
        );
        assert_eq!(call(&mut app, &ctx, "model.open", json!({}))["ok"], false);
        let _ = std::fs::remove_file(&fef_path);

        let obj_path = scratch("hull.obj");
        let obj_text = obj_path.to_string_lossy().to_string();
        let r = call(
            &mut app,
            &ctx,
            "model.export",
            json!({ "path": obj_text, "format": "obj" }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(
            std::fs::read_to_string(&obj_path).unwrap().contains("v "),
            "an OBJ with vertices"
        );
        let _ = std::fs::remove_file(&obj_path);
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.export",
                json!({ "path": obj_text, "format": "tiff" })
            )["ok"],
            false
        );
        assert_eq!(
            call(&mut app, &ctx, "model.export", json!({ "path": obj_text }))["ok"],
            false
        );

        // ---- report.design ------------------------------------------------
        let report_path = scratch("dossier.html");
        let report_text = report_path.to_string_lossy().to_string();
        let r = call(
            &mut app,
            &ctx,
            "report.design",
            json!({ "path": report_text, "draft": 0.62, "kg": 2.2 }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(report_path.exists());
        assert!(r["result"]["bytes"].as_u64().unwrap_or(0) > 1000);
        let _ = std::fs::remove_file(&report_path);
        assert_eq!(
            call(&mut app, &ctx, "report.design", json!({}))["ok"],
            false
        );

        // ---- model.load_demo ----------------------------------------------
        let r = call(&mut app, &ctx, "model.load_demo", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["symmetric"], false);
        let r = call(
            &mut app,
            &ctx,
            "model.load_demo",
            json!({ "symmetric": true }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["symmetric"], true);
        assert!(
            app.surface().unwrap().layers.iter().any(|l| l.symmetric),
            "a half model"
        );
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "model.load_demo",
                json!({ "symmetric": "yes" })
            )["ok"],
            false
        );

        // A symmetric layer models only y >= 0, so a move across the
        // centreline is clamped instead of producing an unmirrorable half.
        let off_centre = app
            .surface()
            .unwrap()
            .points
            .iter()
            .position(|p| p.p.y > 0.1)
            .expect("a point off the centreline");
        let r = call(
            &mut app,
            &ctx,
            "point.move",
            json!({ "index": off_centre, "xyz": [3.0, -2.0, 0.4] }),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["clamped"], true, "{r}");
        assert_eq!(r["result"]["point"][1], 0.0, "{r}");
        assert_eq!(app.surface().unwrap().points[off_centre].p.y, 0.0);
    }

    #[test]
    fn hostile_requests_never_panic() {
        let (mut app, ctx) = demo();
        let hostile = [
            json!({}),
            json!({"x": 1e300, "y": -1e300, "width": 1e12, "height": -5, "path": "", "index": 1e30}),
            json!({"xyz": [1e308, 1e308, 1e308], "delta": [f64::MAX, 0.0, 0.0], "steps": 1e12, "edge": [1, 2], "x0": "a"}),
            json!({"key": 5, "button": "sideways", "view": "nope", "analysis": 7, "level": "high", "symmetric": 1}),
            json!({"target": {"x": 1}, "lcg": "nope", "free_trim": "yes", "kg": -1, "draft": "deep"}),
            json!({"edge": [0, 0], "crease": "maybe", "format": "tiff", "mode": [], "path": 7}),
            json!({"index": 0.5, "delta": [], "steps": -1, "delta_scroll": null}),
            json!({"view": [0, 1], "active_view": {}, "shaded": null}),
        ];
        let methods = [
            "ui.inspect",
            "ui.set",
            "ui.view",
            "ui.camera",
            "ui.click",
            "ui.move",
            "ui.drag",
            "ui.scroll",
            "ui.key",
            "ui.render",
            "model.load_demo",
            "model.generate",
            "model.open",
            "model.save",
            "model.export",
            "project.get",
            "project.set",
            "loading.compute",
            "damage.compute",
            "performance.compute",
            "seakeeping.irregular",
            "point.move",
            "edge.crease",
            "hydro.compute",
            "stability.cross_curves",
            "stability.weather",
            "report.design",
            "app.quit",
            "no.such.method",
        ];
        for method in methods {
            for params in &hostile {
                let reply = call(&mut app, &ctx, method, params.clone());
                assert!(
                    reply.get("ok").and_then(Value::as_bool).is_some(),
                    "{method} {params}: {reply}"
                );
            }
        }
        // Params of any other JSON type are refused, not ignored.
        for bad in [json!([1, 2]), json!("x"), json!(7), json!(true)] {
            let reply = call(&mut app, &ctx, "ui.inspect", bad.clone());
            assert_eq!(reply["ok"], false, "{bad}");
        }
        // The refusal is clean: the model and the queues are untouched.
        assert!(app.control_point_count() > 0);
        assert!(app.synthetic_len() <= MAX_SYNTHETIC);
        assert_eq!(
            call(&mut app, &ctx, "app.quit", json!({}))["result"]["quitting"],
            true
        );
    }

    #[test]
    fn tcp_listener_serves_json_lines_round_trip() {
        let listener = spawn_listener("127.0.0.1:0", ControlOptions::default())
            .expect("bind an ephemeral port");
        let addr = listener.addr;
        let rx = listener.receiver;
        let token = listener.token.expect("token");
        assert_ne!(addr.port(), 0, "the listener reports the port it got");
        let (mut app, ctx) = demo();

        let stream = TcpStream::connect(addr).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
        let mut writer = stream;

        fn read_reply(reader: &mut BufReader<TcpStream>) -> Value {
            let mut line = String::new();
            reader.read_line(&mut line).expect("a reply line");
            serde_json::from_str(line.trim())
                .unwrap_or_else(|e| panic!("the reply is not JSON: {line:?} ({e})"))
        }

        // Authenticate first
        writer
            .write_all(
                format!("{{\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n")
                    .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();
        let auth_reply = read_reply(&mut reader);
        assert_eq!(auth_reply["ok"], true, "{auth_reply}");

        // Send one request and answer it as the window's frame would.
        let exchange = |writer: &mut TcpStream,
                        reader: &mut BufReader<TcpStream>,
                        app: &mut App,
                        request: &str|
         -> Value {
            writer.write_all(request.as_bytes()).unwrap();
            writer.write_all(b"\n").unwrap();
            writer.flush().unwrap();
            let request = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the request reached the app");
            let reply = handle(app, &ctx, &request);
            request
                .reply
                .send(reply)
                .expect("the app is still listening");
            read_reply(reader)
        };

        let first = exchange(
            &mut writer,
            &mut reader,
            &mut app,
            "{\"method\":\"ui.inspect\",\"params\":{}}",
        );
        assert_eq!(first["ok"], true, "{first}");
        assert_eq!(first["result"]["version"], seacraft_core::VERSION);
        assert_eq!(first["result"]["points"], app.control_point_count());

        // A malformed line is answered by the socket, so the app never sees it.
        writer.write_all(b"not json\n").unwrap();
        writer.flush().unwrap();
        let bad = read_reply(&mut reader);
        assert_eq!(bad["ok"], false, "{bad}");
        assert!(
            bad["error"].as_str().unwrap().contains("invalid JSON"),
            "{bad}"
        );

        // An unknown method is a clean error from `handle`.
        let unknown = exchange(
            &mut writer,
            &mut reader,
            &mut app,
            "{\"method\":\"no.such.method\",\"params\":{}}",
        );
        assert_eq!(unknown["ok"], false, "{unknown}");
        assert!(
            unknown["error"]
                .as_str()
                .unwrap()
                .contains("unknown method"),
            "{unknown}"
        );

        // A line past the cap is refused and the rest of it discarded, so the
        // next request on the same connection still works.
        let mut too_long = vec![b'a'; MAX_LINE + 10];
        too_long.push(b'\n');
        writer.write_all(&too_long).unwrap();
        writer.flush().unwrap();
        let long = read_reply(&mut reader);
        assert_eq!(long["ok"], false, "{long}");
        assert!(long["error"].as_str().unwrap().contains("KiB"), "{long}");
        let again = exchange(
            &mut writer,
            &mut reader,
            &mut app,
            "{\"method\":\"ui.inspect\",\"params\":{}}",
        );
        assert_eq!(again["ok"], true, "{again}");
        assert_eq!(again["result"]["version"], first["result"]["version"]);
    }

    #[test]
    fn wrong_token_rejected_and_connection_closed() {
        let listener =
            spawn_listener("127.0.0.1:0", ControlOptions::default()).expect("spawn listener");
        let stream = TcpStream::connect(listener.addr).expect("connect");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;

        writer
            .write_all(b"{\"method\":\"auth\",\"params\":{\"token\":\"wrong_token_hex\"}}\n")
            .unwrap();
        writer.flush().unwrap();

        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read reply");
        assert!(n > 0);
        let reply: Value = serde_json::from_str(line.trim()).expect("json reply");
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].as_str().unwrap().contains("invalid token"));

        // Connection must be closed
        line.clear();
        let eof = reader.read_line(&mut line).expect("read eof");
        assert_eq!(eof, 0, "connection must be closed after failed auth");
    }

    #[test]
    fn right_token_authenticates_and_methods_work() {
        let listener =
            spawn_listener("127.0.0.1:0", ControlOptions::default()).expect("spawn listener");
        let token = listener.token.expect("token");
        let rx = listener.receiver;
        let (mut app, ctx) = demo();

        let stream = TcpStream::connect(listener.addr).expect("connect");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;

        // 1. Auth succeeds
        writer
            .write_all(
                format!("{{\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n")
                    .as_bytes(),
            )
            .unwrap();
        writer.flush().unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result"]["authenticated"], true);

        // 2. Next method works
        writer
            .write_all(b"{\"method\":\"ui.inspect\",\"params\":{}}\n")
            .unwrap();
        writer.flush().unwrap();

        let req = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("receive request");
        let res = handle(&mut app, &ctx, &req);
        req.reply.send(res).unwrap();

        line.clear();
        reader.read_line(&mut line).unwrap();
        let inspect_reply: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(inspect_reply["ok"], true);
        assert_eq!(inspect_reply["result"]["version"], seacraft_core::VERSION);
    }

    #[test]
    fn method_before_auth_is_rejected_and_connection_closed() {
        let listener =
            spawn_listener("127.0.0.1:0", ControlOptions::default()).expect("spawn listener");
        let stream = TcpStream::connect(listener.addr).expect("connect");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;

        writer
            .write_all(b"{\"method\":\"ui.inspect\",\"params\":{}}\n")
            .unwrap();
        writer.flush().unwrap();

        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read reply");
        assert!(n > 0);
        let reply: Value = serde_json::from_str(line.trim()).expect("json reply");
        assert_eq!(reply["ok"], false);
        assert!(reply["error"]
            .as_str()
            .unwrap()
            .contains("first request must be auth"));

        // Connection must be closed
        line.clear();
        let eof = reader.read_line(&mut line).expect("read eof");
        assert_eq!(eof, 0, "connection must be closed after method before auth");
    }

    #[test]
    fn read_root_violation_rejected() {
        let (mut app, ctx) = demo();
        let dir = std::env::temp_dir().join(format!("seacraft-read-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        app.set_control_roots(seacraft_core::control_auth::ControlRoots::new(
            vec![dir.clone()],
            vec![],
        ));

        // Reading /etc/passwd or /etc/hosts outside the root is rejected
        #[cfg(unix)]
        {
            let target = if std::path::Path::new("/etc/passwd").exists() {
                "/etc/passwd"
            } else {
                "/etc/hosts"
            };
            let r = call(&mut app, &ctx, "model.open", json!({ "path": target }));
            assert_eq!(r["ok"], false);
            assert!(
                r["error"]
                    .as_str()
                    .unwrap()
                    .contains("read-root rule violation"),
                "{r}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_root_violation_with_traversal_rejected() {
        let (mut app, ctx) = demo();
        let dir = std::env::temp_dir().join(format!("seacraft-write-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        app.set_control_roots(seacraft_core::control_auth::ControlRoots::new(
            vec![],
            vec![dir.clone()],
        ));

        // Writing with .. traversal outside root is rejected
        let traversal_path = dir.join("../evil_traversal.fef");
        let r = call(
            &mut app,
            &ctx,
            "model.save",
            json!({ "path": traversal_path.to_string_lossy().to_string() }),
        );
        assert_eq!(r["ok"], false);
        assert!(
            r["error"]
                .as_str()
                .unwrap()
                .contains("write-root rule violation"),
            "{r}"
        );

        let traversal_render = dir.join("../evil_traversal.png");
        let r2 = call(
            &mut app,
            &ctx,
            "ui.render",
            json!({ "path": traversal_render.to_string_lossy().to_string() }),
        );
        assert_eq!(r2["ok"], false);
        assert!(
            r2["error"]
                .as_str()
                .unwrap()
                .contains("write-root rule violation"),
            "{r2}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn control_catches_panics_and_history_mark_restore_works() {
        let (mut app, ctx) = demo();
        let r = call(&mut app, &ctx, "test.panic", json!({}));
        assert_eq!(r["ok"], false);
        assert!(r["error"]
            .as_str()
            .unwrap()
            .contains("internal error: deliberate control panic"));
        // GUI is still running
        let insp = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(insp["ok"], true);

        // Test history mark and restore
        let mark_res = call(&mut app, &ctx, "history.mark", json!({}));
        assert_eq!(mark_res["ok"], true);
        let mark = mark_res["result"]["mark"].as_u64().unwrap();

        let original_z = app.surface().unwrap().points[0].p.z;
        let mv = call(
            &mut app,
            &ctx,
            "point.move",
            json!({"index": 0, "xyz": [0.0, 0.0, original_z + 1.0]}),
        );
        assert_eq!(mv["ok"], true);
        assert!((app.surface().unwrap().points[0].p.z - (original_z + 1.0)).abs() < 1e-9);

        let restore = call(&mut app, &ctx, "history.restore", json!({"mark": mark}));
        assert_eq!(restore["ok"], true);
        assert!((app.surface().unwrap().points[0].p.z - original_z).abs() < 1e-9);
    }

    #[test]
    fn performance_compute_positive() {
        let (mut app, ctx) = demo();
        let set_res = call(
            &mut app,
            &ctx,
            "ui.set",
            json!({ "performance_panel": true }),
        );
        assert_eq!(set_res["ok"], true);
        assert!(app.show_performance());

        let res = call(
            &mut app,
            &ctx,
            "performance.compute",
            json!({
                "speed_kn": 0.0,
                "lambda_max": 10.0,
            }),
        );
        assert_eq!(res["ok"], true, "{res}");
        let r = &res["result"];
        let rows = r["resistance"]["rows"].as_array().expect("rows array");
        assert!(!rows.is_empty(), "resistance table must not be empty");
        let row_6kn = rows
            .iter()
            .find(|row| (row["speed_kn"].as_f64().unwrap_or(0.0) - 6.0).abs() < 1e-3)
            .expect("row at 6 kn");
        let rt_6kn = row_6kn["rt_kn"].as_f64().expect("rt_kn at 6 kn");
        assert!(
            rt_6kn > 0.0 && rt_6kn.is_finite(),
            "rt_kn must be finite positive: {rt_6kn}"
        );

        let freqs = r["raos"]["frequencies"]
            .as_array()
            .expect("raos frequencies");
        assert!(!freqs.is_empty());
        let longest = freqs
            .iter()
            .max_by(|a, b| {
                let la = a["lambda_over_l"].as_f64().unwrap_or(0.0);
                let lb = b["lambda_over_l"].as_f64().unwrap_or(0.0);
                la.total_cmp(&lb)
            })
            .expect("longest wavelength point");
        assert!(longest["lambda_over_l"].as_f64().unwrap() >= 10.0 - 1e-4);
        let heave = longest["heave_amp"].as_f64().expect("heave_amp");
        assert!(
            (heave - 1.0).abs() <= 0.05,
            "heave at longest wavelength must be ~1 (got {heave})"
        );
    }

    #[test]
    fn model_transform_takes_scale_and_translate_over_control() {
        let (mut app, ctx) = demo();
        let before = app.surface().unwrap().points[5].p;
        let r = call(
            &mut app,
            &ctx,
            "model.transform",
            json!({"scale": [1.1, 1.0, 1.0], "translate": [0.5, 0.0, 0.0]}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let after = app.surface().unwrap().points[5].p;
        assert!(after.x != before.x, "x moved: {before:?} -> {after:?}");
        assert!((after.z - before.z).abs() < 1e-12);
    }

    #[test]
    fn explicit_cross_curves_match_the_core_and_leave_the_dock_alone() {
        use seacraft_core::stability::{cross_curves, StabilityCondition};
        let (mut app, ctx) = demo();
        let r = call(
            &mut app,
            &ctx,
            "stability.cross_curves",
            json!({"kg": 1.0, "heels": [0.0, 30.0], "displacements": [10.0], "draft": 0.62}),
        );
        assert_eq!(r["ok"], true, "{r}");
        let res = &r["result"];
        assert_eq!(res["heels_deg"], json!([0.0, 30.0]), "{res}");
        let kn = res["curves"][1]["points"][0]["kn"]
            .as_f64()
            .expect("kn at 30");
        let mut base = app.hydro_options().clone();
        base.draft = 0.62;
        let hull = seacraft_core::hydro::Hull::new(app.surface().unwrap(), base.subdivision);
        let want = cross_curves(
            &hull,
            &[30.0],
            &[10.0],
            &StabilityCondition::fixed(1.0, base.trim),
            &base,
        );
        assert!((kn - want[0].points[0].kn.unwrap()).abs() < 1e-9);
        assert!(app.cross_curves().is_none(), "the dock was not recomputed");
        assert_eq!(app.kg(), demo().0.kg(), "the dock KG is unchanged");
    }

    #[test]
    fn a_refused_topology_edit_leaves_no_undo_entry() {
        let (mut app, ctx) = demo();
        let before = app.undo_len();
        // Points 0 and 1 are not two corners of face 5 that can be joined.
        let r = call(
            &mut app,
            &ctx,
            "model.insert_edge",
            json!({"face": 5, "a": 0, "b": 1}),
        );
        assert_eq!(r["ok"], false, "{r}");
        assert_eq!(app.undo_len(), before);
    }

    #[test]
    fn weather_and_seakeeping_default_to_the_app_state() {
        use seacraft_core::project::Project;
        use seacraft_core::weather::WindageItem;
        let (mut app, ctx) = demo();
        app.set_project(Project {
            windage: vec![WindageItem {
                name: "Mast".into(),
                area_m2: 2.0,
                z_centroid: 4.0,
                profile: None,
            }],
            ..Default::default()
        });
        let r = call(&mut app, &ctx, "stability.weather", json!({"kg": 1.0}));
        assert_eq!(r["ok"], true, "{r}");
        let extra = r["result"]["extra_windage_area_m2"]
            .as_f64()
            .or_else(|| r["result"]["weather"]["extra_windage_area_m2"].as_f64());
        assert_eq!(extra, Some(2.0), "{r}");

        app.set_kg(1.3);
        let sk = |app: &mut App, extra: Value| {
            let mut p = json!({"speed_kn": 0.0, "lambda_min": 1.0, "lambda_max": 3.0});
            if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            let r = call(app, &ctx, "seakeeping.compute", p);
            assert_eq!(r["ok"], true, "{r}");
            r["result"]["coefficients"][0]["c55"].as_f64().expect("c55")
        };
        // C55 carries ρg∇(KB − KG): the default must be the app's KG.
        let by_default = sk(&mut app, json!({}));
        assert_eq!(by_default, sk(&mut app, json!({"kg": 1.3})));
        assert_ne!(by_default, sk(&mut app, json!({"kg": 0.0})));
    }

    #[test]
    fn resistance_and_planing_take_a_level() {
        let (mut app, ctx) = demo();
        for method in ["resistance.compute", "planing.compute"] {
            let r = call(
                &mut app,
                &ctx,
                method,
                json!({"level": 1, "speeds": [20.0]}),
            );
            assert_eq!(r["ok"], true, "{method}: {r}");
            let r = call(&mut app, &ctx, method, json!({"level": 9}));
            assert_eq!(r["ok"], false, "{method} refuses level 9: {r}");
        }
    }

    #[test]
    fn undoing_a_transform_restores_the_measurement_draft() {
        let (mut app, ctx) = demo();
        let draft = app.hydro_options().draft;
        let r = call(
            &mut app,
            &ctx,
            "model.transform",
            json!({"draft": draft * 1.5}),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(
            (app.hydro_options().draft - draft).abs() > 1e-3,
            "draft moved"
        );
        app.undo();
        assert_eq!(app.hydro_options().draft, draft);
        app.redo();
        assert!((app.hydro_options().draft - draft).abs() > 1e-3);
    }

    #[test]
    fn restoring_a_mark_rolls_the_project_back_too() {
        let (mut app, ctx) = demo();
        let mark = call(&mut app, &ctx, "history.mark", json!({}));
        assert_eq!(mark["ok"], true, "{mark}");
        let id = mark["result"]["mark"].clone();
        let project = json!({
            "format": "seacraft-project",
            "version": 1,
            "windage": [{"name": "Mast", "area_m2": 1.0, "z_centroid": 2.0}],
        });
        let r = call(&mut app, &ctx, "project.set", json!({"project": project}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.project().windage.len(), 1);
        let r = call(&mut app, &ctx, "history.restore", json!({"mark": id}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(
            app.project().windage.is_empty(),
            "the project edit was rolled back"
        );
    }
}
