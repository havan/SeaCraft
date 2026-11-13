//! A small PDF writer: pages at true size, drawings reused across pages, stroked lines, and
//! Unicode text in an embedded subset of DejaVu Sans.
//!
//! It exists so that patterns and tables can be printed without a dependency: PDF 1.4, streams
//! not compressed, a classic cross-reference table. The file carries no date and no random id, so
//! the same input gives the same bytes.
//!
//! Lengths here are PDF points, 1/72 inch; [`PT_PER_MM`] converts. Coordinates are written to
//! three decimals, about a third of a micrometre.
//!
//! Text is a Type 0 font over a CIDFontType2 with the `Identity-H` encoding, so a string is the
//! font's own glyph ids, and a `ToUnicode` map lets a reader search and copy it. A character
//! DejaVu Sans has no glyph for is drawn as `?` and reported in [`Document::missing`]; it never
//! fails the document.
//!
//! Written from the PDF 1.4 reference (file structure, content stream operators, composite
//! fonts); nothing here is ported.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::OnceLock;

use crate::ttf::Font;

/// Points in one millimetre: a point is 1/72 inch and an inch is 25.4 mm, both by definition.
pub const PT_PER_MM: f64 = 72.0 / 25.4;

/// The longest side a page may have, 200 inches (5.08 m): the limit PDF readers implement.
pub const MAX_PAGE_PT: f64 = 14_400.0;

/// The name the embedded subset goes by. The six letter tag marks a subset.
const FONT_NAME: &str = "AAAAAA+DejaVuSans";

/// Which point of a line of text sits at the position given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// The start of the text: it runs on from the position.
    Start,
    /// The middle of the text.
    Middle,
    /// The end of the text: it runs up to the position.
    End,
}

/// How a line is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// Width on paper, in millimetres.
    pub width_mm: f64,
    /// 0 is black, 1 is white.
    pub grey: f64,
    /// Dashes: millimetres on, millimetres off. `None` is a solid line.
    pub dash_mm: Option<(f64, f64)>,
}

/// A drawing added with [`Pdf::add_form`], to place on pages with [`Content::form`].
#[derive(Clone, Copy, Debug)]
pub struct FormId(usize);

/// The embedded font, parsed once. It is compiled in and its tests read it, so it parses.
fn font() -> &'static Font<'static> {
    static FONT: OnceLock<Font<'static>> = OnceLock::new();
    FONT.get_or_init(|| Font::parse(crate::fonts::DEJAVU_SANS).expect("the embedded font parses"))
}

/// The glyph that draws `c`, or `None` when the font has none.
fn glyph_for(c: char) -> Option<u16> {
    font().glyph(c)
}

/// The glyph drawn in place of a character the font lacks.
fn question_mark() -> u16 {
    glyph_for('?').unwrap_or(0)
}

/// A number as PDF writes it here: three decimals, and never a negative zero.
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_owned()
    } else {
        s
    }
}

/// A rotation's cosine or sine: an exact `0`, `1` or `-1` where it is one, else six decimals.
fn turn(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 {
        "0".to_owned()
    } else if r == 1.0 {
        "1".to_owned()
    } else if r == -1.0 {
        "-1".to_owned()
    } else {
        format!("{r:.6}")
    }
}

/// The width of one line of `text` at `size_pt`, in points. Characters the font lacks count as
/// the `?` that is drawn for them.
pub fn text_width_pt(text: &str, size_pt: f64) -> f64 {
    let f = font();
    let units: u64 = text
        .chars()
        .map(|c| f.advance(glyph_for(c).unwrap_or_else(question_mark)) as u64)
        .sum();
    units as f64 / f.metrics().units_per_em as f64 * size_pt
}

