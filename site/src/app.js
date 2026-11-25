/* The hull on the page is data: site/build.py runs seacraft on each example and writes one JSON file per
   hull. This script draws that data. It computes no hydrostatics of its own.

   The page comes in several languages. Its words are in the <script id="strings"> build.py wrote from
   site/locales, and whatever seacraft said is in the hull's data once per language. */
"use strict";

const NS = "http://www.w3.org/2000/svg";
const $ = (id) => document.getElementById(id);

const LANG = document.documentElement.lang;
// The site's root and this language's folder in it, relative to this page: "" and "" in English.
const ROOT = document.documentElement.dataset.root || "";
const LANG_DIR = document.documentElement.dataset.langDir || "";
const STRINGS = JSON.parse($("strings").textContent);

/* A string of the page in its language, with each {name} filled from `values`. */
function t(key, values = {}) {
  const text = STRINGS[key];
  if (text === undefined) {
    console.error(`no string ${key}`);
    return key;
  }
  return text.replace(/\{([a-z_]+)\}/g, (whole, name) => (name in values ? values[name] : whole));
}

/* What seacraft said, in this page's language: build.py stores it as {lang: text}. */
const said = (texts) => texts[LANG] ?? texts.en;

/* A document seacraft writes once per language: report.html in English, report.tr.html in Turkish. */
const localizedFile = (name) => (LANG_DIR ? name.replace(/\.([a-z]+)$/, `.${LANG}.$1`) : name);

/* Remember a language chosen with the switch, and keep the place on the page. */
for (const link of document.querySelectorAll(".lang-link")) {
  link.addEventListener("click", () => {
    try { localStorage.setItem("seacraft-lang", link.dataset.lang); } catch (e) { /* Private mode: not remembered. */ }
    if (location.hash) link.hash = location.hash;
  });
}
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
const SERIES = ["var(--series-1)", "var(--series-2)", "var(--series-3)"];
const cache = new Map();

const state = { hull: null, row: 0, shown: 0, raf: 0, request: 0, manifest: null, waveFn: null };

function el(name, attrs = {}, parent) {
  const node = document.createElementNS(NS, name);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  if (parent) parent.appendChild(node);
  return node;
}

function html(tag, props = {}, parent) {
  const node = Object.assign(document.createElement(tag), props);
  if (parent) parent.appendChild(node);
  return node;
}

// The decimal point is always ".", as in seacraft's own output; thousands are grouped as the language does.
const formatNumber = (value, options) =>
  value.toLocaleString("en-GB", options).replace(/,/g, t("js.number.group"));

const niceNumber = (value, digits = 2) =>
  formatNumber(value, { minimumFractionDigits: digits, maximumFractionDigits: digits });

function significant(value, sig = 4) {
  if (value === 0) return "0";
  const digits = Math.max(0, sig - 1 - Math.floor(Math.log10(Math.abs(value))));
  return formatNumber(value, { maximumFractionDigits: digits, minimumFractionDigits: 0 });
}

function fileSize(bytes) {
  if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function niceTicks(low, high, count) {
  const span = high - low || 1;
  const raw = span / count;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * magnitude).find((s) => s >= raw);
  const ticks = [];
  for (let v = Math.ceil(low / step - 1e-9) * step; v <= high + step * 1e-6; v += step) {
    ticks.push(Math.abs(v) < step * 1e-9 ? 0 : v);
  }
  return ticks;
}

/* --- The sea: profile and body plan, one vertical scale, one waterline --------------------------- */

const sea = { svg: $("sea"), parts: null, unitsPerPixel: 0.05, box: null };

function narrowStage() {
  return $("stage").clientWidth < 620;
}

