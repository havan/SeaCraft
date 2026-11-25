//! End-to-end i18n checks for the `seacraft` CLI binary: `--lang`, `SEACRAFT_LANG`,
//! Turkish human-readable output and generated documents, and English invariance of
//! `--help`, `--version`, `--json` and `--csv`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use seacraft_core::i18n::Locale;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("seacraft-cli-i18n-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test workdir");
        Dir(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_seacraft"));
    cmd.current_dir(dir).args(args);
    cmd.env_remove("SEACRAFT_LANG");
    for &(k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("run seacraft")
}

fn ok(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> String {
    let out = run(dir, env, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// Decode every `Tj` string written by `seacraft_core::pdf` using the PDF's own ToUnicode CMap.
fn pdf_shown_text(bytes: &[u8]) -> Vec<String> {
    fn object_bytes(bytes: &[u8], n: usize) -> &[u8] {
        let s = String::from_utf8_lossy(bytes);
        let at = s.rfind("startxref\n").expect("startxref") + 10;
        let xref: usize = s[at..].lines().next().unwrap().parse().unwrap();
        let entries = bytes[xref + 5..].iter().position(|&b| b == b'\n').unwrap() + xref + 6;
        let offset: usize = std::str::from_utf8(&bytes[entries + 20 * n..entries + 20 * n + 10])
            .unwrap()
            .parse()
            .unwrap();
        let start = bytes[offset..]
            .windows(4)
            .position(|w| w == b"obj\n")
            .unwrap()
            + offset
            + 4;
        let end = bytes[start..]
            .windows(6)
            .position(|w| w == b"endobj")
            .unwrap()
            + start;
        &bytes[start..end]
    }
    fn stream_slice(obj: &[u8]) -> Option<&[u8]> {
        let rest = obj.strip_prefix(b"<< /Length ")?;
        let sp = rest.iter().position(|&b| b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..sp]).ok()?.trim().parse().ok()?;
        let start = obj.windows(10).position(|w| w == b">>\nstream\n")? + 10;
        obj.get(start..start + len)
    }

    let s = String::from_utf8_lossy(bytes);
    let mut map = std::collections::BTreeMap::new();
    if let Some(at) = s.find("/ToUnicode ") {
        let n: usize = s[at + 11..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        if let Some(cmap) = stream_slice(object_bytes(bytes, n)) {
            for line in String::from_utf8_lossy(cmap).lines() {
                let Some((glyph, text)) = line
                    .strip_prefix('<')
                    .and_then(|l| l.split_once("> <"))
                    .and_then(|(g, t)| Some((g, t.strip_suffix('>')?)))
                else {
                    continue;
                };
                if glyph.len() != 4 || text.len() % 4 != 0 || (glyph == "0000" && text == "FFFF") {
                    continue;
                }
                let units: Vec<u16> = (0..text.len())
                    .step_by(4)
                    .map(|i| u16::from_str_radix(&text[i..i + 4], 16).unwrap())
                    .collect();
                map.insert(
                    u16::from_str_radix(glyph, 16).unwrap(),
                    String::from_utf16(&units).unwrap(),
                );
            }
        }
    }

    let size: usize = s[s.rfind("/Size ").unwrap() + 6..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let mut out = Vec::new();
    for n in 1..size {
        let obj = object_bytes(bytes, n);
        if String::from_utf8_lossy(&obj[..obj.len().min(200)]).contains("/Length1") {
            continue;
        }
        let Some(stream) = stream_slice(obj) else {
            continue;
        };
        for line in String::from_utf8_lossy(stream).lines() {
            if let (Some(a), Some(b)) = (line.find('<'), line.find("> Tj")) {
                let hex = &line[a + 1..b];
                let decoded: String = (0..hex.len())
                    .step_by(4)
                    .map(|i| map[&u16::from_str_radix(&hex[i..i + 4], 16).unwrap()].as_str())
                    .collect();
                out.push(decoded);
            }
        }
    }
    out
}

#[test]
fn cli_lang_flag_env_override_and_invariant_outputs() {
    let dir = Dir::new("precedence");
    let hull = example("demo-hull.fef");

    // --lang tr produces Turkish hydrostatics while keeping dot decimals.
    let tr_hydro = ok(
        &dir.0,
        &[],
        &["--lang", "tr", "hydro", &hull, "--kg", "2.2"],
    );
    assert!(tr_hydro.contains("su çekiminde hidrostatik"), "{tr_hydro}");
    assert!(tr_hydro.contains("hacim"), "{tr_hydro}");
    assert!(tr_hydro.contains("deplasman"), "{tr_hydro}");
    assert!(tr_hydro.contains("omurgadan"), "{tr_hydro}");
    assert!(tr_hydro.contains("11.85"), "{tr_hydro}");

    // SEACRAFT_LANG=tr works when --lang is omitted, and --lang en overrides it.
    let env_tr = ok(
        &dir.0,
        &[("SEACRAFT_LANG", "tr")],
        &["hydro", &hull, "--kg", "2.2"],
    );
    assert!(env_tr.contains("su çekiminde hidrostatik"), "{env_tr}");

    let override_en = ok(
        &dir.0,
        &[("SEACRAFT_LANG", "tr")],
        &["--lang", "en", "hydro", &hull, "--kg", "2.2"],
    );
    assert!(override_en.contains("hydrostatics at T"), "{override_en}");
    assert!(!override_en.contains("hacim"), "{override_en}");

    // --json stays English under --lang tr.
    let json = ok(
        &dir.0,
        &[],
        &["--lang", "tr", "hydro", &hull, "--kg", "2.2", "--json"],
    );
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(parsed.get("volume_m3").is_some(), "{parsed}");
    assert!(parsed.get("displacement_t").is_some(), "{parsed}");

    // --help and --version stay invariant in English even under SEACRAFT_LANG=tr.
    let ver = ok(&dir.0, &[("SEACRAFT_LANG", "tr")], &["--version"]);
    assert!(ver.starts_with("seacraft "), "{ver}");
    let help = ok(&dir.0, &[("SEACRAFT_LANG", "tr")], &["--help"]);
    assert!(help.contains("FREE!ship hull modeller"), "{help}");

    // Invalid --lang fails with a clear message.
    let bad = run(&dir.0, &[], &["--lang", "xx", "hydro", &hull]);
    assert!(!bad.status.success());
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(err.contains("unknown language: xx"), "{err}");
}

#[test]
fn cli_lang_tr_localizes_stability_examples_waves_and_documents() {
    let dir = Dir::new("docs");
    let hull = example("demo-hull.fef");

    // stability --lang tr uses Turkish disclaimer and GEÇTİ / KALDI badge.
    let stab = ok(
        &dir.0,
        &[],
        &["--lang", "tr", "stability", &hull, "--kg", "2.2"],
    );
    assert!(
        stab.contains(seacraft_core::not_validated_note(Locale::Tr)),
        "{stab}"
    );
    assert!(stab.contains("GEÇTİ") || stab.contains("KALDI"), "{stab}");

    // example --list text is Turkish, while --json remains English.
    let ex_tr = ok(&dir.0, &[], &["--lang", "tr", "example", "--list"]);
    assert!(ex_tr.contains("Wigley gövdesi (kıyaslama)"), "{ex_tr}");
    let ex_json = ok(
        &dir.0,
        &[],
        &["--lang", "tr", "example", "--list", "--json"],
    );
    let ex_rows: serde_json::Value = serde_json::from_str(&ex_json).expect("JSON");
    assert!(
        ex_rows
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["title"].as_str().unwrap().contains("Wigley hull")),
        "{ex_rows}"
    );

    // report HTML
    let report = dir.path("report-tr.html");
    let rep_out = ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "report",
            &hull,
            "-o",
            report.to_str().unwrap(),
            "--kg",
            "2.2",
        ],
    );
    assert!(rep_out.contains("yazıldı:"), "{rep_out}");
    let rep_html = std::fs::read_to_string(&report).unwrap();
    assert!(rep_html.contains("<html lang=\"tr\">"));
    assert!(rep_html.contains("Ana Boyutlar ve Özellikler"));
    assert!(rep_html.contains(seacraft_core::not_validated_note(Locale::Tr)));

    // linesplan SVG
    let lines = dir.path("lines-tr.svg");
    ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "linesplan",
            &hull,
            "-o",
            lines.to_str().unwrap(),
        ],
    );
    let lines_svg = std::fs::read_to_string(&lines).unwrap();
    assert!(lines_svg.contains("Posta planı - postalar"), "{lines_svg}");

    // offsets HTML + PDF
    let off_html = dir.path("offsets-tr.html");
    let off_pdf = dir.path("offsets-tr.pdf");
    ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "offsets",
            &hull,
            "--level",
            "2",
            "--html",
            off_html.to_str().unwrap(),
            "--pdf",
            off_pdf.to_str().unwrap(),
        ],
    );
    let off_html_s = std::fs::read_to_string(&off_html).unwrap();
    assert!(off_html_s.contains("<html lang=\"tr\">"));
    assert!(off_html_s.contains("ofset tablosu"));
    let off_pdf_bytes = std::fs::read(&off_pdf).unwrap();
    let off_pdf_text = pdf_shown_text(&off_pdf_bytes);
    let off_pdf_joined = off_pdf_text.join(" ");
    assert!(off_pdf_joined.contains("ofset tablosu"), "{off_pdf_joined}");
    assert!(off_pdf_joined.contains("Posta"), "{off_pdf_joined}");

    // table HTML
    let tbl_html = dir.path("table-tr.html");
    ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "table",
            &hull,
            "--level",
            "2",
            "--html",
            tbl_html.to_str().unwrap(),
        ],
    );
    let tbl_s = std::fs::read_to_string(&tbl_html).unwrap();
    assert!(tbl_s.contains("<html lang=\"tr\">"), "{tbl_s}");
    assert!(tbl_s.contains("Hidrostatik - "), "{tbl_s}");
    assert!(tbl_s.contains("Yoğunluk "), "{tbl_s}");

    // waves SVG + stdout regime sentence
    let wave_svg = dir.path("waves-tr.svg");
    let wave_out = ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "waves",
            &hull,
            "--fn",
            "0.45",
            "--level",
            "1",
            "-o",
            wave_svg.to_str().unwrap(),
        ],
    );
    assert!(wave_out.contains("Tekne hızı civarında"), "{wave_out}");
    let wave_svg_s = std::fs::read_to_string(&wave_svg).unwrap();
    assert!(wave_svg_s.contains("hızda dalga deseni"), "{wave_svg_s}");

    // templates SVG
    let tpl_svg = dir.path("templates-tr.svg");
    let tpl_out = ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "templates",
            &hull,
            "--level",
            "1",
            "--scale",
            "1:10",
            "-o",
            tpl_svg.to_str().unwrap(),
        ],
    );
    assert!(tpl_out.contains("pafta"), "{tpl_out}");
    let tpl_svg_s = std::fs::read_to_string(&tpl_svg).unwrap();
    assert!(tpl_svg_s.contains("Posta 1"), "{tpl_svg_s}");

    // panels SVG on pram fixture
    let mut pram = seacraft_core::panels::pram_fixture(0.0, 0.0);
    for layer in &mut pram.layers {
        layer.developable = true;
    }
    let pram_path = dir.path("pram.fef");
    std::fs::write(&pram_path, seacraft_core::fef::write(&pram)).unwrap();
    let panels_svg = dir.path("panels-tr.svg");
    let panels_out = ok(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "panels",
            pram_path.to_str().unwrap(),
            "--level",
            "1",
            "--scale",
            "1:10",
            "-o",
            panels_svg.to_str().unwrap(),
        ],
    );
    assert!(panels_out.contains("kontrplak plaka"), "{panels_out}");
    let panels_svg_s = std::fs::read_to_string(&panels_svg).unwrap();
    assert!(
        panels_svg_s.contains("Modellenen yüzey üzerinde açındırılmıştır"),
        "{panels_svg_s}"
    );
}

