#!/usr/bin/env python3
"""Build the SeaCraft website into a folder that GitHub Pages can serve.

Every number and hull drawing on the site comes from running the `seacraft` binary on the example
hulls in `examples/`, so the site cannot drift from the program. The curves in the drawings are cut
from the mesh `seacraft export` writes; this script draws them and measures nothing. Standard library
only.

    cargo build --release --bin seacraft
    python3 site/build.py --seacraft target/release/seacraft --out site/_site
"""

import argparse
import csv
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

# Page order. `resistance` picks the method whose range of hulls this example falls in, and `motions`
# is off for the planing hull, because strip theory does not describe a boat on the plane.
HULLS = [
    {"id": "trawler-22m", "label": "Trawler, 22 m", "resistance": "holtrop", "motions": True},
    {"id": "yacht-10m", "label": "Yacht, 10 m", "resistance": "dsyhs", "motions": True},
    {"id": "kayik-9m", "label": "Kayık, 9 m", "resistance": "holtrop", "motions": True},
    {"id": "runabout-6m", "label": "Runabout, 6.5 m", "resistance": "savitsky", "motions": False},
    {"id": "container-230m", "label": "Container ship, 230 m", "resistance": "holtrop", "motions": True},
    {"id": "wigley", "label": "Wigley hull, 10 m", "resistance": "holtrop", "motions": True},
]
RESISTANCE_NAMES = {
    "holtrop": "Holtrop and Mennen",
    "dsyhs": "the Delft yacht series",
    "savitsky": "Savitsky",
}