function buildSea(hull) {
  const svg = sea.svg;
  svg.textContent = "";
  const g = hull.lines;
  const gap = (g.x1 - g.x0) * 0.05 + g.y1 * 0.6;
  const bodyCentre = g.x1 + gap + g.y1;
  const narrow = narrowStage();

  // Room for the marks and the grip is fixed on screen, so work out the scale first.
  const size = svg.getBoundingClientRect();
  const marksPx = 46;
  const gripPx = 40;
  const left = narrow ? bodyCentre - g.y1 : g.x0;
  const extent = bodyCentre + g.y1 - left;
  const height = g.z1 - g.z0;
  const margin = height * 0.03;
  const scale = Math.max(1e-6, Math.min(
    (size.width - marksPx - gripPx) / extent,
    size.height / (height + 2 * margin)
  ));
  const box = { x: left - marksPx / scale, w: extent + (marksPx + gripPx) / scale };
  box.y = -g.z1 - margin;
  box.h = height + 2 * margin;
  svg.setAttribute("viewBox", `${box.x} ${box.y} ${box.w} ${box.h}`);
  svg.setAttribute("preserveAspectRatio", "xMidYMax meet");
  sea.box = box;

  const defs = el("defs", {}, svg);
  const clipAir = el("rect", { x: -1e5, width: 2e5, y: -1e5, height: 1e5 }, el("clipPath", { id: "clip-air" }, defs));
  const clipSea = el("rect", { x: -1e5, width: 2e5, y: 0, height: 1e5 }, el("clipPath", { id: "clip-sea" }, defs));

  // Styled by attribute: a <use> clone does not reliably match CSS rules that name its ancestors.
  const art = el("g", {
    id: "art", fill: "none", stroke: "currentColor", "stroke-width": 1,
    "vector-effect": "non-scaling-stroke", "stroke-linejoin": "round",
  }, defs);
  const line = (d, parent, width = 1) => el("path", { d, "stroke-width": width, "vector-effect": "non-scaling-stroke" }, parent);
  const bodyShift = `translate(${bodyCentre} 0)`;
  if (!narrow) {
    for (const b of g.buttocks) line(b.profile, art);
    line(g.profile_outline, art, 1.75);
  }
  const body = el("g", { transform: bodyShift }, art);
  for (const s of g.stations) line(s.body, body);
  line(g.body_outline, body, 1.75);

  // Draft marks, as they are painted on a stem.
  const ticks = niceTicks(0, g.z1, Math.max(1, Math.floor((g.z1 * scale) / 34)));
  const step = ticks[1] || g.z1;
  const markX = left - 18 / scale;
  const marks = el("g", {}, art);
  sea.marks = [];
  for (let z = 0; z <= g.z1 + 1e-6; z += step) {
    const label = el("text", {
      x: markX, y: -z, "text-anchor": "end", "dominant-baseline": "central",
      fill: "currentColor", stroke: "none", "font-family": "Big Shoulders, sans-serif", "font-weight": 700,
    }, marks);
    label.textContent = `${+z.toFixed(2)}`;
    sea.marks.push(label);
  }
  line(`M${left - 10 / scale},${-g.z1}V${-g.z0}`, marks, 1.5);

  const outlineFill = (cls, clip) => {
    const group = el("g", { "clip-path": `url(#${clip})` }, svg);
    if (!narrow) el("path", { d: g.profile_outline, class: cls }, group);
    el("path", { d: g.body_outline, class: cls, transform: bodyShift }, group);
  };
  outlineFill("fill-air", "clip-air");
  const water = el("rect", { x: -1e5, width: 2e5, y: 0, height: 1e5, class: "water" }, svg);
  outlineFill("fill-sea", "clip-sea");

  el("use", { href: "#art", class: "in-air", "clip-path": "url(#clip-air)" }, svg);
  el("use", { href: "#art", class: "in-sea", "clip-path": "url(#clip-sea)" }, svg);

  const waterline = el("line", { x1: -1e5, x2: 1e5, y1: 0, y2: 0, class: "wl" }, svg);
  const hint = el("g", { class: "hint" }, svg);
  el("rect", {}, hint);
  el("text", { "text-anchor": "end", "dominant-baseline": "central" }, hint).textContent = t("js.sea.hint");
  const grip = el("g", {}, svg);
  el("circle", { class: "grip", r: 1 }, grip);
  el("path", { class: "grip-mark" }, grip);
  const grab = el("rect", { x: -1e5, width: 2e5, class: "grab" }, svg);

  sea.parts = { clipAir, clipSea, water, waterline, hint, grip, grab };
  sizeSea();
  bindDrag(grab);
}

/* Sizes that must stay constant on screen whatever the hull's scale. */
function sizeSea() {
  if (!state.hull || !sea.parts) return;
  const rect = sea.svg.getBoundingClientRect();
  const box = sea.box;
  const scale = Math.min(rect.width / box.w, rect.height / box.h);
  if (!scale) return;
  sea.unitsPerPixel = 1 / scale;
  const u = sea.unitsPerPixel;
  const fontSize = 21 * u;
  for (const t of sea.marks) t.setAttribute("font-size", fontSize);
  const { hint, grip, grab } = sea.parts;
  const hintText = hint.querySelector("text");
  hintText.setAttribute("font-size", 14 * u);
  hintText.setAttribute("x", -22 * u);
  const hintWidth = hintText.getComputedTextLength();
  const hintBox = hint.querySelector("rect");
  hintBox.setAttribute("x", -hintWidth - 34 * u);
  hintBox.setAttribute("y", -12 * u);
  hintBox.setAttribute("width", hintWidth + 34 * u);
  hintBox.setAttribute("height", 24 * u);
  grab.setAttribute("height", 56 * u);
  const circle = grip.querySelector("circle");
  circle.setAttribute("r", 15 * u);
  grip.querySelector("path").setAttribute(
    "d",
    `M${-5 * u},${-3 * u}L0,${-8 * u}L${5 * u},${-3 * u}M${-5 * u},${3 * u}L0,${8 * u}L${5 * u},${3 * u}`
  );
  placeWater(state.shown);
}

function draftAt(position) {
  const table = state.hull.table;
  const low = Math.max(0, Math.min(table.length - 1, Math.floor(position)));
  const high = Math.min(table.length - 1, low + 1);
  return table[low].draft + (table[high].draft - table[low].draft) * (position - low);
}

function placeWater(position) {
  const y = -draftAt(position) - (state.hull.lines.z0 || 0);
  const { clipAir, clipSea, water, waterline, hint, grip, grab } = sea.parts;
  clipAir.setAttribute("y", y - 1e5);
  clipSea.setAttribute("y", y);
  water.setAttribute("y", y);
  waterline.setAttribute("y1", y);
  waterline.setAttribute("y2", y);
  const u = sea.unitsPerPixel;
  const box = sea.box;
  grab.setAttribute("y", y - 28 * u);
  const gripX = box.x + box.w - 15 * u;
  grip.setAttribute("transform", `translate(${gripX} ${y})`);
  hint.setAttribute("transform", `translate(${gripX} ${y})`);
}

function bindDrag(grab) {
  grab.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    grab.setPointerCapture(event.pointerId);
    sea.svg.classList.add("touched");
    const move = (e) => {
      const matrix = sea.svg.getScreenCTM();
      if (!matrix) return;
      const z = -(e.clientY - matrix.f) / matrix.d;
      const table = state.hull.table;
      let best = 0;
      for (let i = 0; i < table.length; i++) {
        if (Math.abs(table[i].draft - z) < Math.abs(table[best].draft - z)) best = i;
      }
      setRow(best, false);
    };
    const up = () => {
      grab.removeEventListener("pointermove", move);
      grab.removeEventListener("pointerup", up);
      grab.removeEventListener("pointercancel", up);
    };
    grab.addEventListener("pointermove", move);
    grab.addEventListener("pointerup", up);
    grab.addEventListener("pointercancel", up);
    move(event);
  });
}

