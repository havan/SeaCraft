#!/usr/bin/env python3
"""Build the SeaCraft website into a folder that GitHub Pages can serve.

Every number and hull drawing on the site comes from running the `seacraft` binary on the example
hulls in `examples/`, so the site cannot drift from the program. The curves in the drawings are cut
from the mesh `seacraft export` writes; this script draws them and measures nothing. Standard library
only.

    cargo build --release --bin seacraft
    python3 site/build.py --seacraft target/release/seacraft --out site/_site

The page is written once per language in `LANGS`, from `site/src/index.html` and the strings in
`site/locales/`. Whatever seacraft says in words (a hull's description, where it heels, a warning) is
taken in each language from seacraft itself, run with `--lang`; its `--json` stays English.

`--program-root` names the checkout the program, the example hulls and the program's catalogs come from,
when it is not this one: the Pages workflow builds the page from main with a release's program.

A hull already in the output folder is kept when its fingerprint still matches (see `hull_key`), so
a change to the page alone rebuilds in seconds. A clean folder, as in CI, always builds every hull.
"""

import argparse
import csv
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = Path(__file__).resolve().parent / "src"
FONTS = Path(__file__).resolve().parent / "fonts"
SITE_LOCALES = Path(__file__).resolve().parent / "locales"
# The checkout the program, the example hulls and the program's catalogs come from; see --program-root.
PROGRAM = ROOT

# The page's languages, English first: it is the page at the site's root, the others in a folder each.
LANGS = ("en", "tr")

# Holds a finished hull's fingerprint. Written last, so a hull cut short by a failure has none.
KEY_FILE = ".build-key"

# Page order. `resistance` picks the method whose range of hulls this example falls in, and `motions`
# is off for the planing hulls, because strip theory does not describe a boat on the plane. Each hull's
# label on the picker is `hull.<id>` in site/locales, and each method's name `method.<resistance>`.
HULLS = [
    {"id": "trawler-22m", "resistance": "holtrop", "motions": True},
    {"id": "yacht-10m", "resistance": "dsyhs", "motions": True},
    {"id": "kayik-9m", "resistance": "holtrop", "motions": True},
    {"id": "runabout-6m", "resistance": "savitsky", "motions": False},
    {"id": "skiff-4m", "resistance": "savitsky", "motions": False},
    {"id": "container-230m", "resistance": "holtrop", "motions": True},
    {"id": "wigley", "resistance": "holtrop", "motions": True},
]


def run(seacraft, *args, cwd=None, stderr=False):
    """Run seacraft and return stdout (and stderr after it, when asked), failing the build on any error."""
    done = subprocess.run([str(seacraft), *map(str, args)], capture_output=True, encoding="utf-8", cwd=cwd)
    if done.returncode != 0:
        sys.exit(f"seacraft {' '.join(map(str, args))} failed:\n{done.stderr}")
    return done.stdout + done.stderr if stderr else done.stdout


# --- seacraft's words in each language ------------------------------------------------------------
#
# `--json` is English whatever `--lang` says, so the other languages come from the same command run
# without `--json`: its lines are matched against the program's own catalog, never a copy kept here.

_program_catalogs = {}


def program_catalog(lang):
    """The program's strings in `lang`, from locales/ in the program's checkout."""
    if lang not in _program_catalogs:
        path = PROGRAM / "locales" / f"{lang}.json"
        try:
            _program_catalogs[lang] = json.loads(path.read_text(encoding="utf-8"))
        except OSError as error:
            sys.exit(f"cannot read the program's {lang} strings: {error}")
    return _program_catalogs[lang]


def program_string(lang, key):
    text = program_catalog(lang).get(key)
    if text is None:
        sys.exit(f"the program's {lang} catalog has no {key}")
    return text


def template_pattern(template):
    """A regular expression matching a whole line printed from a catalog template, one group per placeholder."""
    parts = re.split(r"\{[a-z_]+\}", template)
    return re.compile("^" + "(.*)".join(re.escape(p) for p in parts) + "$")


# The templates seacraft prints a warning with, in the human-readable output of the commands the page runs.
WARNING_KEYS = ("cli.common.warning", "cli.common.warning_indent", "cli.heel.warning")


def warnings_in(text, lang):
    """The warnings in a command's human-readable output, in the order it printed them."""
    patterns = [template_pattern(program_string(lang, key)) for key in WARNING_KEYS]
    found = []
    for line in text.splitlines():
        for pattern in patterns:
            match = pattern.match(line)
            if match:
                found.append(match.group(1).strip())
                break
    return found


def settles_in(text, lang):
    """Where `seacraft heel` says the hull settles: the text of its `settles` row."""
    label = program_string(lang, "cli.heel.row_settles")
    for line in text.splitlines():
        if line.strip().startswith(label + " "):
            return line.strip()[len(label):].strip()
    sys.exit(f"no {label!r} row in seacraft heel's {lang} output")


def printed_in(seacraft, *args, cwd=None):
    """What one command prints without `--json` in each language besides English: `{lang: text}`."""
    return {lang: run(seacraft, "--lang", lang, *args, cwd=cwd, stderr=True) for lang in LANGS[1:]}


