use std::path::PathBuf;

use eframe::egui;
use seacraft_gui::App;

fn usage() {
    println!(
        "seacraft-gui {}\n\n\
         Usage: seacraft-gui [--open FILE.fef]\n\n\
         Options:\n  \
         --open FILE   Load a FREE!ship .fef model at startup\n  \
         --version     Print the version and exit\n  \
         --help        Print this help and exit",
        seacraft_core::VERSION
    );
}

fn main() -> eframe::Result {
    let mut open: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => open = args.next().map(PathBuf::from),
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
        Box::new(move |cc| Ok(Box::new(App::new(cc, open.clone())))),
    )
}
