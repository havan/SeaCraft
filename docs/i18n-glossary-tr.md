# Turkish Naval Architecture Glossary (i18n)

Terminology conventions for SeaCraft's Turkish (`tr`) translation.

## Rules

1. **Decimal separator:** Always keep `.` as the decimal separator (consistent with CAD/engineering software and `.fef` files).
2. **Symbols & SI units:** Keep international naval-architecture symbols (`LWL`, `LOA`, `LPP`, `BWL`, `T`, `Δ`, `∇`, `Cb`, `Cp`, `Cm`, `Cwp`, `KB`, `KG`, `KM`, `GM`, `GZ`, `KN`, `LCB`, `LCF`, `LCG`, `VCG`, `TCG`, `MTC`, `TPC`, `Fn`, `Rn`, `RAO`, `MSI`, `FSM`, `FSC`, `R_T`, `P_E`, `C_w`) and SI units (`m`, `mm`, `m²`, `m³`, `t`, `kg/m³`, `kN`, `kW`, `kn`, `°`, `rad/s`, `s`) unchanged.
3. **Formats, methods & proper nouns:** Keep `.fef`, `.seacraft.json`, `SVG`, `DXF`, `IGES`, `OBJ`, `STL`, `3MF`, `PLY`, `PDF`, `FREE!ship`, `SeaCraft`, `Loop`, `Catmull-Clark`, `Holtrop–Mennen`, `Michell`, `Savitsky`, `IMO A.749`, `SOLAS`, `JONSWAP`, `Bretschneider`, and `Wigley` unchanged.
4. **Turkish casing:** Never call Rust `.to_uppercase()` or `.to_lowercase()` on translated Turkish strings (standard Unicode case folding maps `i` to `I` instead of `İ`, and `I` to `i` instead of `ı`). Store the exact casing in `locales/tr.json`.
5. **Disclaimers:** Never soften or weaken caveats from `DISCLAIMER.md` or `disclaimer.not_validated`.
6. **Turkish-first with parenthetical secondaries:** Prefer Turkish naval-architecture terms first. Where two usages are close or widely paired in practice, include the secondary term in parentheses in headings, labels, and tooltips (e.g., `Su çekimi (Draft)`, `Boyuna kesitler (Bato hatları)`). In tight buttons, HUD badges, or narrow table columns, use the shorter primary term.

## Term Table

| English | Turkish (Primary / Heading Form) | Compact Form (Buttons / Tight Columns) | Notes |
|---|---|---|---|
| Draft | Su çekimi (Draft) | Su çekimi | |
| Buttocks | Boyuna kesitler (Bato hatları) | Boyuna kesitler | |
| Stations | Postalar | Postalar | |
| Waterlines | Su hatları | Su hatları | |
| Lines plan | Endaze resmi | Endaze | |
| Crease | Keskin kenar | Keskin kenar | |
| Subdivision | Alt bölünme | Alt bölünme | |
| Control net | Kontrol ağı | Kontrol ağı | |
| Control point | Kontrol noktası | Kontrol noktası | |
| Layer | Katman | Katman | |
| Hull | Tekne / Gövde | Tekne | Context-dependent (`Tekne formu`, `Gövde`) |
| Forward / Aft | Baş / Kıç | Baş / Kıç | |
| Port / Starboard | İskele / Sancak | İsk. / Snc. | Compact forms for HUD triad & margins |
| Up | Yukarı | Yuk. | Compact form for HUD triad |
| Keel | Omurga | Omurga | |
| Trim | Trim | Trim | |
| Heel | Meyil | Meyil | |
| Displacement | Deplasman | Deplasman | |
| Hydrostatics | Hidrostatik | Hidrostatik | |
| Stability | Stabilite (Denge) | Stabilite | |
| Righting arm | Doğrultucu kol | Doğrultucu kol | |
| Cross curves | Çapraz eğriler | Çapraz eğriler | |
| Loading condition | Yükleme durumu | Yükleme durumu | |
| Tank / Free surface | Tank / Serbest yüzey | Tank / Serbest yüzey | |
| Damage stability / Compartment | Yaralı stabilite / Bölme | Yaralı stabilite / Bölme | |
| Windage / Weather criterion | Rüzgâr alanı / Hava kriteri | Rüzgâr alanı / Hava kriteri | |
| Resistance | Direnç | Direnç | |
| Wave pattern | Dalga deseni | Dalga deseni | |
| Seakeeping | Denizcilik (Deniz tutma) | Denizcilik | |
| Curvature combs | Eğrilik tarakları | Eğrilik tarakları | |
| Builder templates / Station moulds | İnşa şablonları / Posta kalıpları | İnşa şablonları | |
| Plywood panels / Nesting | Kontrplak paneller / Yerleşim (Nesting) | Paneller / Yerleşim | |
| Table of offsets | Ofset tablosu | Ofset tablosu | |
| Design report | Tasarım raporu | Tasarım raporu | |
| PASS / FAIL / WARN | GEÇTİ / KALDI / UYARI | GEÇTİ / KALDI / UYARI | |
| Developable / Not developable | Açınabilir / Açınamaz | Açınabilir / Açınamaz | Never *açılabilir* (that is "can be opened") |
| Deadrise | Kalkıntı açısı (deadrise) | Kalkıntı açısı | Not *karina* (the underwater body) or *dip açısı* |
| IS Code Part A | IS Kodu Bölüm A | IS Kodu Bölüm A | |
| Loll | Yatık denge (loll) | Yatık denge | |
| Seam (plywood, stitch-and-glue) | Dikiş | Dikiş | *Seam drift*: dikiş sapması |
| Thin-ship theory | İnce gemi (thin-ship) kuramı | İnce gemi | |
| Lies flat / does not lie flat | Düz açılır / Düz açılmaz | Düz açılır / Düz açılmaz | |
| Degrees | ° | ° | Never *der* (rule 2) |
| Percent of a number | %{value} | %{value} | Never put a suffix on a placeholder (`%{pct}'i`): the right vowel-harmony suffix depends on the number |