def run(seacraft, *args, cwd=None):
    """Run seacraft and return stdout, failing the build on any error."""
    done = subprocess.run([str(seacraft), *map(str, args)], capture_output=True, text=True, cwd=cwd)
    if done.returncode != 0:
        sys.exit(f"seacraft {' '.join(map(str, args))} failed:\n{done.stderr}")
    return done.stdout


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
    for line in Path(path).read_text().splitlines():
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
        x = x0 + length * (k + 0.001) / 161.002
        heights = [p[2] for seg in mesh.cut(0, x) for p in seg]
        if heights:
            top.append((x, max(heights)))
            bottom.append((x, min(heights)))
    outline = simplify(top, tol) + simplify(bottom, tol)[::-1]
    profile_outline = path([outline], lambda p: (p[0], -p[1]), places) + "Z"

    # Outline in the body plan: the widest point at each height, mirrored.
    widths = []
    for k in range(81):
        z = z0 + (z1 - z0) * (k + 0.001) / 81.002
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
    with open(hull_dir / "gz.csv", newline="") as handle:
        for row in csv.DictReader(handle):
            by_displacement.setdefault(float(row["displacement_t"]), []).append(
                [float(row["heel_deg"]), float(row["gz_m"])]
            )
    (hull_dir / "gz.csv").unlink()
    series = [
        {"name": f"{sig(d, 4):g} t", "points": [[h, round(g, 4)] for h, g in pts]}
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
        args = ("planing", f"{hull_id}.fef", "--speeds", joined, "--json")
        command = shown(hull_id, "planing", "--speeds", joined)
    else:
        args = ("resistance", f"{hull_id}.fef", "--method", method, "--speeds", joined, "--json")
        command = shown(hull_id, "resistance", "--method", method, "--speeds", joined)
    data = json.loads(run(seacraft, *args, cwd=hull_dir))
    points = []
    for row in data["speeds"]:
        newtons = row.get("rt_n", row.get("resistance_n"))
        if newtons is not None:
            points.append([row["speed_kn"], round(newtons / 1000, 3)])
    return {
        "command": command,
        "method": RESISTANCE_NAMES[method],
        "points": points,
        "warnings": warning_text(data),
    }


def motions(seacraft, hull_id, hull_dir, draft, kg):
    args = ("seakeeping", f"{hull_id}.fef", "--draft", draft, "--kg", kg, "--points", 40, "--json")
    data = json.loads(run(seacraft, *args, cwd=hull_dir))
    rows = sorted(data["frequencies"], key=lambda f: f["lambda_over_l"])
    return {
        "command": shown(hull_id, "seakeeping", "--draft", f"{draft:g}", "--kg", f"{kg:g}", "--points", 40),
        "heave": [[round(f["lambda_over_l"], 3), round(f["heave_amp"], 4)] for f in rows],
        "pitch": [[round(f["lambda_over_l"], 3), round(f["pitch_amp"], 4)] for f in rows],
        "warnings": warning_text(data),
    }


def default_speeds(project, length):
    speeds = project.get("report", {}).get("speeds_kn") or []
    if len(speeds) >= 4:
        return speeds
    # Froude number 0.15 to 0.40 when the example does not name its own speeds.
    top = 0.40 * (9.81 * length) ** 0.5 * 1.944
    return [round(top * k / 8, 1) for k in range(3, 9)]


def build_hull(seacraft, hull, description, out):
    hull_id = hull["id"]
    hull_dir = out / "hulls" / hull_id
    hull_dir.mkdir(parents=True)
    fef = hull_dir / f"{hull_id}.fef"
    shutil.copy(ROOT / "examples" / f"{hull_id}.fef", fef)
    shutil.copy(ROOT / "examples" / f"{hull_id}.seacraft.json", hull_dir)

    project = json.loads((hull_dir / f"{hull_id}.seacraft.json").read_text())
    info = run(seacraft, "info", fef.name, cwd=hull_dir)
    name = info.splitlines()[0].strip()
    length, beam, draft = parse_info(info)
    kg = float(project.get("kg") or project.get("report", {}).get("kg") or 0) or round(draft, 2)

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

    # The documents a reader can open: SeaCraft's own files, unchanged.
    run(seacraft, "report", fef.name, "-o", "report.html", cwd=hull_dir)
    run(seacraft, "export", fef.name, "-o", "viewer.html", cwd=hull_dir)
    run(seacraft, "linesplan", fef.name, "-o", "lines.svg", cwd=hull_dir)

    data = {
        "id": hull_id,
        "label": hull["label"],
        "name": name,
        "description": description,
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
        "files": {
            "report": (hull_dir / "report.html").stat().st_size,
            "viewer": (hull_dir / "viewer.html").stat().st_size,
            "lines": (hull_dir / "lines.svg").stat().st_size,
            "fef": fef.stat().st_size,
        },
    }
    (hull_dir / "data.json").write_text(json.dumps(data, separators=(",", ":"), ensure_ascii=False))
    print(f"  {hull_id}: {len(table)} drafts, {(hull_dir / 'data.json').stat().st_size // 1024} KiB data")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seacraft", default=str(ROOT / "target/release/seacraft"))
    parser.add_argument("--out", default=str(Path(__file__).resolve().parent / "_site"))
    args = parser.parse_args()

    seacraft = Path(args.seacraft).resolve()
    out = Path(args.out).resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    version = run(seacraft, "--version").split()[-1]
    listing = {}
    for line in run(seacraft, "example", "--list").splitlines():
        hull_id, _, rest = line.partition(" ")
        listing[hull_id] = rest.strip()
    for hull in HULLS:
        if hull["id"] not in listing:
            sys.exit(f"`seacraft example --list` has no hull {hull['id']}")
        build_hull(seacraft, hull, listing[hull["id"]], out)

    commit = subprocess.run(
        ["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, cwd=ROOT
    ).stdout.strip()
    manifest = {
        "version": version,
        "commit": commit,
        "hulls": [{"id": h["id"], "label": h["label"]} for h in HULLS],
    }
    (out / "hulls" / "index.json").write_text(json.dumps(manifest, ensure_ascii=False))

    for item in SRC.iterdir():
        if item.is_file():
            text = None
            if item.suffix == ".html":
                text = item.read_text().replace("{{version}}", version).replace("{{commit}}", commit)
            if text is None:
                shutil.copy(item, out / item.name)
            else:
                (out / item.name).write_text(text)
    assets = out / "assets"
    assets.mkdir()
    shutil.copy(ROOT / "docs" / "assets" / "gui.png", assets / "gui.png")
    fonts = out / "fonts"
    fonts.mkdir()
    for item in FONTS.iterdir():
        shutil.copy(item, fonts / item.name)
    print(f"built SeaCraft {version} ({commit}) into {out}")


if __name__ == "__main__":
    main()