/// `text` broken into lines no wider than `width_pt` at `size_pt`, at spaces. A word wider than
/// the line gets a line to itself. An empty text is no lines.
pub fn wrap_text(text: &str, size_pt: f64, width_pt: f64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if text_width_pt(&format!("{line} {word}"), size_pt) <= width_pt => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

/// What goes on a page or in a reusable drawing: lines, text and placed drawings, in the order
/// they are added. Coordinates are points, x to the right and y up, from the lower-left corner.
#[derive(Default, Clone, Debug)]
pub struct Content {
    ops: String,
    /// Glyphs shown, each with the character it stands for.
    glyphs: BTreeMap<u16, char>,
    missing: BTreeSet<char>,
    forms: BTreeSet<usize>,
}

impl Content {
    /// Empty content.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember the graphics state (the clip and the transform) until [`Content::restore`].
    pub fn save(&mut self) {
        self.ops.push_str("q\n");
    }

    /// Return to the state at the matching [`Content::save`].
    pub fn restore(&mut self) {
        self.ops.push_str("Q\n");
    }

    /// Move everything drawn from here on by `(dx_pt, dy_pt)`.
    pub fn translate(&mut self, dx_pt: f64, dy_pt: f64) {
        let _ = writeln!(self.ops, "1 0 0 1 {} {} cm", num(dx_pt), num(dy_pt));
    }

    /// Draw only inside this rectangle from here on.
    pub fn clip_rect(&mut self, x_pt: f64, y_pt: f64, w_pt: f64, h_pt: f64) {
        let _ = writeln!(
            self.ops,
            "{} {} {} {} re W n",
            num(x_pt),
            num(y_pt),
            num(w_pt),
            num(h_pt)
        );
    }

    /// A stroked line through `points_pt`, joined back to its start when `closed`. Fewer than two
    /// points, or a point that is not a finite number, draws nothing.
    pub fn polyline(&mut self, points_pt: &[(f64, f64)], closed: bool, stroke: Stroke) {
        if points_pt.len() < 2
            || points_pt
                .iter()
                .any(|p| !p.0.is_finite() || !p.1.is_finite())
        {
            return;
        }
        let dash = match stroke.dash_mm {
            Some((on, off)) if on > 0.0 && off > 0.0 => {
                format!("[{} {}]", num(on * PT_PER_MM), num(off * PT_PER_MM))
            }
            _ => "[]".to_owned(),
        };
        // Round joins, so a line of many short segments has no spikes; butt ends, so a line is
        // exactly as long as its end points say.
        let _ = writeln!(
            self.ops,
            "{} w {} G {dash} 0 d 1 j 0 J",
            num(stroke.width_mm.max(0.0) * PT_PER_MM),
            num(stroke.grey.clamp(0.0, 1.0)),
        );
        for (i, (x, y)) in points_pt.iter().enumerate() {
            let op = if i == 0 { 'm' } else { 'l' };
            let _ = writeln!(self.ops, "{} {} {op}", num(*x), num(*y));
        }
        if closed {
            self.ops.push_str("h\n");
        }
        self.ops.push_str("S\n");
    }

    /// One line of text. `grey` 0 is black. `angle_deg` turns it anticlockwise about its anchor.
    /// Characters without a glyph are drawn as `?` and recorded.
    pub fn text(
        &mut self,
        at_pt: (f64, f64),
        size_pt: f64,
        angle_deg: f64,
        anchor: Anchor,
        grey: f64,
        text: &str,
    ) {
        let finite = [at_pt.0, at_pt.1, size_pt, angle_deg]
            .iter()
            .all(|v| v.is_finite());
        if text.is_empty() || !finite || size_pt <= 0.0 {
            return;
        }
        let mut hex = String::with_capacity(4 * text.len());
        for c in text.chars() {
            let glyph = match glyph_for(c) {
                Some(g) => {
                    self.glyphs.entry(g).or_insert(c);
                    g
                }
                None => {
                    self.missing.insert(c);
                    self.glyphs.entry(question_mark()).or_insert('?');
                    question_mark()
                }
            };
            let _ = write!(hex, "{glyph:04X}");
        }
        let back = match anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => text_width_pt(text, size_pt) / 2.0,
            Anchor::End => text_width_pt(text, size_pt),
        };
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let (x, y) = (at_pt.0 - back * cos, at_pt.1 - back * sin);
        let _ = writeln!(
            self.ops,
            "BT /F1 {} Tf {} g {} {} {} {} {} {} Tm <{hex}> Tj ET",
            num(size_pt),
            num(grey.clamp(0.0, 1.0)),
            turn(cos),
            turn(sin),
            turn(-sin),
            turn(cos),
            num(x),
            num(y),
        );
    }

    /// Add everything in `other` after what is here, as if it had been drawn here.
    pub fn append(&mut self, other: &Content) {
        self.ops.push_str(&other.ops);
        for (glyph, c) in &other.glyphs {
            self.glyphs.entry(*glyph).or_insert(*c);
        }
        self.missing.extend(&other.missing);
        self.forms.extend(&other.forms);
    }

    /// Place a drawing added with [`Pdf::add_form`], under the current transform and clip.
    pub fn form(&mut self, id: FormId) {
        self.forms.insert(id.0);
        let _ = writeln!(self.ops, "/X{} Do", id.0);
    }

    /// The `/Resources` dictionary this content needs, given where the font and forms are.
    fn resources(&self, font_object: usize, form_object: impl Fn(usize) -> usize) -> String {
        let mut out = String::from("<<");
        if !self.glyphs.is_empty() {
            let _ = write!(out, " /Font << /F1 {font_object} 0 R >>");
        }
        if !self.forms.is_empty() {
            out.push_str(" /XObject <<");
            for form in &self.forms {
                let _ = write!(out, " /X{form} {} 0 R", form_object(*form));
            }
            out.push_str(" >>");
        }
        out.push_str(" >>");
        out
    }
}