function setRow(index, animate) {
  if (!state.hull) return; // Nothing loaded yet.
  state.row = index;
  const row = state.hull.table[index];
  $("draft").value = index;
  $("draft").setAttribute("aria-valuetext", t("js.sea.draft_valuetext", { draft: niceNumber(row.draft) }));
  const unit = (v, u) => `${v}<small>${u}</small>`;
  $("r-draft").innerHTML = unit(niceNumber(row.draft), "m");
  $("r-displacement").innerHTML = unit(significant(row.displacement), "t");
  $("r-cb").textContent = niceNumber(row.cb, 3);
  $("r-waterplane").innerHTML = unit(significant(row.waterplane), "m²");
  $("r-km").innerHTML = unit(niceNumber(row.km), "m");
  $("r-wetted").innerHTML = unit(significant(row.wetted), "m²");
  cancelAnimationFrame(state.raf);
  if (!animate || reducedMotion.matches) {
    state.shown = index;
    placeWater(index);
    return;
  }
  const from = state.shown;
  const start = performance.now();
  const duration = 1100;
  const frame = (now) => {
    const t = Math.max(0, Math.min(1, (now - start) / duration)); // A frame's time can precede `start`.
    const eased = 1 - (1 - t) ** 3;
    state.shown = from + (index - from) * eased;
    placeWater(state.shown);
    if (t < 1) state.raf = requestAnimationFrame(frame);
  };
  state.raf = requestAnimationFrame(frame);
}

/* --- The lines plan figure ------------------------------------------------------------------------ */

function drawLinesFigure(hull) {
  const svg = $("lines-figure");
  svg.textContent = "";
  const g = hull.lines;
  const length = g.x1 - g.x0;
  const height = g.z1 - g.z0;
  const pad = length * 0.015;
  const gap = Math.max(length * 0.04, height * 0.25);
  const totalW = length + gap + 2 * g.y1 + 2 * pad;
  const planTop = height + gap;
  const totalH = Math.max(planTop + g.y1, height) + 2 * pad;
  svg.setAttribute("viewBox", `${g.x0 - pad} ${-g.z1 - pad} ${totalW} ${totalH}`);

  const grid = (x1, y1, x2, y2) => el("line", { x1, y1, x2, y2, class: "grid-line" }, svg);
  // Station lines through profile and plan, waterlines through profile and body plan.
  const bodyCentre = g.x1 + gap + g.y1;
  const stationXs = g.stations.map((s) => s.x);
  for (const x of stationXs) grid(x, -g.z1, x, -g.z0);
  const planY = -g.z1 + planTop + g.y1;
  for (const x of stationXs) grid(x, planY - g.y1, x, planY);
  for (const w of g.waterlines) {
    grid(g.x0, -w.z, g.x1, -w.z);
    grid(bodyCentre - g.y1, -w.z, bodyCentre + g.y1, -w.z);
  }
  grid(g.x0, -g.z0, g.x1, -g.z0);
  grid(g.x0, planY, g.x1, planY);
  grid(bodyCentre, -g.z1, bodyCentre, -g.z0);

  for (const b of g.buttocks) el("path", { d: b.profile, class: "buttock" }, svg);
  el("path", { d: g.profile_outline, class: "outline" }, svg);

  // Half-breadth plan, bow to the right, centreline along the bottom.
  const plan = el("g", { transform: `translate(0 ${planY}) scale(1 -1)` }, svg);
  for (const w of g.waterlines) el("path", { d: w.plan, class: "waterline" }, plan);

  const body = el("g", { transform: `translate(${bodyCentre} 0)` }, svg);
  for (const s of g.stations) el("path", { d: s.body, class: "station" }, body);
}

/* --- Where it heels: one seacraft heel run per weight position ------------------------------------ */

// The station's half section as polylines of [y, -z]; build.py cut it from the mesh.
const polylines = (d) =>
  d.split("M").filter(Boolean).map((part) => part.split("L").map((p) => p.split(",").map(Number)));

// Where the solved waterplane (n . p + d = 0) crosses the centreline at the station: the point the
// drawing turns the hull about, so the water stays level on screen.
function waterAtCentre(step, x) {
  const [nx, , nz, d] = step.waterplane;
  return -(nx * x + d) / nz;
}

// A point of the hull, [y, -z] in the hull's own axes, on screen once heeled.
function heeled([y, up], degrees, centre) {
  const a = (degrees * Math.PI) / 180;
  const v = up + centre;
  return [y * Math.cos(a) - v * Math.sin(a), y * Math.sin(a) + v * Math.cos(a)];
}

const heel = { hull: null, parts: null, angle: 0, centre: 0, raf: 0 };

// A settled state the figure can draw: near ±90° the waterplane stands on edge and has no level line.
const drawable = (step) => step.kind === "stable" && Math.abs(step.waterplane[2]) > 1e-6;

// Turn the hull about the user-space origin with the SVG transform attribute: a CSS transform on an SVG
// element turns about the viewBox corner in some browsers, which moved the hull out of its frame.
function turnHull(groups, angle, centre) {
  cancelAnimationFrame(heel.raf);
  const apply = (a, c) => {
    for (const g of groups) g.setAttribute("transform", `rotate(${a}) translate(0 ${c})`);
    heel.angle = a;
    heel.centre = c;
  };
  const from = [heel.angle, heel.centre];
  if (reducedMotion.matches || !heel.started) {
    heel.started = true;
    apply(angle, centre);
    return;
  }
  const start = performance.now();
  const frame = (now) => {
    const t = Math.max(0, Math.min(1, (now - start) / 300));
    const eased = 1 - (1 - t) ** 3;
    apply(from[0] + (angle - from[0]) * eased, from[1] + (centre - from[1]) * eased);
    if (t < 1) heel.raf = requestAnimationFrame(frame);
  };
  heel.raf = requestAnimationFrame(frame);
}

