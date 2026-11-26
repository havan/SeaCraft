# The SeaCraft website

The source of <https://seacraft.app/>. `.github/workflows/pages.yml` publishes it when a release is published,
building the page and the program from the release's tag, so the page's numbers are the ones the downloads give.
Run by hand from `main`, the workflow publishes main's page with the program and example hulls of a release tag
(the latest unless you name one), for a fix to the page between releases. Pull requests and pushes to `main` only
build it. The `github-pages` environment must allow the tags `v*` as well as `main`.

`build.py` runs the `seacraft` binary on each hull in `examples/` and writes the results next to the page. No
number is typed into the page, so it cannot drift from the program. The drawings are cut from the mesh
`seacraft export` writes; the script measures nothing. The page draws that data and computes no hydrostatics
of its own. Every number shown must remain output from `seacraft`, and the "not validated" notice stays
next to it (see `DISCLAIMER.md`).

```console
cargo build --release --bin seacraft
python3 site/build.py
python3 -m http.server --directory site/_site 8000
python3 site/test_build.py    # how build.py reads seacraft's output
```

Building every hull takes a minute or two, so `build.py` keeps a hull already in `_site/` when the `seacraft`
binary, `build.py` and that hull's files in `examples/` are all unchanged. Editing the page rebuilds in about a
second. Rebuilding the binary after a change to the code builds the hulls again, because their numbers may
have moved. `--rebuild-hulls` builds them all regardless. The published site starts from an empty folder, so it
always builds every hull.

## Languages

The page is written once per language in `LANGS` in `build.py`: English at the root, every other language in a
folder of its own (`tr/`). `src/index.html` is a template: each `{{t:key}}` is a string from `locales/<lang>.json`,
and the `js.*`, `hull.*` and `method.*` strings go into the page for `app.js`. A first visit to the English page
from a browser set to Turkish goes to `tr/`; a language chosen with the switch is remembered and wins, and
the switch's links also carry it (`?lang=en`) for a browser that stores nothing, and arriving from another page
of the site counts as a choice too.

What `seacraft` says in words comes from the program in each language, never from these catalogs: `--json` stays
English, so `build.py` runs the same command again with `--lang tr` and reads its warnings and its heel summary
against the program's own `locales/tr.json`. The build stops if the two disagree on how many warnings there are.
The reports, lines plans and plywood sheets are written once per language (`report.tr.html`, `build/tr/`).

The site's catalogs follow the program's rules in `docs/i18n.md` and `docs/i18n-glossary-tr.md`, with one
difference: a string may hold the inline markup the page needs (`<code>`, `<a href>`), the same tags in every
language. `test_build.py` checks the keys, placeholders and tags match, and that every key is used.

| Path | What it is |
|---|---|
| `build.py` | Runs `seacraft`, writes `_site/`. Python standard library only |
| `src/` | The page: HTML template, CSS and script, with no build step or dependencies |
| `locales/` | The page's strings, one file per language |
| `fonts/` | Self-hosted fonts and their licences, so the site makes no third-party requests |
