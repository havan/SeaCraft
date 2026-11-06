"""Oracle for crates/seacraft-core/src/seakeeping.rs (B7a): 2D heave added mass
and damping of Lewis sections in deep water, by two independent methods:
  * Frank close-fit source panels (any section), and
  * Ursell (1949) wave source + wave-free multipoles (semicircle only).
Run: python3 heave_2d.py   (numpy + scipy). Written 2026-10-23.
The wave-source kernel F = e^{KW}E1(KW) + i*pi*sgn(x)*e^{KW} was checked
against direct numerical principal-value integration (value and gradient).

Fluid y < 0, y up. FSBC phi_y = K phi at y = 0, K = w^2/g.
G = ln r1 - ln r2 - 2 PV int_0^inf e^{k(y+b)} cos k(x-a) / (k - K) dk + i C e^{K(y+b)} cos K(x-a)
"""
import sys
import numpy as np
from scipy import optimize, integrate, special

RHO, G_ACC = 1025.0, 9.81


def lewis_xy(a1, a3, ms, th):
    y = ms * ((1 + a1) * np.sin(th) - a3 * np.sin(3 * th))
    z = -ms * ((1 - a1) * np.cos(th) + a3 * np.cos(3 * th))
    return y, z


def poly_area_half(a1, a3, ms, n=4000):
    th = np.linspace(0, np.pi / 2, n)
    y, z = lewis_xy(a1, a3, ms, th)
    # closed by the waterline (z=0) and centreline (y=0)
    yy = np.concatenate([[0.0], y, [0.0]])
    zz = np.concatenate([[0.0], z, [0.0]])
    return 0.5 * abs(np.sum(yy[:-1] * zz[1:] - yy[1:] * zz[:-1]))


def solve_lewis(b, t, sigma):
    def res(p):
        a1, a3 = p
        h0 = (1 + a1 + a3) / (1 - a1 + a3)
        ms = b / (1 + a1 + a3)
        s = poly_area_half(a1, a3, ms) / (b * t)
        return [h0 - b / t, s - sigma]
    a1, a3 = optimize.fsolve(res, [0.1, 0.0], xtol=1e-13)
    return a1, a3, b / (1 + a1 + a3)


def panels(a1, a3, ms, n):
    th = np.linspace(0, np.pi / 2, n + 1)
    # cosine spacing toward the waterline helps the free-surface corner
    y, z = lewis_xy(a1, a3, ms, th)
    p0 = np.stack([y[:-1], z[:-1]], 1)
    p1 = np.stack([y[1:], z[1:]], 1)
    return p0, p1


GX, GW = np.polynomial.legendre.leggauss(8)


def solve(a1, a3, ms, K, n=60, csign=-1.0):
    p0, p1 = panels(a1, a3, ms, n)
    mid = 0.5 * (p0 + p1)
    d = p1 - p0
    L = np.hypot(d[:, 0], d[:, 1])
    tvec = d / L[:, None]
    # outward normal (into fluid): contour runs keel -> waterline on the +y side,
    # body is to the left (toward y=0), so the fluid normal is the right-hand normal
    nrm = np.stack([tvec[:, 1], -tvec[:, 0]], 1)
    assert np.all(nrm[:, 0] * mid[:, 0] + nrm[:, 1] * mid[:, 1] > -1e-9)
    N = len(L)
    A = np.zeros((N, N), complex)   # normal velocity at i due to unit sigma on j (and its mirror)
    P = np.zeros((N, N), complex)   # potential at i
    for j in range(N):
        for mirror in (1.0, -1.0):
            qa = (p0[j] + np.outer((GX + 1) / 2, d[j]))
            qa[:, 0] *= mirror
            w = GW / 2 * L[j]
            for i in range(N):
                x, y = mid[i]
                a, b = qa[:, 0], qa[:, 1]
                dx, dy = x - a, y - b
                r1 = dx ** 2 + dy ** 2
                dyi = y + b
                r2 = dx ** 2 + dyi ** 2
                if K == np.inf:
                    reg = 0.0; gxr = 0.0; gyr = 0.0
                else:
                    W = dyi + 1j * dx
                    Fv = np.exp(K * W) * special.exp1(K * W) + 1j * np.pi * np.sign(dx) * np.exp(K * W)
                    dF = -1.0 / W + K * Fv
                    reg = -2 * Fv.real
                    gyr = -2 * dF.real
                    gxr = -2 * (1j * dF).real
                    ex = np.exp(K * dyi)
                    reg = reg + 1j * csign * 2 * np.pi * ex * np.cos(K * dx)
                    gyr = gyr + 1j * csign * 2 * np.pi * K * ex * np.cos(K * dx)
                    gxr = gxr - 1j * csign * 2 * np.pi * K * ex * np.sin(K * dx)
                self_panel = (i == j and mirror > 0)
                if self_panel:
                    gx1 = gy1 = 0.0
                    phi1 = L[j] * (np.log(L[j] / 2) - 1)
                else:
                    gx1 = dx / r1; gy1 = dy / r1
                    phi1 = None
                gx = gx1 - dx / r2 + gxr
                gy = gy1 - dyi / r2 + gyr
                vn = np.sum(w * (gx * nrm[i, 0] + gy * nrm[i, 1]))
                phi_other = np.sum(w * (-0.5 * np.log(r2) + reg))
                if self_panel:
                    vn += np.pi
                    ph = phi1 + phi_other
                else:
                    ph = np.sum(w * 0.5 * np.log(r1)) + phi_other
                A[i, j] += vn
                P[i, j] += ph
    rhs = nrm[:, 1].astype(complex)   # unit heave velocity
    sig = np.linalg.solve(A, rhs)
    phi = P @ sig
    I = 2 * np.sum(phi * nrm[:, 1] * L)   # both sides
    return I