function buildHeel(hull) {
  const svg = $("heel-figure");
  svg.textContent = "";
  const data = hull.heel;
  const stable = data.steps.filter(drawable);
  const upright = stable.length ? waterAtCentre(stable[0], data.station.x) : hull.lines.z0 + hull.draft;
  const halves = polylines(data.station.body).map((line) => line.map(([y, up]) => [Math.abs(y), up]));
  // Only a part that reaches the centreline closes with its mirror into a section; a bulwark rail
  // standing on its own is drawn as a line.
  const shapes = halves
    .filter((line) => line.some(([y]) => y < 1e-3))
    .map((line) => [...line, ...line.slice().reverse().map(([y, up]) => [-y, up])]);
  const toPath = (points) => `M${points.map((p) => p.map((v) => v.toFixed(4)).join(",")).join("L")}`;

  // Frame every pose the slider can show, so the figure never rescales between positions.
  const marks = data.steps.flatMap((s) => [[s.y, -data.z]]);
  const poses = [[0, upright], ...stable.map((s) => [s.heel_deg, waterAtCentre(s, data.station.x)])];
  let [left, right, top, bottom] = [Infinity, -Infinity, Infinity, -Infinity];
  for (const [angle, centre] of poses) {
    for (const point of [...halves.flat(), ...halves.flat().map(([y, up]) => [-y, up]), ...marks]) {
      const [x, y] = heeled(point, angle, centre);
      left = Math.min(left, x); right = Math.max(right, x);
      top = Math.min(top, y); bottom = Math.max(bottom, y);
    }
  }
  const pad = (right - left) * 0.08;
  const box = { x: left - pad, y: top - pad, w: right - left + 2 * pad, h: bottom - top + 2 * pad };
  svg.setAttribute("viewBox", `${box.x} ${box.y} ${box.w} ${box.h}`);

  const defs = el("defs", {}, svg);
  el("rect", { x: -1e4, y: -1e4, width: 2e4, height: 1e4 }, el("clipPath", { id: "heel-air" }, defs));
  el("rect", { x: -1e4, y: 0, width: 2e4, height: 1e4 }, el("clipPath", { id: "heel-sea" }, defs));
  const water = el("rect", { x: -1e4, y: 0, width: 2e4, height: 1e4, class: "heel-water" }, svg);
  const hulls = ["air", "sea"].map((side) => {
    const clip = el("g", { "clip-path": `url(#heel-${side})`, class: `heel-${side}` }, svg);
    const body = el("g", { class: "heel-hull" }, clip);
    for (const shape of shapes) el("path", { d: `${toPath(shape)}Z`, class: "heel-fill" }, body);
    for (const half of halves) {
      el("path", { d: toPath(half), class: "heel-line" }, body);
      el("path", { d: toPath(half.map(([y, up]) => [-y, up])), class: "heel-line" }, body);
    }
    return body;
  });
  const waterline = el("line", { x1: -1e4, x2: 1e4, y1: 0, y2: 0, class: "heel-wl" }, svg);

  // In the hull's axes, so they turn with it.
  const pins = el("g", { class: "heel-hull heel-pins" }, svg);
  const weight = el("rect", { class: "heel-weight" }, pins);
  const centreG = el("circle", { class: "heel-g0" }, pins);
  // On screen, level whatever the heel.
  const over = el("g", { class: "heel-over" }, svg);
  const action = el("line", { class: "heel-action" }, over);
  const lever = el("line", { class: "heel-lever" }, over);
  const g = el("circle", { class: "heel-g" }, over);
  const b = el("circle", { class: "heel-b" }, over);
  const labels = ["B", "G", "GZ"].map((text) => {
    const label = el("text", { class: `heel-label heel-label-${text.toLowerCase()}` }, over);
    label.textContent = text;
    return label;
  });

  heel.hull = hull;
  heel.started = false;
  heel.parts = { svg, box, upright, water, hulls, waterline, pins, weight, centreG, over, action, lever, g, b, labels };
}

function setHeel(index) {
  if (!heel.parts) return; // Nothing loaded yet.
  const { svg, box, upright, hulls, pins, weight, centreG, action, lever, g, b, labels } = heel.parts;
  const data = heel.hull.heel;
  const step = data.steps[index];
  const settled = drawable(step);
  const angle = settled ? step.heel_deg : 0;
  const centre = settled ? waterAtCentre(step, data.station.x) : upright;
  turnHull([...hulls, pins], angle, centre);
  svg.classList.toggle("unsettled", !settled);

  // Sizes fixed on screen, whatever the hull's size.
  // The figure is fitted inside its box, so whichever side fills decides the scale.
  const fit = Math.min((svg.clientWidth || 600) / box.w, (svg.clientHeight || 400) / box.h);
  const u = 1 / Math.max(1e-6, fit);
  weight.setAttribute("x", step.y - 6 * u);
  weight.setAttribute("y", -data.z - 12 * u);
  weight.setAttribute("width", 12 * u);
  weight.setAttribute("height", 12 * u);

  const summary = said(step.summary);
  $("heel-summary").textContent = t("js.heel.summary", { y: niceNumber(step.y), summary });
  $("cmd-heel").textContent = step.command;
  const metres = (v, places) => (v === null ? "–" : `${niceNumber(v, places)} m`);
  $("h-heel").textContent = settled ? `${niceNumber(step.heel_deg, 1)}°` : "–";
  $("h-gz").textContent = metres(step.gz_m, 3);
  $("h-arm").textContent = metres(step.heeling_arm_m, 3);
  $("h-draft").textContent = metres(step.draft, 3);
  const input = $("heel-weight");
  input.value = index;
  const where = t("js.heel.where", { mass: significant(data.mass_t, 2), y: niceNumber(step.y) });
  $("heel-label").textContent = t("js.heel.label", { where });
  input.setAttribute("aria-valuetext", `${where}. ${summary}`);
  const warnings = $("heel-warnings");
  warnings.textContent = "";
  solverWarnings(warnings, said(step.warnings));
  if (!settled) return;

  // B and G as seacraft placed them, and G moved to the centreline, where GZ is measured from.
  const [gx, gy] = heeled(step.g.map((v, i) => (i ? -v : v)), angle, centre);
  const [bx, by] = heeled(step.b.map((v, i) => (i ? -v : v)), angle, centre);
  const [ox, oy] = heeled([0, -step.g[1]], angle, centre);
  centreG.setAttribute("cx", 0);
  centreG.setAttribute("cy", -step.g[1]);
  centreG.setAttribute("r", 5 * u);
  for (const [dot, x, y] of [[g, gx, gy], [b, bx, by]]) {
    dot.setAttribute("cx", x);
    dot.setAttribute("cy", y);
    dot.setAttribute("r", 6 * u);
  }
  action.setAttribute("x1", bx); action.setAttribute("x2", bx);
  action.setAttribute("y1", by); action.setAttribute("y2", Math.min(gy, oy) - 30 * u);
  lever.setAttribute("x1", ox); lever.setAttribute("x2", bx);
  lever.setAttribute("y1", oy); lever.setAttribute("y2", oy);
  const font = 14 * u;
  const place = (label, x, y) => {
    label.setAttribute("x", x);
    label.setAttribute("y", y);
    label.setAttribute("font-size", font);
  };
  // B and G sit on the line of action, outboard of it; GZ is named on the other side of the lever.
  const side = bx >= ox ? 1 : -1;
  const outboard = side > 0 ? "start" : "end";
  place(labels[0], bx + 11 * u * side, by + 5 * u);
  place(labels[1], gx + 11 * u * side, gy - 6 * u);
  place(labels[2], side > 0 ? Math.min(ox, bx) - 10 * u : Math.max(ox, bx) + 10 * u, oy + 5 * u);
  const leverShown = Math.abs(bx - ox) >= 2 * u;
  lever.style.display = leverShown ? "" : "none";
  labels[2].style.display = leverShown ? "" : "none";
  labels[0].setAttribute("text-anchor", outboard);
  labels[1].setAttribute("text-anchor", outboard);
  labels[2].setAttribute("text-anchor", side > 0 ? "end" : "start");
}