/// The method-validity warnings and wave notes, the caveats a reader most needs, are Turkish
/// under `--lang tr` on the terminal and in the wave SVG (demo hull: L/B about 3, a wide transom,
/// Fn 0.15 below Michell's range).
#[test]
fn cli_lang_tr_localizes_method_warnings_and_wave_notes() {
    let dir = Dir::new("warnings");
    let hull = example("demo-hull.fef");
    let svg = dir.path("w.svg");
    let out = run(
        &dir.0,
        &[],
        &[
            "--lang",
            "tr",
            "waves",
            &hull,
            "--fn",
            "0.15",
            "-o",
            svg.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = format!(
        "{}{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        std::fs::read_to_string(&svg).unwrap()
    );
    for english in [
        "is outside",
        "wetted transom",
        "waves shorter than",
        "Far-field thin-ship",
        "applicability range",
    ] {
        assert!(
            !text.contains(english),
            "English `{english}` under --lang tr:\n{text}"
        );
    }
    assert!(text.contains("ıslak ayna alanı"), "{text}");
    assert!(text.contains("kısa dalgalar"), "{text}");

    let res = ok(&dir.0, &[], &["--lang", "tr", "resistance", &hull]);
    assert!(!res.contains("is outside"), "{res}");
    assert!(res.contains("aralığının dışında"), "{res}");
}

/// `--lang` takes exactly a supported code; a bad `SEACRAFT_LANG` is reported, not silently
/// ignored; `LC_ALL=C` forces English over a Turkish `LANG`.
#[test]
fn cli_locale_inputs_are_checked() {
    let dir = Dir::new("locale-inputs");
    let hull = example("demo-hull.fef");
    for bad in ["tr_TR.UTF-8", "EN-zz", "de"] {
        let out = run(&dir.0, &[], &["--lang", bad, "info", &hull]);
        assert!(!out.status.success(), "--lang {bad} accepted");
    }
    let out = run(
        &dir.0,
        &[("SEACRAFT_LANG", "de"), ("LC_ALL", "C")],
        &["info", &hull],
    );
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("SEACRAFT_LANG=de"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let c = ok(
        &dir.0,
        &[("LC_ALL", "C"), ("LANG", "tr_TR.UTF-8")],
        &["hydro", &hull, "--draft", "0.62"],
    );
    let en = ok(
        &dir.0,
        &[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")],
        &["hydro", &hull, "--draft", "0.62"],
    );
    assert_eq!(c, en, "LC_ALL=C must give English");
}