/// A PDF document being put together.
#[derive(Debug, Clone)]
pub struct Pdf {
    title: String,
    forms: Vec<([f64; 4], Content)>,
    pages: Vec<(f64, f64, Content)>,
}

/// A finished PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The file.
    pub bytes: Vec<u8>,
    /// How many pages it has.
    pub pages: usize,
    /// Characters the font has no glyph for, each drawn as `?`: sorted, no repeats.
    pub missing: Vec<char>,
}

/// The file being written: object bodies appended in number order, with where each began.
struct Objects {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
}

impl Objects {
    /// Append the next object; its number is the count so far plus one.
    fn add(&mut self, body: &[u8]) {
        self.offsets.push(self.bytes.len());
        let header = format!("{} 0 obj\n", self.offsets.len());
        self.bytes.extend(header.as_bytes());
        self.bytes.extend(body);
        self.bytes.extend(b"\nendobj\n");
    }

    /// Append a stream object with these extra dictionary entries.
    fn add_stream(&mut self, entries: &str, data: &[u8]) {
        let mut body = format!("<< /Length {}{entries} >>\nstream\n", data.len()).into_bytes();
        body.extend(data);
        body.extend(b"\nendstream");
        self.add(&body);
    }
}

impl Pdf {
    /// An empty document. The title is what a reader shows in its window; any text will do.
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            forms: Vec::new(),
            pages: Vec::new(),
        }
    }

    /// Add a drawing that pages (and later drawings) can place any number of times; the file
    /// holds it once. `bbox_pt` is `[x min, y min, x max, y max]` in the drawing's coordinates:
    /// nothing outside it is drawn.
    pub fn add_form(&mut self, bbox_pt: [f64; 4], content: Content) -> FormId {
        self.forms.push((bbox_pt, content));
        FormId(self.forms.len() - 1)
    }

    /// Add a page of this size, in points.
    pub fn add_page(&mut self, width_pt: f64, height_pt: f64, content: Content) {
        self.pages.push((width_pt, height_pt, content));
    }

    /// How many pages have been added.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Write the file. Fails when the document has no page, when a page has no size or is larger
    /// than a PDF page can be, or when a content names a drawing this document does not have.
    pub fn finish(self) -> Result<Document, String> {
        if self.pages.is_empty() {
            return Err("a PDF needs at least one page".into());
        }
        for (i, (w, h, _)) in self.pages.iter().enumerate() {
            let page = i + 1;
            if !(w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0) {
                return Err(format!("page {page} has no size ({w} x {h} pt)"));
            }
            if *w > MAX_PAGE_PT || *h > MAX_PAGE_PT {
                return Err(format!(
                    "page {page} would be {:.2} x {:.2} m, and a PDF page can be at most 5.08 m \
                     (200 in) on a side",
                    w / PT_PER_MM / 1000.0,
                    h / PT_PER_MM / 1000.0
                ));
            }
        }
        let contents = || {
            self.forms
                .iter()
                .map(|f| &f.1)
                .chain(self.pages.iter().map(|p| &p.2))
        };
        for c in contents() {
            if let Some(form) = c.forms.iter().find(|f| **f >= self.forms.len()) {
                return Err(format!(
                    "a page places drawing {form}, which is not in this document"
                ));
            }
        }
        if self
            .forms
            .iter()
            .any(|(bbox, _)| bbox.iter().any(|v| !v.is_finite()))
        {
            return Err("a drawing's bounds are not finite numbers".into());
        }
        let mut glyphs: BTreeMap<u16, char> = BTreeMap::new();
        let mut missing: BTreeSet<char> = BTreeSet::new();
        for c in contents() {
            for (g, ch) in &c.glyphs {
                glyphs.entry(*g).or_insert(*ch);
            }
            missing.extend(&c.missing);
        }

        // Object numbers: 1 catalog, 2 page tree, 3 information, then five for the font when
        // there is text, then the forms, then two for each page.
        const FONT: usize = 4;
        let font_objects = if glyphs.is_empty() { 0 } else { 5 };
        let first_form = 4 + font_objects;
        let first_page = first_form + self.forms.len();
        let form_object = |form: usize| first_form + form;

        let mut file = Objects {
            // The second line is a comment of bytes over 127, which tells a program that moves
            // the file that it is not text.
            bytes: b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offsets: Vec::new(),
        };
        file.add(b"<< /Type /Catalog /Pages 2 0 R >>");
        let mut kids = String::new();
        for i in 0..self.pages.len() {
            let _ = write!(kids, "{} 0 R ", first_page + 2 * i);
        }
        let tree = format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.trim_end(),
            self.pages.len()
        );
        file.add(tree.as_bytes());
        let mut title = String::from("FEFF");
        for unit in self.title.encode_utf16() {
            let _ = write!(title, "{unit:04X}");
        }
        let info = format!(
            "<< /Title <{title}> /Producer (SeaCraft {}) >>",
            crate::VERSION
        );
        file.add(info.as_bytes());

        if !glyphs.is_empty() {
            Self::write_font(&mut file, &glyphs)?;
        }
        for (bbox, content) in &self.forms {
            let entries = format!(
                " /Type /XObject /Subtype /Form /BBox [{} {} {} {}] /Resources {}",
                num(bbox[0]),
                num(bbox[1]),
                num(bbox[2]),
                num(bbox[3]),
                content.resources(FONT, form_object)
            );
            file.add_stream(&entries, content.ops.as_bytes());
        }
        for (i, (w, h, content)) in self.pages.iter().enumerate() {
            let page = format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources {} /Contents {} 0 R >>",
                num(*w),
                num(*h),
                content.resources(FONT, form_object),
                first_page + 2 * i + 1
            );
            file.add(page.as_bytes());
            file.add_stream("", content.ops.as_bytes());
        }

        let Objects { mut bytes, offsets } = file;
        let xref = bytes.len();
        let size = offsets.len() + 1;
        bytes.extend(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for offset in &offsets {
            bytes.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend(
            format!(
                "trailer\n<< /Size {size} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        Ok(Document {
            bytes,
            pages: self.pages.len(),
            missing: missing.into_iter().collect(),
        })
    }

    /// The five font objects, numbers 4 to 8: the Type 0 font, its CID font, the descriptor, the
    /// subset file and the map back to Unicode.
    fn write_font(file: &mut Objects, glyphs: &BTreeMap<u16, char>) -> Result<(), String> {
        let f = font();
        let m = f.metrics();
        // Font units to the thousandths of an em PDF measures glyphs in.
        let milli = |units: f64| num(units * 1000.0 / m.units_per_em as f64);
        file.add(
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{FONT_NAME} /Encoding /Identity-H \
                 /DescendantFonts [5 0 R] /ToUnicode 8 0 R >>"
            )
            .as_bytes(),
        );
        let mut widths = String::new();
        for glyph in glyphs.keys() {
            let _ = write!(widths, "{glyph} [{}] ", milli(f.advance(*glyph) as f64));
        }
        file.add(
            format!(
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{FONT_NAME} /CIDSystemInfo \
                 << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 6 0 R \
                 /CIDToGIDMap /Identity /DW 1000 /W [{}] >>",
                widths.trim_end()
            )
            .as_bytes(),
        );
        file.add(
            format!(
                "<< /Type /FontDescriptor /FontName /{FONT_NAME} /Flags 4 /FontBBox [{} {} {} {}] \
                 /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 7 0 R >>",
                milli(m.bbox[0] as f64),
                milli(m.bbox[1] as f64),
                milli(m.bbox[2] as f64),
                milli(m.bbox[3] as f64),
                milli(m.ascent as f64),
                milli(m.descent as f64),
                milli(m.cap_height as f64),
            )
            .as_bytes(),
        );
        let subset = f.subset(&glyphs.keys().copied().collect())?;
        file.add_stream(&format!(" /Length1 {}", subset.len()), &subset);

        let mut map = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let pairs: Vec<(&u16, &char)> = glyphs.iter().collect();
        // A block holds at most 100 lines.
        for block in pairs.chunks(100) {
            let _ = writeln!(map, "{} beginbfchar", block.len());
            for (glyph, c) in block {
                let _ = write!(map, "<{glyph:04X}> <");
                for unit in c.encode_utf16(&mut [0; 2]) {
                    let _ = write!(map, "{unit:04X}");
                }
                map.push_str(">\n");
            }
            map.push_str("endbfchar\n");
        }
        map.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend");
        file.add_stream("", map.as_bytes());
        Ok(())
    }
}