function drawHeel(hull) {
  buildHeel(hull);
  const steps = hull.heel.steps;
  $("heel-weight").max = steps.length - 1;
  // Open on a heeled position that settles: the furthest stable one up to the middle of the deck.
  let start = 0;
  for (let i = 0; i <= Math.floor((steps.length - 1) / 2); i++) if (steps[i].kind === "stable") start = i;
  setHeel(start);
}

/* --- The wave pattern: one seacraft waves image per Froude number --------------------------------- */

function setWave(index) {
  const hull = state.hull;
  if (!hull || !hull.waves.fits) return; // Nothing loaded yet, or no pattern for this hull.
  const step = hull.waves.steps[index];
  const fn = step.fn.toFixed(2);
  const image = $("waves-image");
  // Hidden until this image has loaded, so the previous one never sits above these numbers.
  const src = `${ROOT}hulls/${hull.id}/${step.image}`;
  const scale = t("js.waves.scale", { clip: significant(step.clip_m, 2) });
  image.style.visibility = "hidden";
  image.onload = () => {
    if (image.getAttribute("src") !== src) return; // A later speed or hull has replaced this one.
    image.style.visibility = "";
    $("waves-scale").textContent = scale;
  };
  image.onerror = () => {
    if (image.getAttribute("src") !== src) return;
    $("waves-scale").textContent = t("js.waves.failed", { fn });
  };
  image.src = src;
  image.width = step.size[0];
  image.height = step.size[1];
  image.alt = t("js.waves.alt", { hull: said(hull.name), fn });
  $("waves-fn").value = index;
  $("waves-label").textContent = t("js.waves.label", { fn });
  $("waves-fn").setAttribute("aria-valuetext", t("js.waves.valuetext", { fn, regime: said(step.regime) }));
  $("waves-regime").textContent = said(step.regime);
  $("waves-scale").textContent = scale;
  $("cmd-waves").textContent = step.command;
  state.waveFn = step.fn;
}

function drawWaves(hull) {
  const waves = hull.waves;
  $("waves-shown").hidden = !waves.fits;
  $("waves-not").hidden = waves.fits;
  $("cmd-waves-wrap").hidden = !waves.fits;
  if (!waves.fits) $("waves-image").removeAttribute("src");
  if (waves.fits) {
    $("waves-fn").max = waves.steps.length - 1;
    // Keep the speed the reader chose when they switch hulls.
    const wanted = state.waveFn ?? 0.35;
    let index = 0;
    waves.steps.forEach((s, i) => {
      if (Math.abs(s.fn - wanted) < Math.abs(waves.steps[index].fn - wanted)) index = i;
    });
    setWave(index);
    return;
  }
  $("waves-reason").textContent = t("js.waves.not_fit", { hull: said(hull.name), reasons: said(waves.reason).join("; ") });
  const names = $("waves-hulls");
  names.textContent = "";
  const shown = state.manifest.hulls.filter((id) => state.manifest.waves_hulls.includes(id));
  shown.forEach((other, i) => {
    if (i) names.append(i === shown.length - 1 ? t("js.list.or") : t("js.list.comma"));
    const button = html("button", { type: "button", className: "link-button", textContent: t(`hull.${other}`) }, names);
    button.addEventListener("click", async () => {
      const radio = document.querySelector(`#picker input[value="${other}"]`);
      if (radio) radio.checked = true;
      await chooseHull(other, false);
      // The note and its buttons are gone now: carry focus to the control that replaced them.
      if (state.hull && state.hull.id === other && state.hull.waves.fits) $("waves-fn").focus({ preventScroll: true });
    });
  });
}

/* --- Charts ---------------------------------------------------------------------------------------- */

const chartRedraws = new Map();