def said(seacraft, english, *args, cwd=None, settles=False, outputs=None):
    """What one command says, in every language: `{lang: value}`.

    `english` is the value from the command's `--json`. The other languages run the same command without
    `--json` and read the warnings (or, with `settles`, where the hull settles) from what it prints;
    `outputs` from `printed_in` reads two things from one run. The build stops when a language finds a
    different number of warnings, rather than pairing the wrong ones.
    """
    if outputs is None:
        outputs = printed_in(seacraft, *args, cwd=cwd)
    texts = {"en": english}
    for lang in LANGS[1:]:
        printed = outputs[lang]
        if settles:
            texts[lang] = settles_in(printed, lang)
            continue
        found = warnings_in(printed, lang)
        if len(found) != len(english):
            sys.exit(
                f"seacraft {' '.join(map(str, args))}: {len(english)} warnings in --json, "
                f"{len(found)} in the {lang} output:\n{printed}"
            )
        texts[lang] = found
    return texts


def release_version(cargo_toml):
    """The version in `[workspace.package]`: what a release of this commit is called.

    The Pages checkout has no tags, so `seacraft --version` names the commit there; the Download section
    shows this plain version instead. A regex rather than `tomllib`, which needs Python 3.11.
    """
    text = Path(cargo_toml).read_text(encoding="utf-8")
    section = re.search(r"^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    found = section and re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), re.M)
    if not found:
        sys.exit(f"no [workspace.package] version in {cargo_toml}")
    return found.group(1)


def shown(hull_id, *args):
    """The command a reader would type to get this result."""
    return " ".join(["seacraft", args[0], f"{hull_id}.fef", *map(str, args[1:])])


def sig(value, digits=5):
    return float(f"{value:.{digits}g}")


def nums(*values, digits=4):
    return ",".join(f"{v:.{digits}f}".rstrip("0").rstrip(".") for v in values)


# --- Mesh slicing, for drawing only ---------------------------------------------------------------


def read_obj(path):
    verts, tris = [], []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        if line.startswith("v "):
            verts.append(tuple(float(c) for c in line.split()[1:4]))
        elif line.startswith("f "):
            ids = [int(tok.split("/")[0]) - 1 for tok in line.split()[1:]]
            tris.extend((ids[0], ids[k], ids[k + 1]) for k in range(1, len(ids) - 1))
    return verts, tris


class Mesh:
    def __init__(self, verts, tris):
        self.verts, self.tris = verts, tris
        self.bounds = {
            axis: [
                (min(c), max(c), t)
                for t, c in ((t, [verts[i][axis] for i in t]) for t in tris)
            ]
            for axis in range(3)
        }

    def cut(self, axis, at):
        """Line segments where the plane `coordinate[axis] = at` crosses the mesh.

        A plane exactly through mesh vertices gives degenerate crossings, so callers nudge `at` off them.
        """
        segments = []
        verts = self.verts
        for low, high, tri in self.bounds[axis]:
            if low > at or high < at:
                continue
            crossings = []
            for k in range(3):
                a, b = verts[tri[k]], verts[tri[(k + 1) % 3]]
                da, db = a[axis] - at, b[axis] - at
                if (da < 0) != (db < 0):
                    f = da / (da - db)
                    crossings.append(tuple(a[i] + f * (b[i] - a[i]) for i in range(3)))
            if len(crossings) == 2:
                segments.append(crossings)
        return segments


def chain(segments, u, v, tol):
    """Join segments into polylines in the (u, v) coordinates, then simplify them."""
    key = lambda p: (round(p[u], 5), round(p[v], 5))
    ends = {}
    for index, (a, b) in enumerate(segments):
        ends.setdefault(key(a), []).append(index)
        ends.setdefault(key(b), []).append(index)
    used = [False] * len(segments)
    lines = []

    def walk(point, line):
        while True:
            for index in ends[key(point)]:
                if not used[index]:
                    used[index] = True
                    a, b = segments[index]
                    point = b if key(a) == key(point) else a
                    line.append((point[u], point[v]))
                    break
            else:
                return

    for index, (a, b) in enumerate(segments):
        if used[index]:
            continue
        used[index] = True
        forward = [(a[u], a[v]), (b[u], b[v])]
        walk(b, forward)
        backward = []
        walk(a, backward)
        lines.append(simplify(backward[::-1] + forward, tol))
    return lines


def simplify(points, tol):
    """Ramer-Douglas-Peucker, iterative so that long curves cannot hit the recursion limit."""
    if len(points) < 3:
        return points
    keep = {0, len(points) - 1}
    stack = [(0, len(points) - 1)]
    while stack:
        first, last = stack.pop()
        ax, ay = points[first]
        dx, dy = points[last][0] - ax, points[last][1] - ay
        length = (dx * dx + dy * dy) ** 0.5
        worst, at = 0.0, None
        for i in range(first + 1, last):
            px, py = points[i][0] - ax, points[i][1] - ay
            d = abs(px * dy - py * dx) / length if length > 0 else (px * px + py * py) ** 0.5
            if d > worst:
                worst, at = d, i
        if at is not None and worst > tol:
            keep.add(at)
            stack += [(first, at), (at, last)]
    return [points[i] for i in sorted(keep)]


def path(lines, map_point, places):
    out = []
    for line in lines:
        out.append(
            "M" + "L".join(f"{x:.{places}f},{y:.{places}f}" for x, y in map(map_point, line))
        )
    return "".join(out)


def geometry(obj_path, draft):
    """Station, buttock and waterline curves plus outlines, as SVG path data in metres."""
    mesh = Mesh(*read_obj(obj_path))
    xs, ys, zs = zip(*mesh.verts)
    x0, x1, y1, z0, z1 = min(xs), max(xs), max(ys), min(zs), max(zs)
    length = x1 - x0
    tol = length * 0.00025
    places = 2 if length > 60 else 3
    nudge = length * 1.37e-5

    # Outline in profile: the mesh's lowest and highest points at each station.
    top, bottom = [], []
    for k in range(161):
        x = x0 + length * (k + 0.001) / 160.002
        heights = [p[2] for seg in mesh.cut(0, x) for p in seg]
        if heights:
            top.append((x, max(heights)))
            bottom.append((x, min(heights)))
    outline = simplify(top, tol) + simplify(bottom, tol)[::-1]
    profile_outline = path([outline], lambda p: (p[0], -p[1]), places) + "Z"

    # Outline in the body plan: the widest point at each height, mirrored.
    widths = []
    for k in range(81):
        z = z0 + (z1 - z0) * (k + 0.001) / 80.002
        spans = [abs(p[1]) for seg in mesh.cut(2, z) for p in seg]
        if spans:
            widths.append((z, max(spans)))
    widths = simplify(widths, tol)
    right = [(y, -z) for z, y in widths]
    left = [(-y, -z) for z, y in reversed(widths)]
    body_outline = path([right + left], lambda p: p, places) + "Z"

    stations = []
    for k in range(11):
        x = x0 + length * k / 10
        x += nudge if k < 10 else -nudge
        if k == 0:
            x += nudge * 10
        lines = chain(mesh.cut(0, x), 1, 2, tol)
        sign = -1 if k < 5 else 1
        stations.append(
            {
                "x": round(x, 3),
                "body": path(lines, lambda p, s=sign: (s * p[0], -p[1]), places),
            }
        )

    buttocks = []
    for k in (1, 2, 3):
        y = y1 * k / 4 + nudge
        lines = chain(mesh.cut(1, y), 0, 2, tol)
        buttocks.append({"y": round(y, 3), "profile": path(lines, lambda p: (p[0], -p[1]), places)})

    waterlines = []
    for k in (1, 2, 3, 4, 5):
        z = draft * k / 4 + nudge
        if z >= z1:
            continue
        lines = chain(mesh.cut(2, z), 0, 1, tol)
        waterlines.append({"z": round(z, 3), "plan": path(lines, lambda p: (p[0], p[1]), places)})

    return {
        "x0": round(x0, 3),
        "x1": round(x1, 3),
        "y1": round(y1, 3),
        "z0": round(z0, 3),
        "z1": round(z1, 3),
        "profile_outline": profile_outline,
        "body_outline": body_outline,
        "stations": stations,
        "buttocks": buttocks,
        "waterlines": waterlines,
    }


# --- Results for one hull -------------------------------------------------------------------------


def parse_info(text):
    match = re.search(r"principal\s+L ([\d.]+)\s+B ([\d.]+)\s+T ([\d.]+) m", text)
    if not match:
        sys.exit("could not read the principal dimensions from `seacraft info`")
    return tuple(float(v) for v in match.groups())


def drafts_for(draft, depth):
    """Evenly spaced drafts with the design draft exactly on one row."""
    rows = [draft * k / 24 for k in range(1, 37)]
    return [d for d in rows if d <= depth]


def stability_curves(seacraft, hull_id, hull_dir, kg):
    heels = list(range(0, 85, 5))
    args = ("stability", f"{hull_id}.fef", "--kg", kg, "--heels", ",".join(map(str, heels)))
    run(seacraft, *args, "--csv", "gz.csv", cwd=hull_dir)
    by_displacement = {}
    with open(hull_dir / "gz.csv", newline="", encoding="utf-8") as handle:
        for row in csv.DictReader(handle):
            # seacraft writes a dash where the solve found no result: a gap, never a number.
            gz = None if row["gz_m"] == "—" else float(row["gz_m"])
            by_displacement.setdefault(float(row["displacement_t"]), []).append(
                [float(row["heel_deg"]), gz]
            )
    (hull_dir / "gz.csv").unlink()
    series = [
        {"name": f"{sig(d, 4):g} t", "points": [[h, None if g is None else round(g, 4)] for h, g in pts]}
        for d, pts in sorted(by_displacement.items())
    ]
    return {
        "command": shown(hull_id, "stability", "--kg", f"{kg:g}", "--heels", ",".join(map(str, heels))),
        "kg": kg,
        "series": series,
    }


def warning_text(data):
    """What the solver said about its own result, in its own words. The page shows it beside the curve."""
    return [w if isinstance(w, str) else json.dumps(w) for w in data.get("warnings") or []]


def resistance_curve(seacraft, hull_id, hull_dir, method, speeds):
    joined = ",".join(f"{s:g}" for s in speeds)
    if method == "savitsky":
        args = ("planing", f"{hull_id}.fef", "--speeds", joined)
        command = shown(hull_id, "planing", "--speeds", joined)
    else:
        args = ("resistance", f"{hull_id}.fef", "--method", method, "--speeds", joined)
        command = shown(hull_id, "resistance", "--method", method, "--speeds", joined)
    data = json.loads(run(seacraft, *args, "--json", cwd=hull_dir))
    points = []
    for row in data["speeds"]:
        newtons = row.get("rt_n", row.get("resistance_n"))
        if newtons is not None:
            points.append([row["speed_kn"], round(newtons / 1000, 3)])
    return {
        "command": command,
        "method": method,
        "points": points,
        "warnings": said(seacraft, warning_text(data), *args, cwd=hull_dir),
    }


def motions(seacraft, hull_id, hull_dir, draft, kg):
    args = ("seakeeping", f"{hull_id}.fef", "--draft", draft, "--kg", kg, "--points", 40)
    data = json.loads(run(seacraft, *args, "--json", cwd=hull_dir))
    rows = sorted(data["frequencies"], key=lambda f: f["lambda_over_l"])
    return {
        "command": shown(hull_id, "seakeeping", "--draft", f"{draft:g}", "--kg", f"{kg:g}", "--points", 40),
        "heave": [[round(f["lambda_over_l"], 3), round(f["heave_amp"], 4)] for f in rows],
        "pitch": [[round(f["lambda_over_l"], 3), round(f["pitch_amp"], 4)] for f in rows],
        "warnings": said(seacraft, warning_text(data), *args, cwd=hull_dir),
    }


def maybe(value, places):
    return None if value is None else round(value, places)


def section_top(body):
    """The highest point of a station's section, from the SVG path `geometry` cut ([y, -z] points).

    Only the parts that reach the centreline count, so a bulwark rail standing above the deck does
    not; an open boat, which reaches the centreline only at its keel, is topped by its sheer.
    """
    parts = [
        [tuple(map(float, point.split(","))) for point in part.split("L")]
        for part in body.split("M")
        if part
    ]
    hull = [part for part in parts if any(abs(y) < 1e-3 for y, _ in part)] or parts
    return max(-up for part in hull for _, up in part)


def heel_step(result, y):
    """One `seacraft heel --json` result as one position of the trial weight.

    Only a `stable` result has a state to draw. Every other kind (`loll`, `capsizes`, `cannot_float`,
    `unresolved`) keeps its kind and seacraft's own `summary`, and nothing to draw.
    """
    equilibrium = result["equilibrium"]
    state = equilibrium.get("state") if equilibrium["kind"] == "stable" else None
    step = {"y": y, "kind": equilibrium["kind"], "summary": result["summary"], "warnings": warning_text(result)}
    if state is None:
        step.update(dict.fromkeys(("heel_deg", "b", "g", "gz_m", "heeling_arm_m", "draft", "trim_m", "waterplane")))
        return step
    plane = state["waterplane"]
    step.update(
        {
            "heel_deg": round(state["heel_deg"], 2),
            "b": [round(state["b"]["y"], 4), round(state["b"]["z"], 4)],
            "g": [round(state["g"]["y"], 4), round(state["g"]["z"], 4)],
            "gz_m": round(state["gz_m"], 4),
            "heeling_arm_m": round(state["heeling_arm_m"], 4),
            "draft": maybe(state["draft"], 3),
            "trim_m": maybe(state["trim_m"], 3),
            # n . p + d = 0 in model coordinates: the page draws its line across the drawn station.
            "waterplane": [round(v, 6) for v in (plane["n"]["x"], plane["n"]["y"], plane["n"]["z"], plane["d"])],
        }
    )
    return step


# How many positions the trial weight takes across the deck, centreline first.
HEEL_POSITIONS = 7


def heel_steps(seacraft, hull_id, hull_dir, kg, beam, stations):
    """A trial weight walked across the deck, each position one `seacraft heel` run.

    The weight is 10 % of the design displacement, on the deck of the station nearest the upright LCB (where
    the bare hull's LCG defaults to), from the centreline out to 0.4 of the beam. Nothing between positions is drawn.
    """
    fef = f"{hull_id}.fef"
    hydro = json.loads(run(seacraft, "hydro", fef, "--kg", kg, "--json", cwd=hull_dir))
    mass = sig(hydro["displacement_t"] * 0.10, 2)
    x = round(hydro["lcb"]["x"], 3)
    # The section the figure draws, and the weight on its deck.
    station = min(stations, key=lambda s: abs(s["x"] - x))
    z = round(section_top(station["body"]), 3)
    steps = []
    for k in range(HEEL_POSITIONS):
        y = round(0.4 * beam * k / (HEEL_POSITIONS - 1), 3)
        weight = f"{mass:g}@{nums(x, y, z, digits=3)}"
        args = ("heel", fef, "--kg", kg, "--weight", weight)
        result = json.loads(run(seacraft, *args, "--json", cwd=hull_dir))
        step = heel_step(result, y)
        # One run in each language gives both where the hull settles and its warnings.
        outputs = printed_in(seacraft, *args, cwd=hull_dir)
        step["summary"] = said(seacraft, step["summary"], *args, settles=True, outputs=outputs)
        step["warnings"] = said(seacraft, step["warnings"], *args, outputs=outputs)
        step["command"] = shown(hull_id, "heel", "--kg", f"{kg:g}", "--weight", weight)
        steps.append(step)
    return {
        "mass_t": mass,
        "x": x,
        "z": z,
        "steps": steps,
        "station": {"x": station["x"], "body": station["body"]},
    }


# Froude numbers on the waterline length for the wave pattern, slow to past hull speed.
WAVE_FROUDES = [0.25, 0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60]
# Grid points along the waterline length: the default domain is five lengths long, so about 900 px.
WAVE_POINTS_PER_LENGTH = 180


def wave_fits(results):
    """Whether thin-ship theory fits this hull: `seacraft waves` warned at none of the speeds."""
    return bool(results) and all(not r.get("warnings") for r in results)


def wave_steps(seacraft, hull_id, hull_dir, length):
    """The far-field wave pattern at each of `WAVE_FROUDES`, or why it is not shown.

    Whether it is shown follows seacraft's own warnings, never a list kept here. Each image is the PNG
    seacraft wrote, made smaller with pngquant when it is installed.

    Each speed is computed once. The first speed seacraft warns at stops the run and removes the images
    already written: no pattern is shown for a hull the theory does not fit.
    """
    fef = f"{hull_id}.fef"
    spacing = sig(length / WAVE_POINTS_PER_LENGTH, 3)
    folder = hull_dir / "waves"
    folder.mkdir()
    steps = []
    for f in WAVE_FROUDES:
        name = f"fn-{f:.2f}.png"
        image = f"waves/{name}"
        # Run inside the waves folder, so the command shown writes a plain file name, as a reader's would.
        args = ("--fn", f"{f:.2f}", "--spacing", f"{spacing:g}", "-o", name)
        result = json.loads(run(seacraft, "waves", f"../{fef}", *args, "--json", cwd=folder))
        if not wave_fits([result]):
            reason = said(seacraft, warning_text(result), "waves", f"../{fef}", *args, cwd=folder)
            shutil.rmtree(folder)
            return {"fits": False, "steps": [], "reason": {lang: list(dict.fromkeys(r)) for lang, r in reason.items()}}
        shrink_png(hull_dir / image)
        steps.append(
            {
                "fn": f,
                "image": image,
                "size": list(result["grid_points"]),
                "bytes": (hull_dir / image).stat().st_size,
                # The sentence for each language, from the program's catalog by the regime's key.
                "regime": {"en": result["regime"]["sentence"]}
                | {lang: program_string(lang, f"wave.regime.{result['regime']['key']}") for lang in LANGS[1:]},
                "clip_m": round(result["max_abs_zeta_m"], 4),
                "notes": result.get("notes") or [],
                "warnings": warning_text(result),
                "command": shown(hull_id, "waves", *args),
            }
        )
    return {"fits": True, "steps": steps}


def shrink_png(path):
    """Quantise a PNG in place with pngquant, when it is installed and the result is smaller."""
    if not shutil.which("pngquant"):
        return
    done = subprocess.run(
        ["pngquant", "--quality", "65-90", "--strip", "--skip-if-larger", "--force", "--ext", ".png", str(path)],
        capture_output=True,
        encoding="utf-8",
    )
    # 98: not smaller, 99: below the quality floor. Either way the original stays, which is fine.
    if done.returncode not in (0, 98, 99):
        sys.exit(f"pngquant {path} failed:\n{done.stderr}")


def default_speeds(project, length):
    speeds = project.get("report", {}).get("speeds_kn") or []
    if len(speeds) >= 4:
        return speeds
    # Froude number 0.15 to 0.40 when the example does not name its own speeds.
    top = 0.40 * (9.81 * length) ** 0.5 * 1.944
    return [round(top * k / 8, 1) for k in range(3, 9)]


def digest(*paths):
    sha = hashlib.sha256()
    for item in paths:
        sha.update(Path(item).read_bytes())
    return sha.hexdigest()


def hull_key(seacraft, hull_id):
    """Everything a hull's results come from: the binary, this script and the hull's own files.

    A kept hull is therefore what a full build would write, and old numbers never pass as current.
    """
    examples = PROGRAM / "examples"
    catalogs = [PROGRAM / "locales" / f"{lang}.json" for lang in LANGS]
    return digest(seacraft, __file__, examples / f"{hull_id}.fef", examples / f"{hull_id}.seacraft.json", *catalogs)


def hull_is_current(hull_dir, key):
    try:
        return (hull_dir / KEY_FILE).read_text(encoding="utf-8") == key
    except OSError:
        return False


def clear_output(out, keep_hulls):
    """Empty the output folder, except for the hulls in `keep_hulls`."""
    for item in out.iterdir():
        if item.name != "hulls":
            if item.is_dir():
                shutil.rmtree(item)
            else:
                item.unlink()
    for item in (out / "hulls").iterdir():
        if item.is_dir() and item.name not in keep_hulls:
            shutil.rmtree(item)


def localized_file(name, lang):
    """The file a language's page links to: `report.html` in English, `report.tr.html` in Turkish."""
    if lang == LANGS[0]:
        return name
    stem, dot, suffix = name.rpartition(".")
    return f"{stem}.{lang}{dot}{suffix}"


def hull_name(description):
    """A hull's name, as `seacraft example --list` begins its description: `name: what it is`."""
    return description.partition(": ")[0].strip()


def example_descriptions(seacraft):
    """`{hull_id: {lang: description}}` from `seacraft example --list` in each language."""
    listing = {}
    for lang in LANGS:
        for line in run(seacraft, "--lang", lang, "example", "--list").splitlines():
            hull_id, _, rest = line.partition(" ")
            listing.setdefault(hull_id, {})[lang] = rest.strip()
    return listing


def hull_data(out, hull_id):
    return json.loads((out / "hulls" / hull_id / "data.json").read_text(encoding="utf-8"))


def build_hull(seacraft, hull, descriptions, out):
    hull_id = hull["id"]
    hull_dir = out / "hulls" / hull_id
    hull_dir.mkdir(parents=True)
    fef = hull_dir / f"{hull_id}.fef"
    shutil.copy(PROGRAM / "examples" / f"{hull_id}.fef", fef)
    shutil.copy(PROGRAM / "examples" / f"{hull_id}.seacraft.json", hull_dir)

    project = json.loads((hull_dir / f"{hull_id}.seacraft.json").read_text(encoding="utf-8"))
    info = run(seacraft, "info", fef.name, cwd=hull_dir)
    names = {lang: hull_name(text) for lang, text in descriptions.items()}
    if names["en"] != info.splitlines()[0].strip():
        sys.exit(f"{hull_id}: `seacraft example --list` no longer starts with the hull's name, {names['en']!r}")
    length, beam, draft = parse_info(info)
    if project.get("kg") is None:
        sys.exit(f"{hull_id}.seacraft.json has no kg, and the site will not assume one")
    kg = float(project["kg"])

    with tempfile.TemporaryDirectory() as scratch:
        obj = Path(scratch) / "hull.obj"
        run(seacraft, "export", fef.name, "-o", obj, "--level", 3, "--no-mirror", "--up", "z", cwd=hull_dir)
        lines = geometry(obj, draft)

    requirements = project.get("requirements") or {}
    depth = min(requirements.get("depth") or lines["z1"], lines["z1"])
    depth = max(depth, draft * 1.2)
    drafts = drafts_for(draft, min(depth, lines["z1"] * 0.98))
    rows = json.loads(
        run(seacraft, "table", fef.name, "--drafts", nums(*drafts, digits=6), "--json", cwd=hull_dir)
    )["rows"]
    table = [
        {
            "draft": round(r["draft"], 4),
            "displacement": sig(r["displacement_t"]),
            "cb": round(r["block_coefficient"], 4),
            "waterplane": sig(r["waterplane_area_m2"]),
            "km": round(r["km_transverse"], 3),
            "wetted": sig(r["wetted_surface_m2"]),
        }
        for r in rows
    ]

    sac = json.loads(run(seacraft, "sac", fef.name, "--stations", 40, "--json", cwd=hull_dir))

    # The documents a reader can open: SeaCraft's own files, unchanged, in each language it writes them in.
    for lang in LANGS:
        run(seacraft, "--lang", lang, "report", fef.name, "-o", localized_file("report.html", lang), cwd=hull_dir)
        run(seacraft, "--lang", lang, "linesplan", fef.name, "-o", localized_file("lines.svg", lang), cwd=hull_dir)
    run(seacraft, "export", fef.name, "-o", "viewer.html", cwd=hull_dir)

    data = {
        "id": hull_id,
        "name": names,
        "description": descriptions,
        "length": length,
        "beam": beam,
        "draft": draft,
        "kg": kg,
        "lines": lines,
        "table": table,
        "commands": {
            "hydro": shown(hull_id, "hydro", "--kg", f"{kg:g}"),
            "lines": shown(hull_id, "linesplan", "-o", "lines.svg"),
            "sac": shown(hull_id, "sac", "--stations", 40),
            "report": shown(hull_id, "report", "-o", "report.html"),
            "gui": f"cargo run --release -p seacraft-gui -- --open examples/{hull_id}.fef",
            "hydro_source": f"cargo run --release --bin seacraft -- hydro examples/{hull_id}.fef --kg {kg:g}",
        },
        "sac": {
            "draft": round(sac["draft"], 3),
            "points": [[round(s["x"], 3), round(s["area"], 4)] for s in sac["stations"]],
        },
        "stability": stability_curves(seacraft, hull_id, hull_dir, kg),
        "resistance": resistance_curve(
            seacraft, hull_id, hull_dir, hull["resistance"], default_speeds(project, length)
        ),
        "motions": motions(seacraft, hull_id, hull_dir, draft, kg) if hull["motions"] else None,
        "waves": wave_steps(seacraft, hull_id, hull_dir, length),
        "heel": heel_steps(seacraft, hull_id, hull_dir, kg, beam, lines["stations"]),
        # The size of each file the page links to, by language where seacraft writes one per language.
        "files": {
            "report": {lang: (hull_dir / localized_file("report.html", lang)).stat().st_size for lang in LANGS},
            "lines": {lang: (hull_dir / localized_file("lines.svg", lang)).stat().st_size for lang in LANGS},
            "viewer": (hull_dir / "viewer.html").stat().st_size,
            "fef": fef.stat().st_size,
        },
    }
    (hull_dir / "data.json").write_text(
        json.dumps(data, separators=(",", ":"), ensure_ascii=False), encoding="utf-8"
    )
    print(f"  {hull_id}: {len(table)} drafts, {(hull_dir / 'data.json').stat().st_size // 1024} KiB data")


# The example the "Build it" section shows, and how its pieces are nested.
BUILDER_HULL = "skiff-4m"
BUILDER_NEST = "blanks"


def builder_files(seacraft, out, lang):
    """The skiff's plywood panels nested onto sheets, as seacraft writes them in `lang`, into
    `out/build/` for English and `out/build/<lang>/` for the others.

    Fails the build when a panel does not lie flat or no sheet file is written, since the section says both.
    """
    folder = out / "build" / lang_dir(lang)
    folder.mkdir(parents=True)
    fef = folder / f"{BUILDER_HULL}.fef"
    shutil.copy(PROGRAM / "examples" / fef.name, fef)
    panels = ("--lang", lang, "panels", fef.name, "--nest", BUILDER_NEST)
    pdf = json.loads(run(seacraft, *panels, "-o", "panels.pdf", "--json", cwd=folder))
    svg = json.loads(run(seacraft, *panels, "-o", "panels.svg", "--json", cwd=folder))
    fef.unlink()
    flat = [p["panel"] for p in svg["panels"] if not p["lies_flat"]]
    if flat:
        sys.exit(f"{BUILDER_HULL}: panels {', '.join(flat)} do not lie flat, and the site says they do")
    sheets = [Path(f).name for f in svg["sheet_files"]]
    if not sheets:
        sys.exit(f"{BUILDER_HULL}: seacraft panels wrote no sheet files")
    files = ["panels.pdf", "panels.svg", *sheets]
    index = {
        "command": " ".join(["seacraft", "panels", fef.name, "--nest", BUILDER_NEST, "-o", "panels.svg"]),
        "pdf_command": " ".join(["seacraft", "panels", fef.name, "--nest", BUILDER_NEST, "-o", "panels.pdf"]),
        "sheets": sheets,
        "layout": [{"length_m": s["length_m"], "width_m": s["width_m"]} for s in svg["layout"]["sheets"]],
        "plywood_sheets": svg["layout"]["plywood_sheets"],
        "panels": [{"panel": p["panel"], "copies": p["copies"], "lies_flat": p["lies_flat"]} for p in svg["panels"]],
        "pdf_pages": pdf.get("pages"),
        "sizes": {name: (folder / name).stat().st_size for name in files},
    }
    (folder / "index.json").write_text(json.dumps(index, ensure_ascii=False), encoding="utf-8")
    print(f"  build ({lang}): {len(sheets)} sheet files, {index['plywood_sheets']} plywood sheets")


# --- The page, once per language ------------------------------------------------------------------


def lang_dir(lang):
    """The folder a language's page and files live in, relative to the site's root: "" for English."""
    return "" if lang == LANGS[0] else f"{lang}/"


def site_strings(lang):
    """The page's strings in `lang`, from site/locales."""
    return json.loads((SITE_LOCALES / f"{lang}.json").read_text(encoding="utf-8"))


# The strings app.js needs: everything it writes into the page after it has loaded.
SCRIPT_PREFIXES = ("js.", "hull.", "method.")
STRING = re.compile(r"\{\{t:([a-z0-9._]+)\}\}")
VALUE = re.compile(r"\{\{([a-z_]+)\}\}")


def render_page(template, lang, values):
    """The page in `lang`: each `{{t:key}}` from the site's strings, then each `{{name}}` from `values`.

    A string may itself hold a `{{name}}`, such as the version. A key or value the template names that
    does not exist stops the build, so a page never goes out with a hole in it.
    """
    strings = site_strings(lang)

    def string(match):
        if match.group(1) not in strings:
            sys.exit(f"site/locales/{lang}.json has no {match.group(1)}")
        return strings[match.group(1)]

    text = STRING.sub(string, template)
    script = {k: v for k, v in strings.items() if k.startswith(SCRIPT_PREFIXES)}
    # Inside <script>, so no "</" may close it early.
    values = dict(values, strings=json.dumps(script, ensure_ascii=False, sort_keys=True).replace("</", "<\\/"))

    def value(match):
        if match.group(1) not in values:
            sys.exit(f"the page names {{{{{match.group(1)}}}}}, which build.py does not set")
        return values[match.group(1)]

    return VALUE.sub(value, text)


SITE_URL = "https://seacraft.app/"


def page_values(lang, version, commit, release):
    """The values one language's page is filled with, besides its strings."""
    root = "../" * lang_dir(lang).count("/")
    others = [other for other in LANGS if other != lang]
    switch = "".join(
        # ?lang= tells the page the reader chose it, even when the browser stores nothing and sends no referrer.
        f'<a class="lang-link" href="{root}{lang_dir(other)}?lang={other}" hreflang="{other}" lang="{other}" '
        f'data-lang="{other}">{html_escape(site_strings(other)["page.lang.name"])}</a>'
        for other in others
    )
    alternates = "\n".join(
        f'<link rel="alternate" hreflang="{other}" href="{SITE_URL}{lang_dir(other)}">' for other in LANGS
    ) + f'\n<link rel="alternate" hreflang="x-default" href="{SITE_URL}">'
    return {
        "lang": lang,
        "lang_dir": lang_dir(lang),
        "root": root,
        "url": SITE_URL + lang_dir(lang),
        "alternates": alternates,
        "languages": switch,
        "langs": json.dumps(list(LANGS)),
        "version": version,
        "commit": commit,
        "release": release,
        # The release this page's numbers come from, so its downloads give the same numbers.
        "release_url": f"https://github.com/havan/SeaCraft/releases/tag/v{release}",
    }


def html_escape(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


# The desktop screenshots the page shows, from docs/assets.
SCREENSHOTS = ("gui.png", "gui-waves.png", "gui-heel.png")


def copy_screenshots(source, out):
    """Copy the page's screenshots, failing the build when one is missing rather than leaving a broken image."""
    missing = [name for name in SCREENSHOTS if not (source / name).is_file()]
    if missing:
        sys.exit(f"missing screenshots in {source}: {', '.join(missing)}")
    for name in SCREENSHOTS:
        shutil.copy(source / name, out / name)


def main():
    global PROGRAM
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seacraft", default=str(ROOT / "target/release/seacraft"))
    parser.add_argument("--out", default=str(Path(__file__).resolve().parent / "_site"))
    parser.add_argument(
        "--program-root",
        default=str(ROOT),
        help="the checkout the program, example hulls and the program's catalogs come from (default: this one)",
    )
    parser.add_argument(
        "--rebuild-hulls",
        action="store_true",
        help="build every hull, even those whose binary, script and example files have not changed",
    )
    args = parser.parse_args()

    seacraft = Path(args.seacraft).resolve()
    out = Path(args.out).resolve()
    PROGRAM = Path(args.program_root).resolve()
    # hulls/index.json marks a folder this script made. It is there from the start of a build, so a
    # build that fails part way can be run again and keeps the hulls it finished.
    index = out / "hulls" / "index.json"
    if out.exists() and any(out.iterdir()) and not index.is_file():
        sys.exit(f"refusing to delete {out}: it is not empty and is not a site this script built")

    version = run(seacraft, "--version").split()[-1]
    release = release_version(PROGRAM / "Cargo.toml")
    listing = example_descriptions(seacraft)
    for hull in HULLS:
        if set(listing.get(hull["id"], {})) != set(LANGS):
            sys.exit(f"`seacraft example --list` has no hull {hull['id']} in every language")

    keys = {hull["id"]: hull_key(seacraft, hull["id"]) for hull in HULLS}
    kept = set()
    if not args.rebuild_hulls:
        kept = {hull_id for hull_id, key in keys.items() if hull_is_current(out / "hulls" / hull_id, key)}
    index.parent.mkdir(parents=True, exist_ok=True)
    index.touch()
    clear_output(out, kept)

    for hull in HULLS:
        hull_id = hull["id"]
        if hull_id in kept:
            print(f"  {hull_id}: kept, nothing it depends on has changed")
            continue
        build_hull(seacraft, hull, listing[hull_id], out)
        (out / "hulls" / hull_id / KEY_FILE).write_text(keys[hull_id], encoding="utf-8")

    for lang in LANGS:
        builder_files(seacraft, out, lang)

    # The commit the numbers come from: the program's, which is this checkout's unless --program-root says otherwise.
    commit = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"], capture_output=True, encoding="utf-8", cwd=PROGRAM
    ).stdout.strip()
    manifest = {
        "version": version,
        "commit": commit,
        "hulls": [h["id"] for h in HULLS],
        # The hulls whose wave pattern is shown, so the others can point to them.
        "waves_hulls": [h["id"] for h in HULLS if hull_data(out, h["id"])["waves"]["fits"]],
    }
    index.write_text(json.dumps(manifest, ensure_ascii=False), encoding="utf-8")

    template = (SRC / "index.html").read_text(encoding="utf-8")
    for lang in LANGS:
        folder = out / lang_dir(lang)
        folder.mkdir(parents=True, exist_ok=True)
        page = render_page(template, lang, page_values(lang, version, commit, release))
        (folder / "index.html").write_text(page, encoding="utf-8")
    for item in SRC.iterdir():
        if item.is_file() and item.name != "index.html":
            shutil.copy(item, out / item.name)
    assets = out / "assets"
    assets.mkdir()
    copy_screenshots(ROOT / "docs" / "assets", assets)
    fonts = out / "fonts"
    fonts.mkdir()
    for item in FONTS.iterdir():
        shutil.copy(item, fonts / item.name)
    print(f"built SeaCraft {version} ({commit}) into {out} (hulls built: {len(HULLS) - len(kept)}, kept: {len(kept)})")


if __name__ == "__main__":
    main()
