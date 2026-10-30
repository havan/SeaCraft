use std::path::PathBuf;

use eframe::egui;
use seacraft_gui::App;

fn usage() {
    println!(
        "seacraft-gui {}\n\n\
         Usage: seacraft-gui [--open FILE.fef] [--analysis MODE] [--combs] [--control HOST:PORT]\n\n\
         Options:\n  \
         --open FILE             Load a FREE!ship .fef model at startup\n  \
         --analysis MODE         Shading: shaded, zebra, curvature or draft\n  \
         --combs                 Overlay station/buttock/waterline curvature combs\n  \
         --control ADDR          Serve the JSON-lines control socket on ADDR\n                          \
         (e.g. 127.0.0.1:7373; 127.0.0.1:0 picks a free port)\n  \
         --control-token-file PATH Override default token file path\n  \
         --control-no-auth       Disable authentication (warning printed)\n  \
         --control-read-root DIR Restrict file reading to DIR (repeatable)\n  \
         --control-write-root DIR Restrict file writing to DIR (repeatable)\n  \
         --version               Print the version and exit\n  \
         --help                  Print this help and exit",
        seacraft_core::VERSION
    );
}

fn main() -> eframe::Result {
    let mut open: Option<PathBuf> = None;
    let mut analysis = seacraft_core::AnalysisMode::Shaded;
    let mut combs = false;
    let mut control_addr: Option<String> = None;
    let mut control_token_file: Option<PathBuf> = None;
    let mut control_no_auth = false;
    let mut control_read_roots: Vec<PathBuf> = Vec::new();
    let mut control_write_roots: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => open = args.next().map(PathBuf::from),
            "--combs" => combs = true,
            "--control" => match args.next() {
                Some(addr) if !addr.trim().is_empty() => control_addr = Some(addr),
                _ => {
                    eprintln!("--control needs HOST:PORT (e.g. 127.0.0.1:7373)");
                    std::process::exit(2);
                }
            },
            "--control-token-file" => match args.next() {
                Some(p) if !p.trim().is_empty() => control_token_file = Some(PathBuf::from(p)),
                _ => {
                    eprintln!("--control-token-file needs a file PATH");
                    std::process::exit(2);
                }
            },
            "--control-no-auth" => control_no_auth = true,
            "--control-read-root" => match args.next() {
                Some(p) if !p.trim().is_empty() => control_read_roots.push(PathBuf::from(p)),
                _ => {
                    eprintln!("--control-read-root needs a directory PATH");
                    std::process::exit(2);
                }
            },
            "--control-write-root" => match args.next() {
                Some(p) if !p.trim().is_empty() => control_write_roots.push(PathBuf::from(p)),
                _ => {
                    eprintln!("--control-write-root needs a directory PATH");
                    std::process::exit(2);
                }
            },
            "--analysis" => {
                let value = args.next().unwrap_or_default();
                analysis = match value.as_str() {
                    "shaded" => seacraft_core::AnalysisMode::Shaded,
                    "zebra" => seacraft_core::AnalysisMode::Zebra { stripes: 14.0 },
                    "curvature" => seacraft_core::AnalysisMode::Curvature { radius: 1.0 },
                    "draft" => seacraft_core::AnalysisMode::Draft { angle_deg: 3.0 },
                    other => {
                        eprintln!(
                            "unknown analysis mode: {other} (shaded, zebra, curvature, draft)"
                        );
                        std::process::exit(2);
                    }
                };
            }
            "--version" | "-V" => {
                println!("seacraft-gui {}", seacraft_core::VERSION);
                return Ok(());
            }
            "--help" | "-h" => {
                usage();
                return Ok(());
            }
            other => eprintln!("unknown argument: {other} (try --help)"),
        }
    }

    // Bind the control socket before the window exists, so a client that
    // connects to the port we print always finds the app behind it.
    let (control, token_file) = match control_addr {
        Some(addr) => {
            let opts = seacraft_gui::control::ControlOptions {
                token_file: control_token_file,
                no_auth: control_no_auth,
                explicit_token: None,
            };
            match seacraft_gui::control::spawn_listener(&addr, opts) {
                Ok(listener) => {
                    println!(
                        "seacraft-gui: control socket listening on {}",
                        listener.addr
                    );
                    #[cfg(unix)]
                    if let Some(t) = &listener.token_file {
                        signals::remove_on_signal(&t.0);
                    }
                    (Some(listener.receiver), listener.token_file)
                }
                Err(err) => {
                    eprintln!("seacraft-gui: cannot bind the control socket on {addr}: {err}");
                    std::process::exit(2);
                }
            }
        }
        None => (None, None),
    };

    let control_roots =
        seacraft_core::control_auth::ControlRoots::new(control_read_roots, control_write_roots);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([900.0, 600.0])
            .with_title(format!("SeaCraft {}", seacraft_core::VERSION)),
        ..Default::default()
    };

    eframe::run_native(
        "SeaCraft",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, open.clone());
            app.set_analysis(analysis);
            // `--combs` overlays the curves with their combs, as it always did.
            app.display_mut().curves = combs;
            app.display_mut().combs = combs;
            app.set_control_roots(control_roots.clone());
            app.set_token_file(token_file);
            if let Some(rx) = control {
                app.attach_control(rx);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Delete the control token file when the app is killed by a signal, so the
/// secret does not outlive the process. `Drop` covers a normal quit only.
#[cfg(unix)]
mod signals {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::OnceLock;

    static TOKEN_PATH: OnceLock<CString> = OnceLock::new();

    // Only async-signal-safe calls: unlink, signal, raise.
    extern "C" fn on_signal(sig: libc::c_int) {
        if let Some(path) = TOKEN_PATH.get() {
            // SAFETY: a NUL-terminated path that lives for the whole process.
            unsafe { libc::unlink(path.as_ptr()) };
        }
        // SAFETY: restore the default action and re-raise, so the process
        // still ends the way the sender asked.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }

    pub fn remove_on_signal(path: &Path) {
        let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
            return;
        };
        if TOKEN_PATH.set(c_path).is_err() {
            return;
        }
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            // SAFETY: installs a handler that only makes async-signal-safe calls.
            unsafe { libc::signal(sig, handler) };
        }
    }
}