function lineChart(host, spec) {
  host.textContent = "";
  if (spec.title) html("p", { className: "chart-title", textContent: spec.title }, host);
  if (spec.sub) html("p", { className: "chart-sub", textContent: spec.sub }, host);
  if (spec.series.length > 1) {
    const legend = html("ul", { className: "legend" }, host);
    spec.series.forEach((s, i) => {
      const item = html("li", {}, legend);
      const swatch = html("i", {}, item);
      swatch.style.color = SERIES[i];
      item.append(s.name);
    });
  }
  const plot = html("div", { className: "chart-plot" }, host);
  const tip = html("div", { className: "tooltip", hidden: true }, plot);
  const svg = el("svg", { role: "img", "aria-label": spec.alt }, plot);

  const xs = spec.series.flatMap((s) => s.points.map((p) => p[0]));
  const ys = spec.series.flatMap((s) => s.points.map((p) => p[1])).filter((v) => v !== null);
  const yText = (v) => (v === null ? t("js.chart.not_available") : spec.yFormat(v)); // A failed solve is a gap.
  const xLow = Math.min(...xs);
  const xHigh = Math.max(...xs);
  const yLow = Math.min(0, Math.min(...ys) * 1.15); // Room under the curves for the end labels.
  const yHigh = Math.max(...ys) * 1.06 || 1;

  const draw = () => {
    const width = plot.clientWidth;
    if (!width) return;
    const height = spec.height || Math.max(230, Math.min(340, width * 0.46));
    svg.textContent = "";
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.setAttribute("height", height);
    const m = { left: 58, right: spec.series.length > 1 ? 64 : 14, top: 10, bottom: 46 };
    const x = (v) => m.left + ((v - xLow) / (xHigh - xLow || 1)) * (width - m.left - m.right);
    const y = (v) => height - m.bottom - ((v - yLow) / (yHigh - yLow || 1)) * (height - m.top - m.bottom);

    for (const t of niceTicks(yLow, yHigh, 4)) {
      el("line", { x1: m.left, x2: width - m.right, y1: y(t), y2: y(t), class: t === 0 ? "zero" : "gridline" }, svg);
      const label = el("text", { x: m.left - 8, y: y(t), "text-anchor": "end", "dominant-baseline": "central" }, svg);
      label.textContent = significant(t, 3);
    }
    for (const t of niceTicks(xLow, xHigh, width < 420 ? 4 : 6)) {
      const label = el("text", { x: x(t), y: height - m.bottom + 18, "text-anchor": "middle" }, svg);
      label.textContent = significant(t, 3);
    }
    const xTitle = el("text", { x: m.left + (width - m.left - m.right) / 2, y: height - 6, "text-anchor": "middle", class: "axis-title" }, svg);
    xTitle.textContent = spec.xLabel;
    const yTitle = el("text", { x: 14, y: m.top + (height - m.top - m.bottom) / 2, "text-anchor": "middle", class: "axis-title", transform: `rotate(-90 14 ${m.top + (height - m.top - m.bottom) / 2})` }, svg);
    yTitle.textContent = spec.yLabel;

    spec.series.forEach((s, i) => {
      // The line breaks at a missing result rather than joining across it.
      let d = "";
      let joined = false;
      for (const p of s.points) {
        if (p[1] === null) {
          joined = false;
          continue;
        }
        d += `${joined ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`;
        joined = true;
      }
      el("path", { class: "curve", stroke: SERIES[i], d }, svg);
      if (s.points.length <= 12) {
        // Few results: mark each one, so the straight joins are not read as computed values.
        for (const p of s.points) {
          if (p[1] !== null) el("circle", { cx: x(p[0]), cy: y(p[1]), r: 4, fill: SERIES[i], class: "dot" }, svg);
        }
      }
    });

    if (spec.series.length > 1) {
      // Name each line where it ends, nudged apart so labels cannot overlap.
      const ends = spec.series
        .map((s) => ({ name: s.name, last: s.points.filter((p) => p[1] !== null).pop() }))
        .filter((end) => end.last)
        .map((end) => ({ name: end.name, y: y(end.last[1]) }))
        .sort((a, b) => a.y - b.y);
      for (let i = 1; i < ends.length; i++) ends[i].y = Math.max(ends[i].y, ends[i - 1].y + 16);
      for (const end of ends) {
        const label = el("text", { x: width - m.right + 6, y: end.y, "dominant-baseline": "central", class: "direct" }, svg);
        label.textContent = end.name;
      }
    }

    const cross = el("line", { y1: m.top, y2: height - m.bottom, class: "cross", visibility: "hidden" }, svg);
    const dots = spec.series.map((_, i) => el("circle", { r: 4.5, fill: SERIES[i], class: "dot", visibility: "hidden" }, svg));
    const hit = el("rect", { x: m.left, y: m.top, width: Math.max(0, width - m.left - m.right), height: height - m.top - m.bottom, fill: "transparent" }, svg);
    const hide = () => {
      cross.setAttribute("visibility", "hidden");
      dots.forEach((d) => d.setAttribute("visibility", "hidden"));
      tip.hidden = true;
    };
    const move = (event) => {
      const box = svg.getBoundingClientRect();
      const mouse = event.clientX - box.left;
      const value = xLow + ((mouse - m.left) / (width - m.left - m.right)) * (xHigh - xLow);
      const base = spec.series[0].points;
      let index = 0;
      base.forEach((p, i) => {
        if (Math.abs(p[0] - value) < Math.abs(base[index][0] - value)) index = i;
      });
      const px = x(base[index][0]);
      cross.setAttribute("x1", px);
      cross.setAttribute("x2", px);
      cross.setAttribute("visibility", "visible");
      tip.textContent = "";
      html("div", { textContent: `${spec.xName} ${spec.xFormat(base[index][0])}` }, tip);
      spec.series.forEach((s, i) => {
        const point = s.points[index];
        if (point[1] === null) {
          dots[i].setAttribute("visibility", "hidden");
        } else {
          dots[i].setAttribute("cx", px);
          dots[i].setAttribute("cy", y(point[1]));
          dots[i].setAttribute("visibility", "visible");
        }
        const line = html("div", {}, tip);
        const swatch = html("i", {}, line);
        swatch.style.background = SERIES[i];
        line.append(`${spec.series.length > 1 ? s.name + ": " : ""}${yText(point[1])}`);
      });
      tip.hidden = false;
      const left = px + 12 + tip.offsetWidth > width ? px - tip.offsetWidth - 12 : px + 12;
      tip.style.left = `${Math.max(0, left)}px`;
      tip.style.top = `${m.top}px`;
    };
    hit.addEventListener("pointermove", move);
    hit.addEventListener("pointerdown", move);
    hit.addEventListener("pointerleave", hide);
  };

  chartRedraws.set(host, draw);
  draw();

  const details = html("details", { className: "chart-table" }, host);
  html("summary", { textContent: t("js.chart.show_numbers") }, details);
  const scroll = html("div", { className: "chart-table-scroll" }, details);
  const table = html("table", {}, scroll);
  const head = html("tr", {}, html("thead", {}, table));
  html("th", { scope: "col", textContent: spec.xName }, head);
  for (const s of spec.series) html("th", { scope: "col", textContent: spec.series.length > 1 ? s.name : spec.yName }, head);
  const body = html("tbody", {}, table);
  spec.series[0].points.forEach((p, i) => {
    const row = html("tr", {}, body);
    html("td", { textContent: spec.xFormat(p[0]) }, row);
    for (const s of spec.series) html("td", { textContent: yText(s.points[i][1]) }, row);
  });
}

