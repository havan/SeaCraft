# The SeaCraft website

The source of <https://havan.github.io/SeaCraft/>. `.github/workflows/pages.yml` publishes it from `main`.

`build.py` runs the `seacraft` binary on each hull in `examples/` and writes the results next to the page. No
number is typed into the page, so it cannot drift from the program. The drawings are cut from the mesh
`seacraft export` writes; the script measures nothing. The page draws that data and computes no hydrostatics
of its own. Every number shown must remain output from `seacraft`, and the "not validated" notice stays
next to it (see `DISCLAIMER.md`).

```console
cargo build --release --bin seacraft
python3 site/build.py
python3 -m http.server --directory site/_site 8000
```

| Path | What it is |
|---|---|
| `build.py` | Runs `seacraft`, writes `_site/`. Python standard library only |
| `src/` | The page: HTML, CSS and script, with no build step or dependencies |
| `fonts/` | Self-hosted fonts and their licences, so the site makes no third-party requests |
