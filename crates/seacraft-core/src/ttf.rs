//! Read a TrueType font, and write a subset of one.
//!
//! Enough of the format to set text in a PDF ([`crate::pdf`]): which glyph draws a character, how
//! wide it is, the font's vertical metrics, and a smaller copy of the file that holds only the
//! glyphs a document uses. The subset keeps every glyph id and empties the outlines it does not
//! need, so a glyph id read from the whole font is still right in the subset.
//!
//! Every read is checked: a damaged file is an `Err`, never a panic.
//!
//! Written from the OpenType specification's table descriptions (`head`, `hhea`, `maxp`, `hmtx`,
//! `loca`, `glyf`, `cmap`, `post`, `OS/2`); nothing here is ported.

use std::collections::BTreeSet;

/// A font's measurements, in font units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    /// Font units in one em, the unit every other number here is in.
    pub units_per_em: u16,
    /// Height above the baseline (`hhea`).
    pub ascent: i16,
    /// Depth below the baseline, negative (`hhea`).
    pub descent: i16,
    /// Height of a capital letter: `OS/2` `sCapHeight` when the font states it, else the ascent.
    pub cap_height: i16,
    /// The box that holds every glyph: x min, y min, x max, y max (`head`).
    pub bbox: [i16; 4],
}

/// A parsed TrueType font, borrowing the file's bytes.
#[derive(Debug, Clone)]
pub struct Font<'a> {
    data: &'a [u8],
    cmap: Cmap<'a>,
    hmtx: &'a [u8],
    glyphs: u16,
    h_metrics: u16,
    metrics: Metrics,
}

