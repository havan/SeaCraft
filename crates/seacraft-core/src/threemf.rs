//! 3MF (3D Manufacturing Format, Core specification 1.3) export: a ZIP package
//! holding `[Content_Types].xml`, `_rels/.rels` and the model part
//! `3D/3dmodel.model`.
//!
//! Adapted from Tangency Labs' `solidcraft/crates/io/src/threemf.rs`
//! (MIT OR Apache-2.0, see `ATTRIBUTION.md`): the `weld` pass that makes a
//! tessellation watertight, and the package layout (content types part, root
//! relationships part, one model part with the 3MF core namespace, a single
//! mesh object and one build item) follow that file.
//!
//! Differences, because SeaCraft's core has no dependencies:
//!
//! * the archive is written here as a **stored** (compression method 0) ZIP,
//!   with exact CRC-32 values and offsets, using the same IEEE 802.3 table as
//!   the PNG encoder in `raster.rs` — a hull mesh is a few hundred kilobytes,
//!   and correctness matters more than size;
//! * the model is in metres (`unit="meter"`), which is the unit the core works
//!   in, instead of millimetres;
//! * no colours, base materials, face tags, components or model metadata
//!   beyond `Application` and the escaped `Title`;
//! * `weld` only merges coincident vertices; the T-junction splitting of the
//!   original is left out because `subdiv::evaluate` shares its vertices, so
//!   there are no cracks between separately meshed faces to close;
//! * there is no reader: SeaCraft exports 3MF, it does not import it.

use crate::math::Vec3;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The 3MF core namespace (specification 1.3, unchanged since 2015/02).
const CORE_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/core/2015/02";
/// Relationship type naming the model part.
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

// ---- weld ---------------------------------------------------------------

/// Make a tessellation watertight: merge vertices closer than `eps` and drop
/// triangles that collapse in the process.
///
/// Vertices are bucketed into a grid of cell size `eps`; a vertex joins the
/// first already-kept vertex within `eps` found in its own or any of the 27
/// neighbouring cells, so the result does not depend on the input order in any
/// way that matters for a mesh whose duplicates are exact or near-exact. A
/// triangle with two equal indices after the remap is a sliver of zero area and
/// is removed.
pub fn weld(positions: &[Vec3], triangles: &[[u32; 3]], eps: f64) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    // A non-positive tolerance would divide by zero in the grid cells; fall
    // back to an exact-coincidence grid rather than panicking.
    let eps = if eps > 0.0 { eps } else { f64::EPSILON };
    let cell = |p: Vec3| {
        [
            (p.x / eps).round() as i64,
            (p.y / eps).round() as i64,
            (p.z / eps).round() as i64,
        ]
    };

    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    let mut verts: Vec<Vec3> = Vec::with_capacity(positions.len());
    let mut remap: Vec<u32> = Vec::with_capacity(positions.len());

    for p in positions {
        let c = cell(*p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = [
                        c[0].saturating_add(dx),
                        c[1].saturating_add(dy),
                        c[2].saturating_add(dz),
                    ];
                    for &i in grid.get(&key).into_iter().flatten() {
                        if verts.get(i as usize).is_some_and(|q| q.dist(*p) <= eps) {
                            found = Some(i);
                            break 'search;
                        }
                    }
                }
            }
        }
        let i = match found {
            Some(i) => i,
            None => {
                // A mesh with more than u32::MAX vertices is not a hull.
                let i = u32::try_from(verts.len()).unwrap_or(u32::MAX);
                verts.push(*p);
                grid.entry(c).or_default().push(i);
                i
            }
        };
        remap.push(i);
    }

    let tris: Vec<[u32; 3]> = triangles
        .iter()
        .filter_map(|t| {
            let (a, b, c) = (
                *remap.get(t[0] as usize)?,
                *remap.get(t[1] as usize)?,
                *remap.get(t[2] as usize)?,
            );
            (a != b && b != c && c != a).then_some([a, b, c])
        })
        .collect();
    (verts, tris)
}

// ---- XML ----------------------------------------------------------------

/// XML text escaping: the five predefined entities, and control characters
/// that XML 1.0 cannot carry at all are dropped.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

/// A coordinate as written: finite, and without the `-0` spelling that some
/// strict parsers dislike. `Display` for `f64` never uses an exponent, so the
/// result is a plain decimal the 3MF `xs:double` attributes accept.
fn coord(v: f64) -> f64 {
    if v.is_finite() {
        v + 0.0
    } else {
        0.0
    }
}

