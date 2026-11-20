"""Cross curves (KN) and GZ of the Wigley hull, integrated independently of SeaCraft (docs/oracles.md §45).

The hull is the one Kajitani et al. (1983), p. 8, publish:

    y = ±(B/2)(1 − ξ²)(1 − ζ²),  ξ = 2x/L − 1,  ζ = (T − z)/T,

taken wall-sided above T (ζ = 0) to a flat deck at D, as `examples/wigley.fef` models it, with
L 10 m, B 1 m, T 0.625 m, D 1 m. No KN or GZ for this hull is published; this script is the
reference, and it shares no code with SeaCraft.

Kajitani, H., Miyata, H., Ikehata, M., Tanaka, H., Adachi, H., Namimatsu, M. & Ogiwara, S. (1983),
"The summary of the cooperative experiment on Wigley parabolic model in Japan", Proc. 2nd DTNSRDC
Workshop on Ship Wave-Resistance Computations, pp. 5-35, DTIC ADP003037.

Method. Body axes: x aft to fore, y to starboard, z up from the keel. Heeled φ to starboard, a point
is under water when z cos φ − y sin φ ≤ c. Each section is a closed polygon (starboard side from
the keel to the deck edge, the deck, the port side back down), clipped by that half-plane
(Sutherland–Hodgman; the sections are convex) and integrated by the shoelace formula for area and
centroid. Sections are summed by Simpson's rule over x. The plane offset c is solved for the
volume at T, ∇ = 4/9·LBT, by safeguarded secant iteration. Then

    KN = y_B cos φ + z_B sin φ,    GZ = KN − KG sin φ,

with K on the keel at the centreline, which is how SeaCraft's `stability.rs` reports KN. Fore-aft
symmetry keeps the hull level in trim at every heel, so no trim is solved for.

Self-checks (asserted): upright ∇ and KB against the closed forms to 1e-6; KN(0) = 0; KN(5°)
within 1 % of KM sin 5° with KM = 5T/8 + 9B²/(105T); halving both resolutions moves no KN by
more than 1e-5 m.

    python3 -I wigley_gz.py

Standard library only; takes a few minutes. Written 2026-11-20.
"""
import math

L, B, T, D = 10.0, 1.0, 0.625, 1.0
KG = 0.35
VOLUME = 4.0 / 9.0 * L * B * T
HEELS = [0, 5, 10, 20, 30, 40, 50, 60, 70, 80]


def section(x, m):
    """The closed section at x: starboard keel to deck edge, the deck, port back to the keel."""
    xi = 2.0 * x / L - 1.0
    s = 0.5 * B * (1.0 - xi * xi)
    # Rows uniform in z with one exactly at T, where the side turns wall-sided.
    m_low = round(m * T / D)
    zs = [T * k / m_low for k in range(m_low + 1)]
    zs += [T + (D - T) * k / (m - m_low) for k in range(1, m - m_low + 1)]
    stbd = []
    for z in zs:
        zeta = max(0.0, (T - z) / T)
        stbd.append((s * (1.0 - zeta * zeta), z))
    port = [(-y, z) for (y, z) in reversed(stbd)]
    # Anticlockwise seen from ahead (y right, z up): keel, up the starboard side, the deck to
    # port, down the port side. The keel point closes the loop.
    return stbd + port[:-1]


def clip(poly, sin_p, cos_p, c):
    """Sutherland–Hodgman clip of `poly` to z cos φ − y sin φ ≤ c."""
    out = []
    n = len(poly)
    for i in range(n):
        p, q = poly[i], poly[(i + 1) % n]
        dp = p[1] * cos_p - p[0] * sin_p - c
        dq = q[1] * cos_p - q[0] * sin_p - c
        if dp <= 0.0:
            out.append(p)
        if (dp < 0.0) != (dq < 0.0) and dp != dq:
            t = dp / (dp - dq)
            out.append((p[0] + t * (q[0] - p[0]), p[1] + t * (q[1] - p[1])))
    return out


def area_centroid(poly):
    a = sy = sz = 0.0
    n = len(poly)
    for i in range(n):
        y0, z0 = poly[i]
        y1, z1 = poly[(i + 1) % n]
        cr = y0 * z1 - y1 * z0
        a += cr
        sy += (y0 + y1) * cr
        sz += (z0 + z1) * cr
    if abs(a) < 1e-300:
        return 0.0, 0.0, 0.0
    return 0.5 * a, sy / (3.0 * a), sz / (3.0 * a)