function solverWarnings(host, warnings) {
  if (!warnings || !warnings.length) return;
  const note = html("div", { className: "solver-note" }, host);
  html("p", { textContent: t("js.solver.note") }, note);
  const list = html("ul", {}, note);
  for (const warning of warnings) html("li", { textContent: warning }, list);
}

function drawCharts(hull) {
  chartRedraws.clear();
  const name = said(hull.name);
  // The labels of one chart, from its strings: `js.<chart>.title`, `.x_label` and the rest.
  const labels = (chart, values = {}) => ({
    title: t(`js.${chart}.title`),
    sub: t(`js.${chart}.sub`, values),
    alt: t(`js.${chart}.alt`, { hull: name }),
    xLabel: t(`js.${chart}.x_label`),
    yLabel: t(`js.${chart}.y_label`),
    xName: t(`js.${chart}.x_name`),
    yName: t(`js.${chart}.y_name`),
  });
  lineChart($("chart-sac"), {
    ...labels("sac", { draft: niceNumber(hull.sac.draft) }),
    xFormat: (v) => `${niceNumber(v)} m`,
    yFormat: (v) => `${significant(v)} m²`,
    series: [{ name: t("js.sac.y_name"), points: hull.sac.points }],
  });
  lineChart($("chart-gz"), {
    ...labels("gz", { kg: niceNumber(hull.stability.kg) }),
    xFormat: (v) => `${v}°`,
    yFormat: (v) => `${niceNumber(v, 3)} m`,
    series: hull.stability.series,
  });
  lineChart($("chart-resistance"), {
    ...labels("resistance", { method: t(`method.${hull.resistance.method}`) }),
    xFormat: (v) => `${significant(v, 3)} kn`,
    yFormat: (v) => `${significant(v)} kN`,
    series: [{ name: t("js.resistance.y_name"), points: hull.resistance.points }],
  });
  solverWarnings($("chart-resistance"), said(hull.resistance.warnings));
  const host = $("chart-motions");
  host.textContent = "";
  if (hull.motions) {
    const pair = html("div", { className: "chart-pair" }, host);
    const heave = html("div", {}, pair);
    const pitch = html("div", {}, pair);
    const motion = (which) => ({
      title: t(`js.motions.${which}_title`),
      sub: t(`js.motions.${which}_sub`),
      alt: t(`js.motions.${which}_alt`, { hull: name }),
      xLabel: t("js.motions.x_label"),
      xName: t("js.motions.x_name"),
      yLabel: t(`js.motions.${which}_y_label`),
      yName: t(`js.motions.${which}_y_name`),
      xFormat: (v) => niceNumber(v),
      yFormat: (v) => niceNumber(v, 3),
      height: 240,
    });
    lineChart(heave, { ...motion("heave"), series: [{ name: t("js.motions.heave_y_name"), points: hull.motions.heave }] });
    lineChart(pitch, { ...motion("pitch"), series: [{ name: t("js.motions.pitch_y_name"), points: hull.motions.pitch }] });
    solverWarnings(host, said(hull.motions.warnings));
  } else {
    html("p", { className: "chart-empty", textContent: t("js.motions.not_run") }, host);
  }
}

/* --- Build it: the skiff's sheets as seacraft wrote them ----------------------------------------- */