/// The character map subtable in use.
#[derive(Debug, Clone, Copy)]
enum Cmap<'a> {
    /// Format 4: segments of 16 bit characters.
    Segments(&'a [u8]),
    /// Format 12: groups of 32 bit characters.
    Groups(&'a [u8]),
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn i16_at(d: &[u8], at: usize) -> Option<i16> {
    u16_at(d, at).map(|v| v as i16)
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The bytes of the table called `tag`, or `None` when the file has none or is cut short.
pub fn table<'a>(data: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let count = u16_at(data, 4)? as usize;
    for i in 0..count {
        let rec = 12 + 16 * i;
        if data.get(rec..rec + 4)? == tag {
            let start = u32_at(data, rec + 8)? as usize;
            let len = u32_at(data, rec + 12)? as usize;
            return data.get(start..start.checked_add(len)?);
        }
    }
    None
}

/// The bytes of one glyph in `glyf`: empty for a glyph with no outline (a space, or a glyph a
/// subset left out), `None` when the font has no such glyph or is damaged.
pub fn outline(data: &[u8], glyph: u16) -> Option<&[u8]> {
    let long = i16_at(table(data, b"head")?, 50)? != 0;
    let glyphs = u16_at(table(data, b"maxp")?, 4)?;
    if glyph >= glyphs {
        return None;
    }
    let loca = table(data, b"loca")?;
    let at = |i: usize| -> Option<usize> {
        if long {
            u32_at(loca, 4 * i).map(|v| v as usize)
        } else {
            u16_at(loca, 2 * i).map(|v| 2 * v as usize)
        }
    };
    let (start, end) = (at(glyph as usize)?, at(glyph as usize + 1)?);
    table(data, b"glyf")?.get(start..end)
}

/// The glyphs a composite glyph is built from; empty for a simple or an empty glyph.
fn components(outline: &[u8]) -> Option<Vec<u16>> {
    const WORDS: u16 = 0x0001;
    const SCALE: u16 = 0x0008;
    const MORE: u16 = 0x0020;
    const X_Y_SCALE: u16 = 0x0040;
    const TWO_BY_TWO: u16 = 0x0080;
    let mut out = Vec::new();
    if outline.is_empty() || i16_at(outline, 0)? >= 0 {
        return Some(out);
    }
    let mut at = 10;
    loop {
        let flags = u16_at(outline, at)?;
        out.push(u16_at(outline, at + 2)?);
        at += 4 + if flags & WORDS != 0 { 4 } else { 2 };
        at += if flags & SCALE != 0 {
            2
        } else if flags & X_Y_SCALE != 0 {
            4
        } else if flags & TWO_BY_TWO != 0 {
            8
        } else {
            0
        };
        if flags & MORE == 0 {
            return Some(out);
        }
    }
}

/// The sum of a table's big-endian 32 bit words, the last one padded with zeros.
fn checksum(d: &[u8]) -> u32 {
    d.chunks(4).fold(0u32, |sum, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

impl<'a> Font<'a> {
    /// Read a TrueType file. Fails with a reason when a table this reader needs is missing, cut
    /// short or inconsistent.
    pub fn parse(data: &'a [u8]) -> Result<Font<'a>, String> {
        let need = |tag: &[u8; 4], min: usize| -> Result<&'a [u8], String> {
            let name = String::from_utf8_lossy(tag).into_owned();
            let t = table(data, tag).ok_or(format!("the font has no complete `{name}` table"))?;
            if t.len() < min {
                return Err(format!("the font's `{name}` table is too short"));
            }
            Ok(t)
        };
        match u32_at(data, 0) {
            Some(0x0001_0000) | Some(0x7472_7565) => {}
            Some(_) => return Err("not a TrueType font (no TrueType outlines)".into()),
            None => return Err("not a font: the file is too short".into()),
        }
        let head = need(b"head", 54)?;
        let hhea = need(b"hhea", 36)?;
        let maxp = need(b"maxp", 6)?;
        let short = "the font is damaged";
        let units_per_em = u16_at(head, 18).ok_or(short)?;
        if units_per_em == 0 {
            return Err("the font's em size is zero".into());
        }
        let glyphs = u16_at(maxp, 4).ok_or(short)?;
        let h_metrics = u16_at(hhea, 34).ok_or(short)?;
        if glyphs == 0 || h_metrics == 0 || h_metrics > glyphs {
            return Err("the font's glyph counts are inconsistent".into());
        }
        let hmtx = need(b"hmtx", 4 * h_metrics as usize)?;
        let long = i16_at(head, 50).ok_or(short)? != 0;
        let loca = need(b"loca", (glyphs as usize + 1) * if long { 4 } else { 2 })?;
        let glyf = need(b"glyf", 0)?;
        let last = if long {
            u32_at(loca, 4 * glyphs as usize).map(|v| v as usize)
        } else {
            u16_at(loca, 2 * glyphs as usize).map(|v| 2 * v as usize)
        };
        if last.ok_or(short)? > glyf.len() {
            return Err("the font's glyph table is cut short".into());
        }
        let ascent = i16_at(hhea, 4).ok_or(short)?;
        let cap_height = table(data, b"OS/2")
            .filter(|os2| u16_at(os2, 0).is_some_and(|version| version >= 2))
            .and_then(|os2| i16_at(os2, 88))
            .filter(|cap| *cap > 0)
            .unwrap_or(ascent);
        let mut bbox = [0i16; 4];
        for (i, b) in bbox.iter_mut().enumerate() {
            *b = i16_at(head, 36 + 2 * i).ok_or(short)?;
        }
        Ok(Font {
            data,
            cmap: Self::character_map(need(b"cmap", 4)?)?,
            hmtx,
            glyphs,
            h_metrics,
            metrics: Metrics {
                units_per_em,
                ascent,
                descent: i16_at(hhea, 6).ok_or(short)?,
                cap_height,
                bbox,
            },
        })
    }

    /// The Unicode subtable to read: Windows full repertoire, then Windows 16 bit, then Unicode.
    fn character_map(cmap: &'a [u8]) -> Result<Cmap<'a>, String> {
        let count = u16_at(cmap, 2).unwrap_or(0) as usize;
        let mut best: Option<(u8, Cmap<'a>)> = None;
        for i in 0..count {
            let rec = 4 + 8 * i;
            let (Some(platform), Some(encoding), Some(offset)) = (
                u16_at(cmap, rec),
                u16_at(cmap, rec + 2),
                u32_at(cmap, rec + 4),
            ) else {
                break;
            };
            let Some(sub) = cmap.get(offset as usize..) else {
                continue;
            };
            let map = match u16_at(sub, 0) {
                Some(4) => Cmap::Segments(sub),
                Some(12) => Cmap::Groups(sub),
                _ => continue,
            };
            let rank = match (platform, encoding, map) {
                (3, 10, Cmap::Groups(_)) => 0,
                (3, 1, Cmap::Segments(_)) => 1,
                (0, _, Cmap::Groups(_)) => 2,
                (0, _, Cmap::Segments(_)) => 3,
                _ => continue,
            };
            if best.is_none_or(|(r, _)| rank < r) {
                best = Some((rank, map));
            }
        }
        best.map(|(_, map)| map)
            .ok_or("the font has no Unicode character map this reader understands".into())
    }

    /// The font's measurements.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// How many glyphs the font has; glyph ids run from 0 to one less.
    pub fn glyph_count(&self) -> u16 {
        self.glyphs
    }

    /// The glyph that draws `c`, or `None` when the font has none. Glyph 0, the font's "missing"
    /// box, counts as none.
    pub fn glyph(&self, c: char) -> Option<u16> {
        let code = c as u32;
        let id = match self.cmap {
            Cmap::Groups(sub) => {
                let groups = u32_at(sub, 12)? as usize;
                let group = |i: usize| -> Option<(u32, u32, u32)> {
                    let at = 16 + 12 * i;
                    Some((u32_at(sub, at)?, u32_at(sub, at + 4)?, u32_at(sub, at + 8)?))
                };
                let (mut lo, mut hi) = (0, groups);
                let mut found = None;
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let (start, end, first) = group(mid)?;
                    if code < start {
                        hi = mid;
                    } else if code > end {
                        lo = mid + 1;
                    } else {
                        found = Some(first.checked_add(code - start)?);
                        break;
                    }
                }
                found?
            }
            Cmap::Segments(sub) => {
                let code = u16::try_from(code).ok()?;
                let seg_x2 = u16_at(sub, 6)? as usize;
                let (ends, starts) = (14, 16 + seg_x2);
                let (deltas, ranges) = (16 + 2 * seg_x2, 16 + 3 * seg_x2);
                let mut found = None;
                for i in (0..seg_x2).step_by(2) {
                    if u16_at(sub, ends + i)? < code {
                        continue;
                    }
                    let start = u16_at(sub, starts + i)?;
                    if start > code {
                        break;
                    }
                    let delta = u16_at(sub, deltas + i)?;
                    let range = u16_at(sub, ranges + i)? as usize;
                    found = Some(if range == 0 {
                        code.wrapping_add(delta) as u32
                    } else {
                        let at = ranges + i + range + 2 * (code - start) as usize;
                        match u16_at(sub, at)? {
                            0 => 0,
                            g => g.wrapping_add(delta) as u32,
                        }
                    });
                    break;
                }
                found?
            }
        };
        u16::try_from(id)
            .ok()
            .filter(|g| *g != 0 && *g < self.glyphs)
    }

    /// How far the pen moves after `glyph`, in font units; 0 for a glyph the font does not have.
    pub fn advance(&self, glyph: u16) -> u16 {
        if glyph >= self.glyphs {
            return 0;
        }
        // Glyphs past the last full record share its advance.
        let record = glyph.min(self.h_metrics - 1) as usize;
        u16_at(self.hmtx, 4 * record).unwrap_or(0)
    }

    /// A copy of the font that keeps only `glyphs` (and glyph 0, and the parts composite glyphs
    /// are built from). Glyph ids do not change: every other glyph stays in the font with an empty
    /// outline. The result has no `cmap`, so it is for embedding where the document addresses
    /// glyphs by id, as a PDF does. The same input gives the same bytes.
    pub fn subset(&self, glyphs: &BTreeSet<u16>) -> Result<Vec<u8>, String> {
        let damaged = || "the font is damaged".to_string();
        let mut keep: BTreeSet<u16> = BTreeSet::new();
        let mut todo: Vec<u16> = std::iter::once(0).chain(glyphs.iter().copied()).collect();
        while let Some(g) = todo.pop() {
            if g >= self.glyphs {
                return Err(format!(
                    "glyph {g} is not in the font, which has {}",
                    self.glyphs
                ));
            }
            if !keep.insert(g) {
                continue;
            }
            let parts = components(outline(self.data, g).ok_or_else(damaged)?);
            todo.extend(
                parts
                    .ok_or_else(damaged)?
                    .into_iter()
                    .filter(|p| !keep.contains(p)),
            );
        }

        let mut glyf = Vec::new();
        let mut loca = Vec::with_capacity(4 * (self.glyphs as usize + 1));
        for g in 0..self.glyphs {
            loca.extend((glyf.len() as u32).to_be_bytes());
            if keep.contains(&g) {
                glyf.extend(outline(self.data, g).ok_or_else(damaged)?);
                // Long offsets are to four byte boundaries.
                glyf.resize(glyf.len().next_multiple_of(4), 0);
            }
        }
        loca.extend((glyf.len() as u32).to_be_bytes());

        let copy = |tag: &[u8; 4]| table(self.data, tag).map(<[u8]>::to_vec);
        let mut head = copy(b"head").ok_or_else(damaged)?;
        head.get_mut(8..12).ok_or_else(damaged)?.fill(0); // checkSumAdjustment, set below
        head.get_mut(50..52)
            .ok_or_else(damaged)?
            .copy_from_slice(&1i16.to_be_bytes());
        // A version 3 `post` names no glyphs: the 32 byte header is the whole table.
        let mut post = copy(b"post").unwrap_or_default();
        post.resize(32, 0);
        post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"glyf", glyf),
            (*b"head", head),
            (*b"hhea", copy(b"hhea").ok_or_else(damaged)?),
            (*b"hmtx", copy(b"hmtx").ok_or_else(damaged)?),
            (*b"loca", loca),
            (*b"maxp", copy(b"maxp").ok_or_else(damaged)?),
            (*b"post", post),
        ];
        // The hinting programs, and `name`, which carries the copyright notice.
        for tag in [b"cvt ", b"fpgm", b"prep", b"name"] {
            if let Some(t) = copy(tag) {
                tables.push((*tag, t));
            }
        }
        tables.sort_by_key(|(tag, _)| *tag);

        let count = tables.len() as u16;
        let entry_selector = count.ilog2() as u16;
        let search_range = 16 * (1u16 << entry_selector);
        let mut out = Vec::new();
        out.extend(0x0001_0000u32.to_be_bytes());
        for v in [
            count,
            search_range,
            entry_selector,
            16 * count - search_range,
        ] {
            out.extend(v.to_be_bytes());
        }
        let mut offset = 12 + 16 * tables.len();
        let mut head_at = 0;
        for (tag, body) in &tables {
            if tag == b"head" {
                head_at = offset;
            }
            out.extend(tag);
            out.extend(checksum(body).to_be_bytes());
            out.extend((offset as u32).to_be_bytes());
            out.extend((body.len() as u32).to_be_bytes());
            offset += body.len().next_multiple_of(4);
        }
        for (_, body) in &tables {
            out.extend(body);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const SANS: &[u8] = crate::fonts::DEJAVU_SANS;

    fn font() -> Font<'static> {
        Font::parse(SANS).expect("DejaVu Sans parses")
    }

    fn be16(d: &[u8], at: usize) -> u16 {
        u16::from_be_bytes([d[at], d[at + 1]])
    }

    fn be32(d: &[u8], at: usize) -> u32 {
        u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
    }

    /// The sum of a table's big-endian words, the last one padded with zeros.
    fn word_sum(d: &[u8]) -> u32 {
        d.chunks(4).fold(0u32, |sum, c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            sum.wrapping_add(u32::from_be_bytes(w))
        })
    }

    #[test]
    fn dejavu_has_the_turkish_letters() {
        for c in "ğĞşŞıİöÖüÜçÇ".chars() {
            assert!(font().glyph(c).is_some(), "{c}");
        }
    }

    #[test]
    fn digits_share_one_advance() {
        let f = font();
        let w = f.advance(f.glyph('0').unwrap());
        assert!(w > 0);
        for c in '1'..='9' {
            assert_eq!(f.advance(f.glyph(c).unwrap()), w, "{c}");
        }
    }

    #[test]
    fn a_character_the_font_lacks_has_no_glyph() {
        // U+8239, the Chinese character for a boat. Written as an escape because a GUI test
        // checks that the embedded fonts draw every character in the sources.
        assert_eq!(font().glyph('\u{8239}'), None);
    }

    /// 2048 is the em size stored in this file's `head` table, read here a second way.
    #[test]
    fn the_em_size_is_read_from_head() {
        let head = table(SANS, b"head").unwrap();
        assert_eq!(font().metrics().units_per_em, be16(head, 18));
        assert_eq!(font().metrics().units_per_em, 2048);
    }

    /// The size limit is this font's fixed tables (`hmtx` 25 kB, `loca` 25 kB, `name` 16 kB, the
    /// hinting programs 2 kB) plus a handful of outlines; the whole file is 757 kB.
    #[test]
    fn a_subset_keeps_used_outlines_and_empties_the_rest() {
        let f = font();
        let used: BTreeSet<u16> = "Kayık 9 m".chars().map(|c| f.glyph(c).unwrap()).collect();
        let sub = f.subset(&used).unwrap();
        for g in &used {
            assert_eq!(outline(&sub, *g), outline(SANS, *g), "glyph {g}");
        }
        let z = f.glyph('Z').unwrap();
        assert!(!outline(SANS, z).unwrap().is_empty());
        assert!(outline(&sub, z).unwrap().is_empty());
        assert!(sub.len() < 80_000, "{}", sub.len());
        assert_eq!(table(&sub, b"name"), table(SANS, b"name"));
        assert_eq!(table(&sub, b"hmtx"), table(SANS, b"hmtx"));
        assert_eq!(f.subset(&used).unwrap(), sub, "deterministic");
        // The subset is itself a font this reader accepts, apart from the `cmap` it leaves out.
        assert!(table(&sub, b"cmap").is_none());
        assert_eq!(be16(table(&sub, b"maxp").unwrap(), 4), f.glyph_count());
    }

    #[test]
    fn a_composite_glyph_brings_its_components() {
        let f = font();
        let composite = "ğşçöüİ"
            .chars()
            .map(|c| f.glyph(c).unwrap())
            .find(|g| (be16(outline(SANS, *g).unwrap(), 0) as i16) < 0)
            .expect("DejaVu builds one of these letters from parts");
        let sub = f.subset(&BTreeSet::from([composite])).unwrap();
        // The first component's glyph id follows the 10 byte header and the 2 byte flags.
        let first = be16(outline(SANS, composite).unwrap(), 12);
        assert_ne!(first, composite);
        assert!(!outline(&sub, first).unwrap().is_empty());
        let parts = components(outline(SANS, composite).unwrap()).unwrap();
        assert!(parts.len() >= 2, "{parts:?}");
        assert_eq!(parts[0], first);
        for part in parts {
            assert_eq!(outline(&sub, part), outline(SANS, part), "component {part}");
            assert!(!outline(&sub, part).unwrap().is_empty());
        }
    }

    /// The TrueType specification: a table's checksum is the sum of its words, `head` being summed
    /// with `checkSumAdjustment` zero, and the whole file sums to `0xB1B0AFBA`.
    #[test]
    fn a_subset_has_valid_checksums() {
        let f = font();
        let sub = f.subset(&BTreeSet::from([f.glyph('a').unwrap()])).unwrap();
        let tables = be16(&sub, 4) as usize;
        assert!(tables >= 9, "{tables}");
        let mut last_tag = [0u8; 4];
        for i in 0..tables {
            let rec = 12 + 16 * i;
            let tag: [u8; 4] = sub[rec..rec + 4].try_into().unwrap();
            assert!(tag > last_tag, "the directory is sorted by tag");
            last_tag = tag;
            let (sum, off, len) = (
                be32(&sub, rec + 4),
                be32(&sub, rec + 8),
                be32(&sub, rec + 12),
            );
            assert_eq!(off % 4, 0);
            let mut body = sub[off as usize..(off + len) as usize].to_vec();
            if &tag == b"head" {
                body[8..12].fill(0);
            }
            assert_eq!(word_sum(&body), sum, "{}", String::from_utf8_lossy(&tag));
        }
        assert_eq!(sub.len() % 4, 0);
        assert_eq!(word_sum(&sub), 0xB1B0_AFBA);
        assert_eq!(be32(table(&sub, b"post").unwrap(), 0), 0x0003_0000);
        assert_eq!(be16(table(&sub, b"head").unwrap(), 50), 1, "long loca");
    }

    #[test]
    fn a_broken_font_is_an_error_not_a_panic() {
        assert!(Font::parse(b"").is_err());
        assert!(Font::parse(&SANS[..1000]).is_err());
        assert!(Font::parse(&[0xFF; 4096]).is_err());
        // Every prefix of the table directory, and a file whose tables are cut short.
        for len in (0..400).chain([5_000, 50_000, 500_000]) {
            assert!(Font::parse(&SANS[..len]).is_err(), "{len}");
        }
        assert_eq!(outline(&SANS[..1000], 3), None);
        assert_eq!(table(b"", b"head"), None);
    }

    /// A glyph id past the end of the font has no advance and no outline, and cannot be subset.
    #[test]
    fn a_glyph_id_out_of_range_is_refused() {
        let f = font();
        let n = f.glyph_count();
        assert_eq!(f.advance(n), 0);
        assert_eq!(outline(SANS, n), None);
        assert!(f.subset(&BTreeSet::from([n])).is_err());
    }
}
