"""Oracle for E3: Frank close-fit panels with an interior free-surface lid.

Extra source panels on y = 0 between the waterline and the centreline (the
"lid") carry the condition d(phi)/dy = 0 seen from inside the body (mode
'neumann'). The interior homogeneous problem (phi = 0 on the hull, rigid lid)
then has only the trivial solution, so the irregular frequencies of the plain
source method disappear. Imposing phi = 0 on the lid instead (mode
'dirichlet') does NOT work: it diverges with refinement (waterline corner).
Checked against Ursell multipoles (heave_2d.py) for the semicircle: plain
Frank (60 panels) has irregular frequencies at xi = KR ~ 1.82 and ~ 4.78,
where its added mass is off by up to 60 %; with the lid it stays within
~1.5 % of Ursell for xi 0.5..4.8.
Run: python3 -I heave_2d_lid.py   (numpy + scipy). Written by Claude, 2026-10-23."""
import sys
sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
import numpy as np
from scipy import special
import heave_2d as h

GX, GW = h.GX, h.GW


def kernel(x, y, a, b, K, csign=-1.0):
    """G and grad at field (x,y) for sources at quadrature points (a,b), (Rankine + wave)."""
    dx, dy = x - a, y - b
    dyi = y + b
    r1 = dx ** 2 + dy ** 2
    r2 = dx ** 2 + dyi ** 2
    W = dyi + 1j * dx
    Fv = np.exp(K * W) * special.exp1(K * W) + 1j * np.pi * np.sign(dx) * np.exp(K * W)
    dF = -1.0 / W + K * Fv
    ex = np.exp(K * dyi)
    wave = -2 * Fv.real + 1j * csign * 2 * np.pi * ex * np.cos(K * dx)
    gyw = -2 * dF.real + 1j * csign * 2 * np.pi * K * ex * np.cos(K * dx)
    gxw = -2 * (1j * dF).real - 1j * csign * 2 * np.pi * K * ex * np.sin(K * dx)
    with np.errstate(divide="ignore", invalid="ignore"):
        phi = 0.5 * np.log(r1) - 0.5 * np.log(r2) + wave
        gx = dx / r1 - dx / r2 + gxw
        gy = dy / r1 - dyi / r2 + gyw
    return phi, gx, gy


def solve(a1, a3, ms, K, n=60, nlid=20, lid=True, mode='dirichlet'):
    p0, p1 = h.panels(a1, a3, ms, n)
    b_half = p1[-1, 0]              # waterline half-beam
    if lid:
        xs = np.linspace(b_half, 0.0, nlid + 1)   # waterline -> centreline
        l0 = np.stack([xs[:-1], np.zeros(nlid)], 1)
        l1 = np.stack([xs[1:], np.zeros(nlid)], 1)
        p0 = np.vstack([p0, l0]); p1 = np.vstack([p1, l1])
    nh = n
    mid = 0.5 * (p0 + p1)
    d = p1 - p0
    L = np.hypot(d[:, 0], d[:, 1])
    t = d / L[:, None]
    nrm = np.stack([t[:, 1], -t[:, 0]], 1)
    N = len(L)
    A = np.zeros((N, N), complex)
    P = np.zeros((N, N), complex)
    for j in range(N):
        w = GW / 2 * L[j]
        for mirror in (1.0, -1.0):
            q = p0[j] + np.outer((GX + 1) / 2, d[j])
            q[:, 0] *= mirror
            for i in range(N):
                x, y = mid[i]
                self_panel = (i == j and mirror > 0)
                phi, gx, gy = kernel(x, y, q[:, 0], q[:, 1], K)
                if self_panel and j < nh:
                    # hull self panel: drop ln r1 (analytic), keep the rest
                    dx, dy = x - q[:, 0], y - q[:, 1]
                    r1 = dx ** 2 + dy ** 2
                    phi = phi - 0.5 * np.log(r1)
                    gx = gx - dx / r1; gy = gy - dy / r1
                    ph = np.sum(w * phi) + L[j] * (np.log(L[j] / 2) - 1)
                    vn = np.sum(w * (gx * nrm[i, 0] + gy * nrm[i, 1])) + np.pi
                elif self_panel:
                    # lid self panel on y = 0: r1 = r2, wave part ~ 2 ln|dx| + smooth;
                    # d/dy from below of 2 ln r over the sheet gives -2 pi sigma
                    dx = x - q[:, 0]
                    ph = np.sum(w * (phi - 2 * np.log(np.abs(dx)))) + 2 * L[j] * (np.log(L[j] / 2) - 1)
                    vn = -2 * np.pi + np.sum(w * gy)
                else:
                    ph = np.sum(w * phi)
                    vn = np.sum(w * (gx * nrm[i, 0] + gy * nrm[i, 1]))
                A[i, j] += vn
                P[i, j] += ph
    M = A.copy()
    rhs = np.zeros(N, complex)
    rhs[:nh] = nrm[:nh, 1]
    if lid and mode == 'dirichlet':
        M[nh:, :] = P[nh:, :]       # phi = 0 on the lid
    elif lid:
        pass                        # A rows on the lid: d(phi)/dy = 0 (lid normal is +y)
    sig = np.linalg.solve(M, rhs)
    phi = P[:nh] @ sig
    return 2 * np.sum(phi * nrm[:nh, 1] * L[:nh])


if __name__ == "__main__":
    nm = h.RHO * np.pi / 2
    print("xi     Ursell a  Ursell b | lid a    lid b    | plain a  plain b   (a/(rho pi R^2/2), b/(rho pi R^2/2 w))")
    for xi in [0.5, 1.0, 1.5, 1.82, 2.5, 3.0, 4.0, 4.78]:
        Iu, _ = h.ursell_solve(1.0, xi)
        Il = solve(0, 0, 1, xi, n=60, nlid=20, lid=True, mode='neumann')
        If = solve(0, 0, 1, xi, n=60, lid=False)
        g = lambda I: (-h.RHO * I.real / nm, -h.RHO * I.imag / nm)
        print(f"{xi:5.2f}  {g(Iu)[0]:8.4f} {g(Iu)[1]:8.4f} | {g(Il)[0]:8.4f} {g(Il)[1]:8.4f} | {g(If)[0]:8.4f} {g(If)[1]:8.4f}")
