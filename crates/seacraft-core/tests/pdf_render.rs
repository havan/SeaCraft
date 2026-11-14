//! The PDF writer against poppler, an independent PDF implementation. Ignored by default because
//! it needs `pdftoppm` and `pdftotext` on the PATH; run it with
//! `cargo test -p seacraft-core --test pdf_render -- --ignored --nocapture`.

use std::path::Path;
use std::process::Command;

use seacraft_core::pdf::{Anchor, Content, Pdf, Stroke, PT_PER_MM};

/// A binary PGM's width, height and pixels.
fn read_pgm(path: &Path) -> (usize, usize, Vec<u8>) {
    let data = std::fs::read(path).unwrap();
    // "P5", width, height and the largest value, each ended by white space, then the pixels.
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        let start = at;
        while !data[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&data[start..at]).into_owned());
        at += 1;
    }
    assert_eq!(fields[0], "P5");
    assert_eq!(fields[3], "255");
    let (w, h): (usize, usize) = (fields[1].parse().unwrap(), fields[2].parse().unwrap());
    (w, h, data[at..at + w * h].to_vec())
}

/// Renders with poppler, an independent implementation, at 254 dpi so that 1 px = 0.1 mm.
/// A4 page; a 0.35 mm line from (50, 150) mm to (150, 150) mm; "Kayık Şile" at (50, 200) mm, 12 pt.
/// `pdftoppm -r 254 -gray -f 1 -l 1` gives a binary PGM, 2100 x 2970 or a pixel more. Rows are counted from the
/// top, so the line is on row 1470: over rows 1468..=1472 the largest count of pixels darker than
/// 128 must be 1000 +/- 2. `pdftotext FILE -` must contain "Kayık Şile".
/// Returns early with a message on stderr when `pdftoppm` is not on PATH.
#[test]
#[ignore = "needs poppler (pdftoppm, pdftotext)"]
fn poppler_draws_100_mm_as_1000_pixels_and_reads_turkish_text() {
    if Command::new("pdftoppm").arg("-v").output().is_err() {
        eprintln!("pdftoppm is not on PATH: nothing was checked");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let file = dir.join("pdf_render.pdf");
    let mut c = Content::new();
    let mm = PT_PER_MM;
    let line = Stroke {
        width_mm: 0.35,
        grey: 0.0,
        dash_mm: None,
    };
    c.polyline(
        &[(50.0 * mm, 150.0 * mm), (150.0 * mm, 150.0 * mm)],
        false,
        line,
    );
    c.text(
        (50.0 * mm, 200.0 * mm),
        12.0,
        0.0,
        Anchor::Start,
        0.0,
        "Kayık Şile",
    );
    let mut pdf = Pdf::new("Render check");
    pdf.add_page(210.0 * mm, 297.0 * mm, c);
    std::fs::write(&file, pdf.finish().unwrap().bytes).unwrap();

    let prefix = dir.join("pdf_render");
    let out = Command::new("pdftoppm")
        .args(["-r", "254", "-gray", "-f", "1", "-l", "1", "-singlefile"])
        .arg(&file)
        .arg(&prefix)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (w, h, px) = read_pgm(&prefix.with_extension("pgm"));
    // The page is 2100.001 x 2970.002 px, three decimals of a point being what they are, and
    // poppler rounds a page up to whole pixels.
    assert!(
        (2100..=2101).contains(&w) && (2970..=2971).contains(&h),
        "{w} x {h}"
    );
    let dark = |row: usize| {
        px[row * w..(row + 1) * w]
            .iter()
            .filter(|p| **p < 128)
            .count()
    };
    let longest = (1468..=1472).map(dark).max().unwrap();
    assert!(
        (998..=1002).contains(&longest),
        "{longest} px, expected 1000"
    );
    // The line starts 50 mm from the left edge: pixel 500.
    let first = px[1470 * w..1471 * w]
        .iter()
        .position(|p| *p < 128)
        .unwrap();
    assert!((499..=501).contains(&first), "{first}");
    // Nothing is drawn well above or below the line.
    assert_eq!(dark(1460) + dark(1480), 0);

    let out = Command::new("pdftotext")
        .arg(&file)
        .arg("-")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let words = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(words.contains("Kayık Şile"), "{words:?}");
    eprintln!(
        "poppler: 100 mm drew as {longest} px at 254 dpi; text read back as {:?}",
        words.trim()
    );
}
