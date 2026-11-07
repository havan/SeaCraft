---
name: seacraft-ground-rules
description: Use before adding a dependency, porting code, adding an asset or wording that names another project, or when unsure whether something may be copied, committed or claimed in SeaCraft.
---

# Ground rules

## Licence

SeaCraft is `GPL-3.0-or-later`. Code may be copied into SeaCraft from projects with compatible licences. It
cannot go the other way into permissive projects, so do not offer it to them unless every author agrees.

- Reading GPL code, including FREE!ship, is fine. Keep attribution intact when adding derived documentation.
  Do not call anything here "clean-room": SeaCraft was written with FREE!ship's source available.
- A new dependency needs a GPLv3-compatible licence. `cargo deny check licenses` and `deny.toml` enforce it.
  After changing dependencies, run `cargo xtask licences`; never edit `THIRD-PARTY-LICENSES.txt` by hand.
  `cargo xtask dist` writes the full texts into a package, and fails if a crate has none.

## Ported code

* **Ported code**: keep the source file, project and licence in the module
  header *and* in `ATTRIBUTION.md`. Adapt the vector type to
  `seacraft_core::math::Vec3`; never add `solidcraft-*`, `fieldcraft-*` or
  `solidcore` as Cargo dependencies (millimetres, `truck`, `egui 0.36`).

Check the upstream licence file rather than assuming. If upstream has a `NOTICE`, carry what applies in ours.

## Honest results

Never call a result verified, validated, approved or compliant unless it is, and say against what. See
`DISCLAIMER.md`. A new reference number states where it came from in `docs/oracles.md`: published (with the
source you read), hand-derived or a script. Do not back-fill a "published" value from the program's output.

## Names

SeaCraft is not affiliated with FREE!ship or Tangency Labs. Name other projects only to attribute them or to
describe compatibility.

## Private files

Local notes stay on your machine, in the folders `.gitignore` lists. Never commit them, or absolute paths from
your machine. `cargo xtask private` fails if you do. Decisions others need go in
`ROADMAP.md`, `docs/architecture.md`, an issue or a PR, not in session notes.