def coeffs(a1, a3, ms, wb, b, n=60):
    w = np.sqrt(wb * G_ACC / b)
    K = w * w / G_ACC
    I = solve(a1, a3, ms, K, n)
    am = -RHO * I.real
    damp = -w * RHO * I.imag
    abar = np.sqrt(abs(damp) * w ** 3 / (RHO * G_ACC ** 2))
    return am, damp, abar, w


def wave_source(x, y, K):
    # source on the free surface at the origin: limit b -> 0 of frank.py's G
    W = y + 1j * x
    F = np.exp(K * W) * special.exp1(K * W) + 1j * np.pi * np.sign(x) * np.exp(K * W)
    return -2 * F.real - 1j * 2 * np.pi * np.exp(K * y) * np.cos(K * x)


def wave_free(x, y, K, m):
    r = np.hypot(x, y)
    th = np.arctan2(x, -y)  # from the downward vertical
    return np.cos(2 * m * th) / r ** (2 * m) + K / (2 * m - 1) * np.cos((2 * m - 1) * th) / r ** (2 * m - 1)


def fsbc_residual(K):
    x = np.linspace(1.3, 4.0, 5); h = 1e-6
    for f in (lambda x, y: wave_source(x, y, K), lambda x, y: wave_free(x, y, K, 2)):
        dy = (f(x, -h) - f(x, -3 * h)) / (2 * h)
        print("FSBC residual", np.max(np.abs(dy - K * f(x, -2 * h))))


def ursell_solve(R, K, M=12, npts=200):
    th = np.linspace(1e-4, np.pi / 2 - 1e-4, npts)
    x, y = R * np.sin(th), -R * np.cos(th)
    h = 1e-6
    def dr(f):
        return (f(x * (1 + h / R), y * (1 + h / R)) - f(x * (1 - h / R), y * (1 - h / R))) / (2 * h)
    cols = [dr(lambda a, b: wave_source(a, b, K))]
    vals = [wave_source(x, y, K)]
    for m in range(1, M + 1):
        cols.append(dr(lambda a, b, m=m: wave_free(a, b, K, m)).astype(complex))
        vals.append(wave_free(x, y, K, m).astype(complex))
    A = np.stack(cols, 1); V = np.stack(vals, 1)
    rhs = (y / R).astype(complex)       # n_y with n into the fluid (radial)
    c, *_ = np.linalg.lstsq(A, rhs, rcond=None)
    phi = V @ c
    ny = y / R
    ds = R * np.gradient(th)
    I = 2 * np.sum(phi * ny * ds)
    return I, np.max(np.abs(A @ c - rhs))



if __name__ == "__main__":
    for (b, t, sig) in [(1.0, 1.0, np.pi / 4), (1.5, 1.0, 0.9)]:
        a1, a3, ms = solve_lewis(b, t, sig)
        Iinf = solve(a1, a3, ms, np.inf, 60)
        norm = RHO * np.pi * b * b / 2
        print(f"\nFrank panels, b={b} T={t} sigma={sig:.4f}: a1={a1:.6f} a3={a3:.6f} Ms={ms:.6f}")
        print(f"  infinite-frequency added mass / (rho pi b^2/2) = {-RHO*Iinf.real/norm:.4f}")
        print("  w^2b/g   a/(rho pi b^2/2)   b/(rho pi b^2/2 w)   Abar")
        for wb in [0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0]:
            am, dp, ab, w = coeffs(a1, a3, ms, wb, b)
            print(f"  {wb:5.2f}   {am/norm:10.4f}   {dp/(norm*w):10.4f}   {ab:8.4f}")
    print("\nUrsell multipoles, semicircle R=1")
    nm = RHO * np.pi / 2
    for xi in [0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0]:
        K = xi; w = np.sqrt(K * G_ACC)
        I, res = ursell_solve(1.0, K)
        am = -RHO * I.real; dp = -w * RHO * I.imag
        ab = np.sqrt(abs(dp) * w ** 3 / (RHO * G_ACC ** 2))
        print(f"  {xi:5.2f}   {am/nm:10.4f}   {dp/(nm*w):10.4f}   {ab:8.4f}")
