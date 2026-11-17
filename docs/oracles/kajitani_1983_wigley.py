"""Readings of Kajitani et al. (1983) for the Wigley Michell oracle (docs/oracles.md §39).

Kajitani, H., Miyata, H., Ikehata, M., Tanaka, H., Adachi, H., Namimatsu, M. & Ogiwara, S. (1983),
"The summary of the cooperative experiment on Wigley parabolic model in Japan", Proc. 2nd DTNSRDC
Workshop on Ship Wave-Resistance Computations, pp. 5-35, DTIC ADP003037
(https://apps.dtic.mil/sti/pdfs/ADP003037.pdf). The PDF is a scan and is not in this repository.

The wave resistance coefficient Cw = C_T - (1 + k) C_F0 is only plotted (Figs. 1, 2, 5), so it was
read off the figures: each page rendered at 300 dpi, the axes calibrated on the printed scale ticks,
and a labelled 0.005 Fn x 0.05e-3 grid drawn over enlarged crops, against which the markers and the
faired curves were read by eye. This script redraws those crops so the readings can be checked:

    pdftoppm -r 300 -f 6 -l 10 -png ADP003037.pdf page
    python3 -I kajitani_1983_wigley.py page-06.png page-07.png page-10.png OUTDIR

It needs Pillow and NumPy, and prints the readings with Michell's ratios to them. Pixel positions are
for the DTIC scan at 300 dpi (2593 x 3385, 2560 x 3355 and 2593 x 3389 pixels); another scan needs
its own calibration. Written 2026-11-18.

Calibration. The figures are drawn on graph paper of 0.01 Fn x 0.2e-3 cells. Fn: the dashed
verticals under the printed 0.1 ... 0.4 labels. Coefficient: the typed scale's tick dashes, which sit
on the half-rows of the paper (the dash beside "0" in Fig. 2 is at y = 2324, the rows at 2300 and
2350); the panel frames sit 0.1e-3 below the zero tick, so a frame is not a zero line. In Figs. 1
and 2 the Cw panel uses the outer (C_T) scale; the Cwp panel has its own inner scale. Fig. 5 is
printed sideways and is turned upright first; its axes are a plain frame with ticks.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw

# (Fn calibration, coefficient calibration): value -> pixel, piecewise linear.
FIG1 = ({0.1: 658, 0.15: 911.5, 0.2: 1162.5, 0.25: 1411, 0.3: 1659.5, 0.35: 1909, 0.4: 2157.5},
        {0: 2350, 1: 2091.25, 2: 1832.5, 3: 1573.75, 4: 1315})
FIG1_CWP = {0: 1983, 1: 1724.25, 2: 1465.5}
FIG2 = ({0.1: 623, 0.15: 872.5, 0.2: 1125, 0.25: 1375, 0.3: 1623.5, 0.35: 1873, 0.4: 2123.5},
        {0: 2325, 1: 2066.5, 2: 1808, 3: 1549.5})
FIG2_CWP = {0: 2066.5, 1: 1808, 2: 1549.5}
FIG5 = ({0: 546, 0.1: 1148.5, 0.2: 1745.5, 0.3: 2339, 0.4: 2935},
        {0: 1995.5, 1: 1768.5, 2: 1539.5, 3: 1313.5, 4: 1085.5, 5: 860, 6: 635.5, 7: 414})

# Cw x 1e3 read from the faired curves and markers: (model, condition, figure, marker,
# hump Fn, hump Cw, hollow Fn, hollow Cw). Uncertainty: Fn +- 0.005, Cw +- 0.05e-3.
CW = [
    ("SRI 4.0 m", "fixed", "Fig. 2", "circle", 0.320, 1.39, 0.350, 1.21),
    ("UT 2.5 m", "fixed", "Fig. 2", "triangle", 0.318, 1.45, 0.350, 1.30),
    ("IHI 6.0 m", "free", "Fig. 1", "inverted triangle", 0.313, 1.81, 0.345, 1.62),
    ("SRI 4.0 m", "free", "Fig. 1", "circle", 0.320, 1.61, 0.350, 1.45),
    ("UT 2.5 m", "free", "Fig. 1", "triangle", 0.320, 1.70, 0.345, 1.53),
    ("YNU 2.0 m", "free", "Fig. 5", "circle", 0.322, 1.59, 0.347, 1.48),
]
# Cwp x 1e3 (wave-pattern analysis) at the hump, the quantity Michell's far-field integral
# corresponds to. More scatter than Cw; the free-condition humps are read from markers.
CWP = [
    ("SRI 4.0 m", "fixed", "Fig. 2", 0.318, 1.37),
    ("UT 2.5 m", "fixed", "Fig. 2", 0.318, 1.48),
    ("IHI 6.0 m", "free", "Fig. 1", 0.313, 1.57),
    ("SRI 4.0 m", "free", "Fig. 1", 0.315, 1.45),
    ("YNU 2.0 m", "free", "Fig. 5", 0.322, 1.32),
]
# Michell on the same hull (SeaCraft, 129 x 49 grid, 1024 angles; Cw on the exact S = 14.879 m^2
# for L = 10 m): hump and hollow.
MICHELL = {"hump": (0.299, 2.1425), "hollow": (0.346, 1.2354)}


def interp(cal, v):
    keys = sorted(cal)
    return float(np.interp(v, keys, [cal[k] for k in keys]))


def crop(img, out, fncal, ccal, fn0, fn1, c0, c1, k=2.0):
    """Crop [fn0, fn1] x [c0, c1], enlarge k times and draw the reading grid."""
    x0, x1 = sorted((interp(fncal, fn0), interp(fncal, fn1)))
    y0, y1 = sorted((interp(ccal, c0), interp(ccal, c1)))
    im = img.crop((int(x0), int(y0), int(x1), int(y1)))
    im = im.resize((int(im.width * k), int(im.height * k)), Image.LANCZOS)
    d = ImageDraw.Draw(im)
    for i in range(int(round((fn1 - fn0) / 0.005)) + 1):
        v = fn0 + 0.005 * i
        x = (interp(fncal, v) - int(x0)) * k
        major = round(v * 1000) % 10 == 0
        d.line([(x, 0), (x, im.height)], fill=(255, 0, 0) if major else (255, 190, 190))
        if major:
            d.text((x + 3, im.height - 14), f"{v:.2f}", fill=(220, 0, 0))
    for j in range(int(round((c1 - c0) / 0.05)) + 1):
        v = c0 + 0.05 * j
        y = (interp(ccal, v) - int(y0)) * k
        major = round(v * 100) % 10 == 0
        d.line([(0, y), (im.width, y)], fill=(0, 0, 255) if major else (190, 190, 255))
        if major:
            d.text((3, y - 12), f"{v:.1f}", fill=(0, 0, 220))
    im.save(out)


def main():
    if len(sys.argv) == 5:
        p6, p7, p10, out = sys.argv[1:5]
        os.makedirs(out, exist_ok=True)
        f1 = Image.open(p6).convert("RGB")
        f2 = Image.open(p7).convert("RGB")
        f5 = Image.open(p10).convert("RGB").rotate(-90, expand=True)
        for name, img, cal, ccal, box in [
            ("fig1_cw_hump", f1, FIG1[0], FIG1[1], (0.295, 0.36, 1.2, 2.0)),
            ("fig1_cw_hollow", f1, FIG1[0], FIG1[1], (0.335, 0.385, 1.0, 1.8)),
            ("fig1_cwp", f1, FIG1[0], FIG1_CWP, (0.29, 0.39, 0.8, 1.8)),
            ("fig2_cw_hump", f2, FIG2[0], FIG2[1], (0.295, 0.345, 1.0, 1.6)),
            ("fig2_cw_hollow", f2, FIG2[0], FIG2[1], (0.335, 0.385, 1.0, 1.6)),
            ("fig2_cwp", f2, FIG2[0], FIG2_CWP, (0.29, 0.39, 0.8, 1.8)),
            ("fig5_cw", f5, FIG5[0], FIG5[1], (0.29, 0.40, 0.8, 2.4)),
        ]:
            crop(img, os.path.join(out, name + ".png"), cal, ccal, *box)
        print(f"crops written to {out}")
    elif len(sys.argv) != 1:
        sys.exit(__doc__)
    (hf, hc), (lf, lc) = MICHELL["hump"], MICHELL["hollow"]
    print(f"Michell: hump {hc:.3f}e-3 at Fn {hf:.3f}, hollow {lc:.3f}e-3 at Fn {lf:.3f}")
    print("Cw: model, condition, hump Fn early by, hump ratio, hollow Fn diff, hollow ratio")
    for model, cond, fig, _, h_fn, h_cw, l_fn, l_cw in CW:
        print(f"  {model:10} {cond:6} {fig}: {h_fn - hf:+.3f} {hc / h_cw:.3f}  "
              f"{l_fn - lf:+.3f} {lc / l_cw:.3f}")
    print("Cwp hump: model, condition, ratio")
    for model, cond, fig, _, c in CWP:
        print(f"  {model:10} {cond:6} {fig}: {hc / c:.3f}")


if __name__ == "__main__":
    main()