/// Test support: read back what the writer wrote, through the file's own tables.
#[cfg(test)]
mod reading {
    use std::collections::BTreeMap;

    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        hay.get(from..)?
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|i| i + from)
    }

    fn lossy(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Object `n`'s bytes, from after `n 0 obj` to `endobj`, found through the xref table.
    fn object(bytes: &[u8], n: usize) -> &[u8] {
        let s = lossy(bytes);
        let at = s.rfind("startxref\n").expect("startxref") + 10;
        let xref: usize = s[at..].lines().next().unwrap().parse().unwrap();
        let entries = find(bytes, b"\n", xref + 5).unwrap() + 1;
        let entry = &bytes[entries + 20 * n..entries + 20 * n + 10];
        let offset: usize = lossy(entry).parse().unwrap();
        let start = find(bytes, b"obj\n", offset).unwrap() + 4;
        // A stream's length says where it ends; binary data may hold the word "endobj".
        let end = match stream_range(&bytes[start..]) {
            Some((_, end)) => find(bytes, b"endobj", start + end).unwrap(),
            None => find(bytes, b"endobj", start).unwrap(),
        };
        &bytes[start..end]
    }

    /// Where an object's stream data starts and ends, when it has one.
    fn stream_range(object: &[u8]) -> Option<(usize, usize)> {
        if !object.starts_with(b"<< /Length ") {
            return None;
        }
        let len: usize = lossy(&object[11..find(object, b" ", 11)?])
            .trim()
            .parse()
            .ok()?;
        let start = find(object, b">>\nstream\n", 0)? + 10;
        Some((start, start + len))
    }

    fn stream(bytes: &[u8], n: usize) -> Vec<u8> {
        let obj = object(bytes, n);
        let (start, end) = stream_range(obj).expect("the object is a stream");
        obj[start..end].to_vec()
    }

    /// The number before ` 0 R` that follows `key` in an object.
    fn reference(object: &[u8], key: &str) -> usize {
        let s = lossy(object);
        let rest = &s[s.find(key).unwrap_or_else(|| panic!("{key} in {s}")) + key.len()..];
        rest.split_whitespace().next().unwrap().parse().unwrap()
    }

    /// Glyph id to text, from the file's ToUnicode map; empty when the file embeds no font.
    fn to_unicode(bytes: &[u8]) -> BTreeMap<u16, String> {
        let mut map = BTreeMap::new();
        let s = lossy(bytes);
        let Some(at) = s.find("/ToUnicode ") else {
            return map;
        };
        let n: usize = s[at + 11..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        for line in lossy(&stream(bytes, n)).lines() {
            let Some((glyph, text)) = line.strip_prefix('<').and_then(|l| l.split_once("> <"))
            else {
                continue;
            };
            let Some(text) = text.strip_suffix('>') else {
                continue;
            };
            if glyph.len() != 4 || text.len() % 4 != 0 || glyph == "0000" && text == "FFFF" {
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
        map
    }

    /// The strings one content stream shows, decoded through `map`.
    fn strings(content: &[u8], map: &BTreeMap<u16, String>) -> Vec<String> {
        lossy(content)
            .lines()
            .filter_map(|line| {
                let hex = &line[line.find('<')? + 1..line.find("> Tj")?];
                Some(
                    (0..hex.len())
                        .step_by(4)
                        .map(|i| map[&u16::from_str_radix(&hex[i..i + 4], 16).unwrap()].as_str())
                        .collect(),
                )
            })
            .collect()
    }

    /// Every string shown with `Tj` anywhere in the file (forms and pages), decoded through the
    /// file's own ToUnicode map, in file order.
    pub(crate) fn shown_text(bytes: &[u8]) -> Vec<String> {
        let map = to_unicode(bytes);
        let s = lossy(bytes);
        let size: usize = s[s.rfind("/Size ").unwrap() + 6..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        (1..size)
            .filter_map(|n| {
                let obj = object(bytes, n);
                // The font file is a stream too, but binary: only content streams show text.
                let is_content = !lossy(&obj[..obj.len().min(200)]).contains("/Length1");
                stream_range(obj)
                    .filter(|_| is_content)
                    .map(|(a, b)| strings(&obj[a..b], &map))
            })
            .flatten()
            .collect()
    }

    /// One page's own content stream, for comparing pages between two documents. `page` counts
    /// from 0.
    pub(crate) fn page_stream(bytes: &[u8], page: usize) -> Vec<u8> {
        let tree = lossy(object(bytes, 2));
        let kids = &tree[tree.find('[').unwrap() + 1..tree.find(']').unwrap()];
        let n: usize = kids
            .split(" 0 R")
            .nth(page)
            .unwrap_or_else(|| panic!("page {page} of {kids}"))
            .trim()
            .parse()
            .unwrap();
        stream(bytes, reference(object(bytes, n), "/Contents "))
    }

    /// The strings shown by one page's own content stream (not its forms). `page` counts from 0.
    pub(crate) fn page_text(bytes: &[u8], page: usize) -> Vec<String> {
        strings(&page_stream(bytes, page), &to_unicode(bytes))
    }
}

#[cfg(test)]
pub(crate) use reading::{page_stream, page_text, shown_text};

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Stroke = Stroke {
        width_mm: 0.35,
        grey: 0.0,
        dash_mm: None,
    };

    fn a4() -> (f64, f64) {
        (210.0 * PT_PER_MM, 297.0 * PT_PER_MM)
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    fn count(bytes: &[u8], needle: &str) -> usize {
        text(bytes).matches(needle).count()
    }

    /// A page with one line of text on it.
    fn page_saying(pdf: &mut Pdf, words: &str) {
        let mut c = Content::new();
        c.text((100.0, 100.0), 12.0, 0.0, Anchor::Start, 0.0, words);
        let (w, h) = a4();
        pdf.add_page(w, h, c);
    }

    /// Reads `startxref`, then checks that every in-use entry points at its own object.
    fn assert_xref_is_right(bytes: &[u8]) {
        let s = text(bytes);
        assert!(s.starts_with("%PDF-1.4\n"));
        assert!(s.ends_with("%%EOF\n"));
        let start: usize = s[s.rfind("startxref\n").unwrap() + 10..]
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let table = &bytes[start..];
        assert!(table.starts_with(b"xref\n0 "));
        let header_end = 5 + table[5..].iter().position(|b| *b == b'\n').unwrap() + 1;
        let n: usize = text(&table[7..header_end - 1]).parse().unwrap();
        assert!(n >= 4);
        assert_eq!(
            &table[header_end..header_end + 20],
            b"0000000000 65535 f \n"
        );
        for i in 1..n {
            let entry = &table[header_end + 20 * i..header_end + 20 * (i + 1)];
            assert_eq!(&entry[10..], b" 00000 n \n", "entry {i}");
            let offset: usize = text(&entry[..10]).parse().unwrap();
            let expect = format!("{i} 0 obj\n");
            assert!(
                bytes[offset..].starts_with(expect.as_bytes()),
                "object {i} at {offset}"
            );
        }
        assert!(s.contains(&format!("/Size {n} ")));
    }

    #[test]
    fn xref_offsets_land_on_their_objects() {
        let mut pdf = Pdf::new("Two pages");
        page_saying(&mut pdf, "one");
        page_saying(&mut pdf, "two");
        assert_eq!(pdf.page_count(), 2);
        let doc = pdf.finish().unwrap();
        assert_eq!(doc.pages, 2);
        assert_xref_is_right(&doc.bytes);
        assert_eq!(count(&doc.bytes, "/Type /Page "), 2);
        assert!(text(&doc.bytes).contains("/Count 2"));
    }

    /// 210 mm and 297 mm at 72 / 25.4 pt per mm: 595.2756 and 841.8898.
    #[test]
    fn an_a4_page_has_the_a4_media_box() {
        let mut pdf = Pdf::new("A4");
        let (w, h) = a4();
        pdf.add_page(w, h, Content::new());
        let doc = pdf.finish().unwrap();
        assert!(text(&doc.bytes).contains("/MediaBox [0 0 595.276 841.890]"));
    }

    /// 1 pt = 1/72 in and 1 in = 25.4 mm by definition, so 10 mm = 28.346 pt and 110 mm = 311.811 pt.
    #[test]
    fn a_100_mm_line_spans_283_465_pt() {
        let mut c = Content::new();
        c.polyline(
            &[
                (10.0 * PT_PER_MM, 10.0 * PT_PER_MM),
                (110.0 * PT_PER_MM, 10.0 * PT_PER_MM),
            ],
            false,
            BLACK,
        );
        let mut pdf = Pdf::new("Line");
        let (w, h) = a4();
        pdf.add_page(w, h, c);
        let doc = pdf.finish().unwrap();
        let stream = text(&page_stream(&doc.bytes, 0));
        assert!(
            stream.contains("28.346 28.346 m\n311.811 28.346 l\nS\n"),
            "{stream}"
        );
        // 0.35 mm is 0.992 pt.
        assert!(stream.contains("0.992 w"), "{stream}");
        assert!(stream.contains("[] 0 d"), "{stream}");
    }

    #[test]
    fn a_dashed_closed_grey_path_says_so() {
        let mut c = Content::new();
        let dashed = Stroke {
            width_mm: 0.18,
            grey: 0.55,
            dash_mm: Some((2.0, 1.0)),
        };
        c.polyline(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true, dashed);
        // A path of fewer than two points draws nothing.
        c.polyline(&[(5.0, 5.0)], false, BLACK);
        c.polyline(&[(f64::NAN, 5.0), (1.0, 1.0)], false, BLACK);
        let mut pdf = Pdf::new("Dash");
        pdf.add_page(100.0, 100.0, c);
        let doc = pdf.finish().unwrap();
        let stream = text(&page_stream(&doc.bytes, 0));
        assert!(stream.contains("0.550 G"), "{stream}");
        assert!(stream.contains("[5.669 2.835] 0 d"), "{stream}");
        assert!(stream.contains("10.000 10.000 l\nh\nS\n"), "{stream}");
        assert_eq!(stream.matches("S\n").count(), 1, "{stream}");
        assert!(!stream.contains("NaN"), "{stream}");
    }

    #[test]
    fn the_same_input_gives_the_same_bytes() {
        let build = || {
            let mut pdf = Pdf::new("Same");
            page_saying(&mut pdf, "Station 3");
            pdf.finish().unwrap().bytes
        };
        let bytes = build();
        assert_eq!(bytes, build());
        assert!(!text(&bytes).contains("/CreationDate"));
        assert!(!text(&bytes).contains("/ID"));
        let producer = format!("/Producer (SeaCraft {})", crate::VERSION);
        assert!(text(&bytes).contains(&producer));
    }

    #[test]
    fn a_form_used_on_three_pages_is_written_once() {
        let mut pdf = Pdf::new("Forms");
        let mut drawing = Content::new();
        drawing.polyline(&[(0.0, 0.0), (50.0, 50.0)], false, BLACK);
        drawing.text((10.0, 10.0), 8.0, 0.0, Anchor::Start, 0.0, "in the form");
        let form = pdf.add_form([0.0, 0.0, 50.0, 50.0], drawing);
        for i in 0..3 {
            let mut c = Content::new();
            c.save();
            c.clip_rect(10.0, 10.0, 80.0, 80.0);
            c.translate(20.0 * i as f64, 0.0);
            c.form(form);
            c.restore();
            pdf.add_page(100.0, 100.0, c);
        }
        let doc = pdf.finish().unwrap();
        assert_eq!(count(&doc.bytes, "/Subtype /Form"), 1);
        assert_eq!(count(&doc.bytes, " Do\n"), 3);
        assert!(text(&doc.bytes).contains("/BBox [0.000 0.000 50.000 50.000]"));
        let second = text(&page_stream(&doc.bytes, 1));
        assert_eq!(
            second,
            "q\n10.000 10.000 80.000 80.000 re W n\n1 0 0 1 20.000 0.000 cm\n/X0 Do\nQ\n"
        );
        // The text is in the form, not on the pages, and is found either way.
        assert_eq!(shown_text(&doc.bytes), ["in the form"]);
        assert!(page_text(&doc.bytes, 0).is_empty());
        assert_xref_is_right(&doc.bytes);
    }

    #[test]
    fn turkish_text_is_embedded_and_searchable() {
        let words = "Kayık Şile ğüöç İ";
        let mut pdf = Pdf::new("Türkçe");
        page_saying(&mut pdf, words);
        let doc = pdf.finish().unwrap();
        let s = text(&doc.bytes);
        for needle in [
            "/Identity-H",
            "/FontFile2",
            "/ToUnicode",
            "/AAAAAA+DejaVuSans",
        ] {
            assert!(s.contains(needle), "{needle}");
        }
        for needle in [
            "/CIDFontType2",
            "/CIDToGIDMap /Identity",
            "/Flags 4",
            "/DW 1000",
        ] {
            assert!(s.contains(needle), "{needle}");
        }
        assert_eq!(shown_text(&doc.bytes), [words]);
        assert_eq!(page_text(&doc.bytes, 0), [words]);
        assert!(doc.missing.is_empty());
        assert_xref_is_right(&doc.bytes);
        // The embedded font is the subset, not the 757 kB file.
        assert!(doc.bytes.len() < 100_000, "{}", doc.bytes.len());
    }

    #[test]
    fn a_character_the_font_lacks_becomes_a_question_mark_and_is_reported() {
        // U+8239 (a Chinese character) and U+1F6A2 (an emoji), neither in DejaVu Sans.
        let mut pdf = Pdf::new("Missing");
        page_saying(&mut pdf, "\u{8239} A");
        page_saying(&mut pdf, "\u{1F6A2}\u{8239}");
        let doc = pdf.finish().unwrap();
        assert_eq!(doc.missing, ['\u{8239}', '\u{1F6A2}']);
        assert_eq!(shown_text(&doc.bytes), ["? A", "??"]);
        assert_xref_is_right(&doc.bytes);
    }

    #[test]
    fn a_title_with_brackets_and_turkish_survives() {
        let mut pdf = Pdf::new("a(b)\\c Şile");
        page_saying(&mut pdf, "x");
        let doc = pdf.finish().unwrap();
        assert_xref_is_right(&doc.bytes);
        // UTF-16BE with a byte order mark, as hex: FEFF, then "a(b)\c Şile".
        let s = text(&doc.bytes);
        let hex = "FEFF 0061 0028 0062 0029 005C 0063 0020 015E 0069 006C 0065".replace(' ', "");
        assert!(
            s.contains(&format!("/Title <{hex}> ")),
            "{}",
            &s[s.find("/Title").unwrap()..][..80]
        );
    }

    #[test]
    fn a_page_over_5_08_m_is_an_error() {
        let mut pdf = Pdf::new("Big");
        pdf.add_page(14_401.0, 100.0, Content::new());
        let err = pdf.finish().unwrap_err();
        assert!(err.contains("5.08 m"), "{err}");
        let mut pdf = Pdf::new("Just fits");
        pdf.add_page(MAX_PAGE_PT, MAX_PAGE_PT, Content::new());
        assert!(pdf.finish().is_ok());
    }

    #[test]
    fn a_page_with_no_size_is_an_error() {
        for (w, h) in [
            (0.0, 100.0),
            (f64::NAN, 100.0),
            (100.0, -5.0),
            (100.0, f64::INFINITY),
        ] {
            let mut pdf = Pdf::new("Bad");
            pdf.add_page(w, h, Content::new());
            assert!(pdf.finish().is_err(), "{w} x {h}");
        }
        assert!(
            Pdf::new("Empty").finish().is_err(),
            "a document needs a page"
        );
    }

    #[test]
    fn a_document_without_text_embeds_no_font() {
        let mut c = Content::new();
        c.polyline(&[(0.0, 0.0), (10.0, 10.0)], false, BLACK);
        c.text((0.0, 0.0), 10.0, 0.0, Anchor::Start, 0.0, "");
        let mut pdf = Pdf::new("Lines only");
        pdf.add_page(100.0, 100.0, c);
        let doc = pdf.finish().unwrap();
        assert!(!text(&doc.bytes).contains("/FontFile2"));
        assert!(!text(&doc.bytes).contains("/Font"));
        assert_xref_is_right(&doc.bytes);
    }

    #[test]
    fn long_text_wraps_at_spaces_and_content_can_be_joined() {
        let words = "one two three four five six seven";
        let width = text_width_pt("one two three", 10.0);
        let lines = wrap_text(words, 10.0, width + 0.01);
        assert_eq!(lines, ["one two three", "four five six", "seven"]);
        assert!(lines.iter().all(|l| text_width_pt(l, 10.0) <= width + 0.01));
        assert_eq!(wrap_text("", 10.0, 100.0), Vec::<String>::new());
        assert_eq!(wrap_text("unbreakable", 10.0, 1.0), ["unbreakable"]);
        assert_eq!(wrap_text("fits", 10.0, 1000.0), ["fits"]);

        let mut pdf = Pdf::new("Joined");
        let form = pdf.add_form([0.0, 0.0, 10.0, 10.0], Content::new());
        let mut part = Content::new();
        part.text((10.0, 10.0), 10.0, 0.0, Anchor::Start, 0.0, "part \u{8239}");
        part.form(form);
        let mut whole = Content::new();
        whole.polyline(&[(0.0, 0.0), (5.0, 5.0)], false, BLACK);
        whole.append(&part);
        pdf.add_page(100.0, 100.0, whole);
        let doc = pdf.finish().unwrap();
        assert_eq!(page_text(&doc.bytes, 0), ["part ?"]);
        assert_eq!(doc.missing, ['\u{8239}']);
        let stream = text(&page_stream(&doc.bytes, 0));
        assert!(stream.find("S\n").unwrap() < stream.find("Tj").unwrap());
        assert!(stream.ends_with("/X0 Do\n"), "{stream}");
        assert!(text(&doc.bytes).contains("/XObject << /X0 "));
    }

    /// The width is the sum of the font's own advances, read here a second way through `ttf`.
    #[test]
    fn anchors_shift_text_by_its_width() {
        let font = crate::ttf::Font::parse(crate::fonts::DEJAVU_SANS).unwrap();
        let zero = font.advance(font.glyph('0').unwrap()) as f64;
        let width = 3.0 * zero / font.metrics().units_per_em as f64 * 10.0;
        assert!((text_width_pt("000", 10.0) - width).abs() < 1e-9);
        assert!(width > 15.0 && width < 25.0, "{width}");
        assert_eq!(text_width_pt("", 10.0), 0.0);

        let stream = |anchor, angle| {
            let mut c = Content::new();
            c.text((100.0, 50.0), 10.0, angle, anchor, 0.5, "000");
            let mut pdf = Pdf::new("Anchor");
            pdf.add_page(200.0, 200.0, c);
            text(&page_stream(&pdf.finish().unwrap().bytes, 0))
        };
        let at = |x: f64, y: f64| format!("1 0 0 1 {x:.3} {y:.3} Tm");
        assert!(stream(Anchor::Start, 0.0).contains(&at(100.0, 50.0)));
        assert!(stream(Anchor::Middle, 0.0).contains(&at(100.0 - width / 2.0, 50.0)));
        assert!(stream(Anchor::End, 0.0).contains(&at(100.0 - width, 50.0)));
        assert!(stream(Anchor::Start, 0.0).contains("0.500 g"));
        // Turned a quarter turn, the text runs up the page and the anchor shifts it down.
        let up = stream(Anchor::End, 90.0);
        assert!(
            up.contains(&format!("0 1 -1 0 100.000 {:.3} Tm", 50.0 - width)),
            "{up}"
        );
    }
}