/// The model part's XML: one object holding the whole mesh, at `unit="meter"`.
fn model_xml(name: &str, verts: &[Vec3], tris: &[[u32; 3]]) -> String {
    let title = esc(name);
    let object_name = if title.is_empty() {
        "SeaCraft hull".to_string()
    } else {
        title.clone()
    };
    let mut x = String::with_capacity(256 + verts.len() * 48 + tris.len() * 40);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        x,
        "<model unit=\"meter\" xml:lang=\"en-US\" xmlns=\"{CORE_NS}\">"
    );
    let _ = writeln!(
        x,
        " <metadata name=\"Application\">SeaCraft {}</metadata>",
        crate::VERSION
    );
    let _ = writeln!(x, " <metadata name=\"Title\">{title}</metadata>");
    x.push_str(" <resources>\n");
    let _ = writeln!(
        x,
        "  <object id=\"1\" type=\"model\" name=\"{object_name}\">"
    );
    x.push_str("   <mesh>\n    <vertices>\n");
    for v in verts {
        let _ = writeln!(
            x,
            "     <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            coord(v.x),
            coord(v.y),
            coord(v.z)
        );
    }
    x.push_str("    </vertices>\n    <triangles>\n");
    for [a, b, c] in tris {
        let _ = writeln!(x, "     <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>");
    }
    x.push_str("    </triangles>\n   </mesh>\n  </object>\n </resources>\n");
    x.push_str(" <build>\n  <item objectid=\"1\"/>\n </build>\n</model>\n");
    x
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n\
 <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n\
 <Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/>\n\
</Types>\n";

/// `_rels/.rels`, naming the model part.
fn rels_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
 <Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"{MODEL_REL}\"/>\n\
</Relationships>\n"
    )
}

// ---- ZIP (stored) -------------------------------------------------------

