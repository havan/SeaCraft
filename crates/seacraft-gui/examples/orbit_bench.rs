//! Real frame times while orbiting, in a real window on the real GPU, with
//! vsync off so a frame's cost is not hidden by the monitor's refresh:
//!
//! ```console
//! cargo run --release -p seacraft-gui --example orbit_bench -- examples/container-230m.fef 3
//! ```
//!
//! It opens a window, orbits the perspective view for `FRAMES` frames, prints
//! one line and closes. Nothing is saved: no recent list, no autosave.

use std::time::Instant;

use eframe::egui;
use seacraft_gui::camera::ViewKind;
use seacraft_gui::App;

const SETTLE: usize = 30;
const FRAMES: usize = 300;

struct Bench {
    app: App,
    label: String,
    frame: usize,
    last: Option<Instant>,
    period: Vec<f64>,
    cpu: Vec<f64>,
}

fn percentile(values: &mut [f64], p: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * p).round() as usize]
}

impl eframe::App for Bench {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let now = Instant::now();
        if self.frame > SETTLE {
            if let Some(last) = self.last {
                self.period.push((now - last).as_secs_f64() * 1e3);
            }
            // CPU time of the previous frame: UI, tessellation and GPU submit.
            if let Some(cpu) = frame.info().cpu_usage {
                self.cpu.push(f64::from(cpu) * 1e3);
            }
        }
        self.last = Some(now);
        self.frame += 1;
        if self.frame == SETTLE {
            self.app.recompute_hydrostatics();
        }
        if self.frame > SETTLE {
            let index = self
                .app
                .cameras()
                .iter()
                .position(|c| c.kind == ViewKind::Perspective)
                .expect("a perspective view");
            self.app.cameras_mut()[index].orbit(3.0, 1.0);
        }
        self.app.draw(ui);
        if self.period.len() >= FRAMES {
            let adapter = frame
                .wgpu_render_state()
                .map(|s| s.adapter.get_info())
                .map(|i| format!("{} {:?}", i.name, i.backend))
                .unwrap_or_default();
            println!(
                "{:<22} p10 {:>6.1}  median {:>6.1}  p95 {:>6.1} ms ({:>5.0} fps)  cpu median {:>6.1} ms  [{adapter}]",
                self.label,
                percentile(&mut self.period, 0.1),
                percentile(&mut self.period, 0.5),
                percentile(&mut self.period, 0.95),
                1e3 / percentile(&mut self.period, 0.5),
                percentile(&mut self.cpu, 0.5),
            );
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint();
    }
}

fn main() -> eframe::Result {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: orbit_bench HULL.fef [LEVEL]");
    let level: u8 = args.next().map_or(3, |l| l.parse().expect("a level"));
    let label = format!(
        "{} L{level}",
        std::path::Path::new(&path)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    );
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1600.0, 1000.0]),
        ..Default::default()
    };
    options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    eframe::run_native(
        "SeaCraft orbit bench",
        options,
        Box::new(move |cc| {
            seacraft_gui::theme::apply(&cc.egui_ctx);
            let mut app = App::headless_for_test(Some(&path));
            app.set_subdivision_level(level);
            Ok(Box::new(Bench {
                app,
                label,
                frame: 0,
                last: None,
                period: Vec::new(),
                cpu: Vec::new(),
            }))
        }),
    )
}