def immersed(heel_deg, c, m, n):
    """Volume and centre of buoyancy (y_B, z_B) below the plane at offset c."""
    sin_p, cos_p = math.sin(math.radians(heel_deg)), math.cos(math.radians(heel_deg))
    vol = my = mz = 0.0
    # Sections are symmetric about midship: integrate the aft half and double it.
    for i in range(n // 2 + 1):
        w = 1.0 if i in (0, n) else (4.0 if i % 2 else 2.0)
        if i == n // 2:
            w /= 2.0  # midship counted once after doubling
        x = L * i / n
        a, yc, zc = area_centroid(clip(section(x, m), sin_p, cos_p, c))
        vol += w * a
        my += w * a * yc
        mz += w * a * zc
    h = L / n
    vol, my, mz = 2.0 * vol * h / 3.0, 2.0 * my * h / 3.0, 2.0 * mz * h / 3.0
    return vol, my / vol, mz / vol


def solve(heel_deg, m, n):
    """The offset c that floats the hull at the design volume, and its (y_B, z_B)."""
    sin_p, cos_p = math.sin(math.radians(heel_deg)), math.cos(math.radians(heel_deg))
    lo = -0.5 * B * sin_p - 1e-9
    hi = D * cos_p + 0.5 * B * sin_p + 1e-9
    f_lo, f_hi = -VOLUME, immersed(heel_deg, hi, m, n)[0] - VOLUME
    c = lo + (hi - lo) * 0.5
    side = 0
    for _ in range(200):
        # Illinois (safeguarded regula falsi): converges superlinearly, never leaves [lo, hi].
        c = (lo * f_hi - hi * f_lo) / (f_hi - f_lo)
        v, yb, zb = immersed(heel_deg, c, m, n)
        f = v - VOLUME
        if abs(f) < 1e-10 * VOLUME:
            return c, yb, zb
        if f < 0.0:
            lo, f_lo = c, f
            if side == -1:
                f_hi /= 2.0
            side = -1
        else:
            hi, f_hi = c, f
            if side == 1:
                f_lo /= 2.0
            side = 1
    raise RuntimeError(f"no waterline at {heel_deg} deg")


def kn_table(m, n):
    rows = []
    for phi in HEELS:
        c, yb, zb = solve(phi, m, n)
        s, co = math.sin(math.radians(phi)), math.cos(math.radians(phi))
        rows.append((phi, c, yb * co + zb * s))
    return rows


def main():
    m, n = 2000, 400

    v0, yb0, zb0 = immersed(0, T, m, n)
    assert abs(v0 - VOLUME) < 1e-6 * VOLUME, (v0, VOLUME)
    assert abs(zb0 - 0.625 * T) < 1e-6, zb0
    print(f"upright at T: volume {v0:.7f} (closed {VOLUME:.7f}), KB {zb0:.7f} (closed {0.625 * T:.7f})")

    fine = kn_table(m, n)
    coarse = kn_table(m // 2, n // 2)
    worst = max(abs(a[2] - b[2]) for a, b in zip(fine, coarse))
    print(f"halving resolution moves KN by at most {worst:.2e} m")
    assert worst < 1e-5, worst

    assert abs(fine[0][2]) < 1e-9, fine[0]
    km = 5.0 * T / 8.0 + 9.0 * B * B / (105.0 * T)
    small = km * math.sin(math.radians(5.0))
    assert abs(fine[1][2] / small - 1.0) < 0.01, (fine[1][2], small)
    print(f"KN(5) {fine[1][2]:.5f} vs KM sin 5 {small:.5f}")

    print()
    print(" heel    KN (m)     GZ (m) at KG 0.35   midship deck edge")
    for phi, c, kn in fine:
        s, co = math.sin(math.radians(phi)), math.cos(math.radians(phi))
        edge = "under" if D * co - 0.5 * B * s < c else "dry"
        print(f"{phi:5d} {kn:10.5f} {kn - KG * s:10.5f}            {edge}")
    print()
    print(f"midship deck edge immersion: {deck_edge_angle(m, n):.2f} deg")


def deck_edge_angle(m, n):
    """The heel at which the waterline reaches the midship deck edge (y = B/2, z = D), by bisection."""

    def wet(phi):
        c = solve(phi, m, n)[0]
        return D * math.cos(math.radians(phi)) - 0.5 * B * math.sin(math.radians(phi)) - c

    lo, hi = 0.0, 90.0
    for _ in range(30):
        mid = 0.5 * (lo + hi)
        if wet(mid) > 0.0:
            lo = mid
        else:
            hi = mid
    return 0.5 * (lo + hi)


if __name__ == "__main__":
    main()