/// IEEE 802.3 CRC-32 table, the same construction as `raster.rs` (which is
/// private to that module, so the table is repeated here).
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// CRC-32 of `bytes`, as ZIP stores it (pre- and post-inverted).
fn crc32(bytes: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in bytes {
        c = CRC_TABLE[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
/// Version made by / needed: 2.0, the lowest that allows stored and deflated.
const ZIP_VERSION: u16 = 20;
/// 1980-01-01 in MS-DOS date form, the earliest a ZIP timestamp can carry.
const DOS_DATE: u16 = 0x0021;
const DOS_TIME: u16 = 0;

/// Pack `files` into a ZIP archive with every entry stored uncompressed.
///
/// Local file headers, a central directory and the end-of-central-directory
/// record are written by hand: sizes and CRC-32 values are exact, every name is
/// ASCII (so the UTF-8 flag stays clear) and no data descriptor is needed
/// because sizes are known before the header goes out.
fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    // Offsets inside a ZIP are 32-bit. A hull's model part is far below that;
    // anything larger would need ZIP64, which is out of scope here.
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<(&str, u32, u32, u32)> = Vec::with_capacity(files.len()); // name, crc, size, offset

    for (name, data) in files {
        let offset = out.len() as u32;
        let size = data.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version needed to extract
        out.extend_from_slice(&0u16.to_le_bytes()); // general purpose flags
        out.extend_from_slice(&0u16.to_le_bytes()); // compression method 0 = stored
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed size
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.push((name, crc, size, offset));
    }

    let cd_offset = out.len() as u32;
    for (name, crc, size, offset) in &central {
        out.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version made by
        out.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method
        out.extend_from_slice(&DOS_TIME.to_le_bytes());
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(&0u16.to_le_bytes()); // comment length
        out.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        out.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        out.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    let cd_size = out.len() as u32 - cd_offset;

    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with the central directory
    out.extend_from_slice(&(central.len() as u16).to_le_bytes()); // entries on this disk
    out.extend_from_slice(&(central.len() as u16).to_le_bytes()); // entries in total
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // archive comment length
    out
}

// ---- writer -------------------------------------------------------------

/// A 3MF package of one mesh: `faces` are polygon index lists into `verts`
/// (fan-triangulated here), welded at 1 µm so the exported surface is
/// watertight, and written in metres.
pub fn write_3mf(name: &str, verts: &[Vec3], faces: &[Vec<usize>]) -> Vec<u8> {
    let n = verts.len();
    let idx = |i: usize| if i < n { u32::try_from(i).ok() } else { None };
    let mut raw: Vec<[u32; 3]> = Vec::with_capacity(faces.len() * 2);
    for face in faces {
        let Some((first, rest)) = face.split_first() else {
            continue;
        };
        // Fan from the first corner: a quad becomes two triangles, an n-gon
        // n - 2. Out-of-range indices are skipped rather than written as a
        // triangle a slicer would reject.
        for pair in rest.windows(2) {
            if let (Some(a), Some(b), Some(c)) = (idx(*first), idx(pair[0]), idx(pair[1])) {
                raw.push([a, b, c]);
            }
        }
    }
    let (verts, tris) = weld(verts, &raw, 1e-6);
    let model = model_xml(name, &verts, &tris);
    let rels = rels_xml();
    zip_stored(&[
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }

    /// `(name, stored crc, data)` of every local entry, walking the file from
    /// its start the way a reader does.
    fn local_entries(zip: &[u8]) -> Vec<(String, u32, Vec<u8>)> {
        let mut out = Vec::new();
        let mut o = 0usize;
        while o + 30 <= zip.len() && u32_at(zip, o) == LOCAL_SIG {
            assert_eq!(u16_at(zip, o + 8), 0, "stored only");
            let crc = u32_at(zip, o + 14);
            let size = u32_at(zip, o + 18) as usize;
            let name_len = u16_at(zip, o + 26) as usize;
            let extra = u16_at(zip, o + 28) as usize;
            let name =
                String::from_utf8(zip[o + 30..o + 30 + name_len].to_vec()).expect("ascii name");
            let start = o + 30 + name_len + extra;
            out.push((name, crc, zip[start..start + size].to_vec()));
            o = start + size;
        }
        out
    }

    #[test]
    fn weld_merges_duplicate_vertices_and_drops_slivers() {
        let positions = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            // An exact duplicate of vertex 0 and one 1e-9 away from vertex 1.
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0 + 1e-9, 0.0, 0.0),
        ];
        let triangles = [
            [0u32, 1, 2],
            [0, 2, 3],
            // Collapses to [0, 0, 1]: zero area once 4 joins 0.
            [0, 4, 1],
            // 5 joins 1, so this stays the quad's second triangle.
            [5, 2, 3],
        ];
        let (verts, tris) = weld(&positions, &triangles, 1e-6);
        assert_eq!(verts.len(), 4, "duplicates merged: {verts:?}");
        assert_eq!(
            tris,
            vec![[0, 1, 2], [0, 2, 3], [1, 2, 3]],
            "sliver dropped, indices remapped"
        );

        // A tolerance below the gap leaves the near-duplicate alone.
        let (verts, tris) = weld(&positions[..4], &triangles[..2], 1e-6);
        assert_eq!((verts.len(), tris.len()), (4, 2));
    }

    #[test]
    fn three_mf_archive_is_a_valid_zip_with_meter_model() {
        // A closed box: 8 vertices, 6 quad faces -> 12 triangles.
        let mut verts = Vec::new();
        for z in [0.0, 1.0] {
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                verts.push(Vec3::new(x, y, z));
            }
        }
        let faces = vec![
            vec![0, 1, 2, 3],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let zip = write_3mf("Hull <&> \"demo\"", &verts, &faces);

        assert_eq!(
            zip.get(..4),
            Some(&LOCAL_SIG.to_le_bytes()[..]),
            "starts with a local file header"
        );

        // End of central directory: last 22 bytes, no archive comment.
        assert!(zip.len() > 22);
        let eocd = zip.len() - 22;
        assert_eq!(u32_at(&zip, eocd), EOCD_SIG, "EOCD record");

        let entries = local_entries(&zip);
        let names: Vec<&str> = entries.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["[Content_Types].xml", "_rels/.rels", "3D/3dmodel.model"]
        );

        // The central directory agrees with the local headers: same count, and
        // its recorded offset really holds the first central header.
        assert_eq!(
            u16_at(&zip, eocd + 10) as usize,
            entries.len(),
            "entries on disk"
        );
        assert_eq!(
            u16_at(&zip, eocd + 8) as usize,
            entries.len(),
            "entries in total"
        );
        let cd_offset = u32_at(&zip, eocd + 16) as usize;
        let cd_size = u32_at(&zip, eocd + 12) as usize;
        assert_eq!(
            u32_at(&zip, cd_offset),
            CENTRAL_SIG,
            "central directory offset"
        );
        assert_eq!(cd_offset + cd_size, eocd, "central directory size");
        assert_eq!(
            u16_at(&zip, cd_offset + 28) as usize,
            u16_at(&zip, 26) as usize,
            "name length"
        );
        assert_eq!(
            u32_at(&zip, cd_offset + 42) as usize,
            0,
            "first local header offset"
        );

        for (name, crc, data) in &entries {
            assert_eq!(crc32(data), *crc, "{name}: CRC-32");
        }

        let model = String::from_utf8(entries[2].2.clone()).expect("utf-8 model");
        assert!(model.contains("unit=\"meter\""), "{model}");
        assert!(model.contains(CORE_NS), "{model}");
        assert!(
            model.contains(
                "<metadata name=\"Title\">Hull &lt;&amp;&gt; &quot;demo&quot;</metadata>"
            ),
            "{model}"
        );
        assert_eq!(model.matches("<vertex ").count(), 8, "{model}");
        assert_eq!(model.matches("<triangle ").count(), 12, "{model}");
        assert_eq!(model.matches("<item ").count(), 1, "{model}");
        assert!(model.contains("<object id=\"1\" type=\"model\""), "{model}");
        assert!(!model.contains('\u{0}'));

        // The content types and relationships parts point at the model part.
        let rels = String::from_utf8(entries[1].2.clone()).expect("utf-8 rels");
        assert!(rels.contains("/3D/3dmodel.model") && rels.contains(MODEL_REL));
        let types = String::from_utf8(entries[0].2.clone()).expect("utf-8 types");
        assert!(types.contains("3dmanufacturing-3dmodel+xml"));
    }
}
