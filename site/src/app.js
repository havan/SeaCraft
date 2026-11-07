/* The hull on the page is data: site/build.py runs seacraft on each example and writes one JSON file per
   hull. This script draws that data. It computes no hydrostatics of its own. */
"use strict";

const NS = "http://www.w3.org/2000/svg";
const $ = (id) => document.getElementById(id);
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
const SERIES = ["var(--series-1)", "var(--series-2)", "var(--series-3)"];
const cache = new Map();

const state = { hull: null, row: 0, shown: 0, raf: 0, request: 0 };

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

const niceNumber = (value, digits = 2) =>
  value.toLocaleString("en-GB", { minimumFractionDigits: digits, maximumFractionDigits: digits });

function significant(value, sig = 4) {
  if (value === 0) return "0";
  const digits = Math.max(0, sig - 1 - Math.floor(Math.log10(Math.abs(value))));
  return value.toLocaleString("en-GB", { maximumFractionDigits: digits, minimumFractionDigits: 0 });
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
      fill: "currentColor", stroke: "none", "font-family": "Big Shoulders Stencil, sans-serif", "font-weight": 700,
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
  el("text", { "text-anchor": "end", "dominant-baseline": "central" }, hint).textContent = "Drag the waterline";
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
  state.row = index;
  const row = state.hull.table[index];
  $("draft").value = index;
  $("draft").setAttribute("aria-valuetext", `${niceNumber(row.draft)} metres`);
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
  const yText = (v) => (v === null ? "not available" : spec.yFormat(v)); // A failed solve is a gap.
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
  html("summary", { textContent: "Show the numbers" }, details);
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
  html("p", { textContent: "What the solver reported about this result:" }, note);
  const list = html("ul", {}, note);
  for (const warning of warnings) html("li", { textContent: warning }, list);
}

function drawCharts(hull) {
  chartRedraws.clear();
  lineChart($("chart-sac"), {
    title: "Sectional area along the hull",
    sub: `At the design draft of ${niceNumber(hull.sac.draft)} m. Stern is on the left.`,
    alt: `Sectional area curve of the ${hull.name}.`,
    xLabel: "Position along the hull (m)",
    yLabel: "Area (m²)",
    xName: "Position",
    yName: "Area",
    xFormat: (v) => `${niceNumber(v)} m`,
    yFormat: (v) => `${significant(v)} m²`,
    series: [{ name: "Area", points: hull.sac.points }],
  });
  lineChart($("chart-gz"), {
    title: "Righting lever against heel",
    sub: `With the centre of gravity ${niceNumber(hull.stability.kg)} m above the keel, at three displacements.`,
    alt: `Righting lever curves of the ${hull.name}.`,
    xLabel: "Heel (degrees)",
    yLabel: "GZ (m)",
    xName: "Heel",
    yName: "GZ",
    xFormat: (v) => `${v}°`,
    yFormat: (v) => `${niceNumber(v, 3)} m`,
    series: hull.stability.series,
  });
  lineChart($("chart-resistance"), {
    title: "Total resistance against speed",
    sub: `Estimated with ${hull.resistance.method}, using this hull’s own measurements.`,
    alt: `Resistance curve of the ${hull.name}.`,
    xLabel: "Speed (knots)",
    yLabel: "Resistance (kN)",
    xName: "Speed",
    yName: "Resistance",
    xFormat: (v) => `${significant(v, 3)} kn`,
    yFormat: (v) => `${significant(v)} kN`,
    series: [{ name: "Resistance", points: hull.resistance.points }],
  });
  solverWarnings($("chart-resistance"), hull.resistance.warnings);
  const host = $("chart-motions");
  host.textContent = "";
  if (hull.motions) {
    const pair = html("div", { className: "chart-pair" }, host);
    const heave = html("div", {}, pair);
    const pitch = html("div", {}, pair);
    const shared = {
      xLabel: "Wave length / ship length",
      xName: "λ/L",
      xFormat: (v) => niceNumber(v),
      height: 240,
    };
    lineChart(heave, {
      ...shared,
      title: "Heave response",
      sub: "Heave amplitude over wave amplitude, z/ζ, in head waves.",
      alt: `Heave response of the ${hull.name}.`,
      yLabel: "Heave (z/ζ)",
      yName: "Heave",
      yFormat: (v) => niceNumber(v, 3),
      series: [{ name: "Heave", points: hull.motions.heave }],
    });
    lineChart(pitch, {
      ...shared,
      title: "Pitch response",
      sub: "Pitch amplitude over wave slope, θ/kζ, in head waves.",
      alt: `Pitch response of the ${hull.name}.`,
      yLabel: "Pitch (θ/kζ)",
      yName: "Pitch",
      yFormat: (v) => niceNumber(v, 3),
      series: [{ name: "Pitch", points: hull.motions.pitch }],
    });
    solverWarnings(host, hull.motions.warnings);
  } else {
    html("p", {
      className: "chart-empty",
      textContent: "Not run for this hull. Strip theory does not describe a boat on the plane, so choose a displacement hull to see this step.",
    }, host);
  }
}

/* --- Applying a hull to the page ------------------------------------------------------------------- */

function applyHull(hull) {
  state.hull = hull;
  $("hull-description").textContent = `${hull.description}.`;
  $("cmd-table").textContent = `seacraft table ${hull.id}.fef`;
  $("loop-hull").textContent = hull.name;
  $("files-hull").textContent = hull.name;
  $("cmd-lines").textContent = hull.commands.lines;
  $("cmd-hydro").textContent = `${hull.commands.hydro}\n${hull.commands.sac}`;
  $("cmd-stability").textContent = hull.stability.command;
  $("cmd-resistance").textContent = hull.resistance.command;
  $("cmd-report").textContent = hull.commands.report;
  $("cmd-motions-wrap").hidden = !hull.motions;
  if (hull.motions) $("cmd-motions").textContent = hull.motions.command;
  const base = `hulls/${hull.id}/`;
  $("file-report").href = `${base}report.html`;
  $("file-viewer").href = `${base}viewer.html`;
  $("file-lines").href = `${base}lines.svg`;
  $("file-fef").href = `${base}${hull.id}.fef`;
  $("size-report").textContent = `HTML, ${fileSize(hull.files.report)}`;
  $("size-viewer").textContent = `HTML, ${fileSize(hull.files.viewer)}`;
  $("size-lines").textContent = fileSize(hull.files.lines);
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
  return design;
}

async function loadHull(id) {
  if (!cache.has(id)) {
    const response = await fetch(`hulls/${id}/data.json`);
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
    $("hull-description").textContent = `This hull’s data did not load (${error.message}). Reload the page to try again.`;
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
    const response = await fetch("hulls/index.json");
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    manifest = await response.json();
  } catch (error) {
    $("hull-description").textContent = `The hull data did not load (${error.message}). Reload the page to try again.`;
    return;
  }
  const picker = $("picker");
  manifest.hulls.forEach((hull, i) => {
    const label = html("label", {}, picker);
    const radio = html("input", { type: "radio", name: "hull", value: hull.id, checked: i === 0 }, label);
    html("span", { textContent: hull.label }, label);
    radio.addEventListener("change", () => chooseHull(hull.id, false));
  });
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
    }, 80);
  }).observe(document.body);
  document.fonts.ready.then(sizeSea);

  chooseHull(manifest.hulls[0].id, true);
}

init();
