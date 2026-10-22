use std::path::PathBuf;

use eframe::egui;
use seacraft_gui::App;

fn usage() {
    println!(
        "seacraft-gui {}\n\n\
         Usage: seacraft-gui [--open FILE.fef] [--analysis MODE] [--combs]\n\n\
         Options:\n  \
         --open FILE       Load a FREE!ship .fef model at startup\n  \
         --analysis MODE   Shading: shaded, zebra, curvature or draft\n  \
         --combs           Overlay station/buttock/waterline curvature combs\n  \
         --version         Print the version and exit\n  \
         --help            Print this help and exit",
        seacraft_core::VERSION
    );
}

fn main() -> eframe::Result {
    let mut open: Option<PathBuf> = None;
    let mut analysis = seacraft_core::AnalysisMode::Shaded;
    let mut combs = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => open = args.next().map(PathBuf::from),
            "--combs" => combs = true,
            "--analysis" => {
                let value = args.next().unwrap_or_default();
                analysis = match value.as_str() {
                    "shaded" => seacraft_core::AnalysisMode::Shaded,
                    "zebra" => seacraft_core::AnalysisMode::Zebra { stripes: 14.0 },
                    "curvature" => seacraft_core::AnalysisMode::Curvature { radius: 1.0 },
                    "draft" => seacraft_core::AnalysisMode::Draft { angle_deg: 3.0 },
                    other => {
                        eprintln!("unknown analysis mode: {other} (shaded, zebra, curvature, draft)");
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
            app.display_mut().combs = combs;
            Ok(Box::new(app))
        }),
    )
}