async function drawBuild() {
  // seacraft writes the sheets once per language: build/ in English, build/<lang>/ in the others.
  const folder = `${ROOT}build/${LANG_DIR}`;
  let index;
  try {
    const response = await fetch(`${folder}index.json`);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    index = await response.json();
  } catch (error) {
    $("build-caption").textContent = t("js.build.failed", { error: error.message });
    return;
  }
  const host = $("build-sheets");
  // Inlined rather than an <img>, so the lines, drawn 0.18 mm wide at full size, stay visible scaled down.
  let sheets;
  try {
    sheets = await Promise.all(index.sheets.map(async (name) => {
      const response = await fetch(`${folder}${name}`);
      if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`);
      const svg = new DOMParser().parseFromString(await response.text(), "image/svg+xml").documentElement;
      if (svg.nodeName !== "svg") throw new Error(`${name} is not an SVG`);
      return svg;
    }));
  } catch (error) {
    $("build-caption").textContent = t("js.build.failed", { error: error.message });
    return;
  }
  for (const [i, svg] of sheets.entries()) {
    svg.removeAttribute("width");
    svg.removeAttribute("height");
    svg.setAttribute("role", "img");
    const sheet = index.layout[i];
    svg.setAttribute("aria-label", t("js.build.sheet_label", {
      n: i + 1, length: niceNumber(sheet.length_m), width: niceNumber(sheet.width_m),
    }));
    host.appendChild(svg);
  }
  const size = (s) => `${niceNumber(s.length_m)} × ${niceNumber(s.width_m)} m`;
  const sizes = [...new Set(index.layout.map(size))];
  let blanks;
  if (sizes.length > 1) blanks = index.layout.map((s, i) => t("js.build.blank", { n: i + 1, size: size(s) })).join(t("js.list.comma"));
  else if (index.layout.length === 1) blanks = t("js.build.one_blank", { size: sizes[0] });
  else blanks = t("js.build.blanks", { count: index.layout.length, size: sizes[0] });
  $("build-caption").textContent = t("js.build.caption", { sheets: index.plywood_sheets, blanks });
  const files = $("build-files");
  const item = (href, title, detail) => {
    const link = html("a", { href }, html("li", {}, files));
    html("b", { textContent: title }, link);
    html("span", { textContent: detail }, link);
  };
  item(`${folder}panels.pdf`, t("js.build.pdf"), t("js.build.pdf_detail", { pages: index.pdf_pages, size: fileSize(index.sizes["panels.pdf"]) }));
  item(`${folder}panels.svg`, t("js.build.svg"), fileSize(index.sizes["panels.svg"]));
  index.sheets.forEach((name, i) => item(`${folder}${name}`, t("js.build.sheet_svg", { n: i + 1 }), fileSize(index.sizes[name])));
}

/* --- Applying a hull to the page ------------------------------------------------------------------- */

function applyHull(hull) {
  state.hull = hull;
  $("hull-description").textContent = `${said(hull.description)}.`;
  $("cmd-table").textContent = `seacraft table ${hull.id}.fef`;
  $("loop-hull").textContent = said(hull.name);
  $("files-hull").textContent = said(hull.name);
  $("cmd-lines").textContent = hull.commands.lines;
  $("cmd-hydro").textContent = `${hull.commands.hydro}\n${hull.commands.sac}`;
  $("cmd-stability").textContent = hull.stability.command;
  $("cmd-resistance").textContent = hull.resistance.command;
  $("cmd-report").textContent = hull.commands.report;
  $("cmd-motions-wrap").hidden = !hull.motions;
  if (hull.motions) $("cmd-motions").textContent = hull.motions.command;
  const base = `${ROOT}hulls/${hull.id}/`;
  $("file-report").href = `${base}${localizedFile("report.html")}`;
  $("file-viewer").href = `${base}viewer.html`;
  $("file-lines").href = `${base}${localizedFile("lines.svg")}`;
  $("file-fef").href = `${base}${hull.id}.fef`;
  $("size-report").textContent = t("js.files.html", { size: fileSize(said(hull.files.report)) });
  $("size-viewer").textContent = t("js.files.html", { size: fileSize(hull.files.viewer) });
  $("size-lines").textContent = fileSize(said(hull.files.lines));
  $("size-fef").textContent = fileSize(hull.files.fef);

  const input = $("draft");
  input.max = hull.table.length - 1;
  let design = 0;
  hull.table.forEach((row, i) => {
    if (Math.abs(row.draft - hull.draft) < Math.abs(hull.table[design].draft - hull.draft)) design = i;
  });
  buildSea(hull);
  drawLinesFigure(hull);
  drawCharts(hull);
  drawHeel(hull);
  drawWaves(hull);
  return design;
}

async function loadHull(id) {
  if (!cache.has(id)) {
    const response = await fetch(`${ROOT}hulls/${id}/data.json`);
    if (!response.ok) throw new Error(`hull ${id}: HTTP ${response.status}`);
    cache.set(id, await response.json());
  }
  return cache.get(id);
}

async function chooseHull(id, firstLoad) {
  const request = ++state.request;
  let hull;
  try {
    hull = await loadHull(id);
  } catch (error) {
    if (request !== state.request) return; // A later choice has already replaced this one.
    $("hull-description").textContent = t("js.load.hull_failed", { error: error.message });
    const shown = state.hull && document.querySelector(`#picker input[value="${state.hull.id}"]`);
    if (shown) shown.checked = true;
    return;
  }
  if (request !== state.request) return; // A later choice has already replaced this one.
  const design = applyHull(hull);
  state.shown = firstLoad ? 0 : Math.min(state.row, hull.table.length - 1);
  placeWater(state.shown);
  setRow(design, true);
}

async function init() {
  let manifest;
  try {
    const response = await fetch(`${ROOT}hulls/index.json`);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    manifest = await response.json();
  } catch (error) {
    $("hull-description").textContent = t("js.load.manifest_failed", { error: error.message });
    return;
  }
  state.manifest = manifest;
  const picker = $("picker");
  manifest.hulls.forEach((id, i) => {
    const label = html("label", {}, picker);
    const radio = html("input", { type: "radio", name: "hull", value: id, checked: i === 0 }, label);
    html("span", { textContent: t(`hull.${id}`) }, label);
    radio.addEventListener("change", () => chooseHull(id, false));
  });
  $("heel-weight").addEventListener("input", (event) => setHeel(+event.target.value));
  $("waves-fn").addEventListener("input", (event) => setWave(+event.target.value));
  $("draft").addEventListener("input", (event) => {
    sea.svg.classList.add("touched");
    setRow(+event.target.value, false);
  });

  let resizeTimer = 0;
  new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      if (state.hull) {
        buildSea(state.hull);
        placeWater(state.shown);
      }
      for (const redraw of chartRedraws.values()) redraw();
      if (heel.parts) setHeel(+$("heel-weight").value);
    }, 80);
  }).observe(document.body);
  document.fonts.ready.then(sizeSea);

  chooseHull(manifest.hulls[0], true);
  drawBuild();
}

init();
