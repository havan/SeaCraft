"""Independent oracle for E2 (irregular seas): spectrum moments, significant
responses and the motion sickness incidence. Run: python3 -I irregular_seas.py

Spectrum (ITTC / Bretschneider, two parameters Hs, Tp):
    S(w) = 5/16 Hs^2 wp^4 w^-5 exp(-5/4 (wp/w)^4),  wp = 2 pi / Tp
Moments m_n = int w^n S dw = (A/4) B^((n-4)/4) Gamma(1 - n/4), with
A = 5/16 Hs^2 wp^4, B = 5/4 wp^4  ->  m0 = Hs^2/16 exactly,
T1 = 2 pi m0/m1 = 0.7718 Tp, Tz = 2 pi sqrt(m0/m2) = 0.7104 Tp (m4 diverges).

JONSWAP: S_J = N * S_PM * gamma^r, r = exp(-(w-wp)^2 / (2 s^2 wp^2)),
s = 0.07 (w <= wp) / 0.09 (w > wp); N fixed so that m0 = Hs^2/16
(Goda's closed form N ~ 1 - 0.287 ln gamma is within ~1 %).

Response in head seas: m_n^R = int |H(w)|^2 we^n S(w) dw over the incident
frequency (no Jacobian), we = w + w^2 U/g. Significant amplitude = 2 sqrt(m0^R).

MSI (O'Hanlon & McCauley 1974, 2-hour exposure, as in Lloyd, Seakeeping):
    MSI = 100 Phi((log10(a/g) - mu) / 0.4),  mu = -0.819 + 2.32 (log10 w)^2
with a the mean absolute vertical acceleration (0.798 sqrt(m4) for a
Gaussian process) and w in rad/s.
"""
import numpy as np
from scipy import special, stats, integrate

G = 9.81


def bretschneider(w, hs, tp):
    wp = 2 * np.pi / tp
    return 5 / 16 * hs**2 * wp**4 * w**-5.0 * np.exp(-1.25 * (wp / w) ** 4)


def jonswap(w, hs, tp, gamma):
    """JONSWAP on the grid `w`, scaled so that m0 = Hs^2/16; also returns the
    scale relative to the plain Bretschneider shape (compare with Goda)."""
    wp = 2 * np.pi / tp

    def raw(x):
        s = np.where(x <= wp, 0.07, 0.09)
        return bretschneider(x, hs, tp) * gamma ** np.exp(-((x - wp) ** 2) / (2 * s**2 * wp**2))

    m0 = integrate.quad(raw, 1e-3, 60 * wp, points=[wp], limit=800)[0]
    n = (hs**2 / 16) / m0
    return raw(w) * n, n


def moment(f, n):
    return integrate.quad(lambda w: w**n * f(w), 1e-3, np.inf, limit=400)[0]


def msi(a_over_g, w):
    mu = -0.819 + 2.32 * np.log10(w) ** 2
    return 100 * stats.norm.cdf((np.log10(a_over_g) - mu) / 0.4)


if __name__ == "__main__":
    hs, tp = 2.0, 8.0
    wp = 2 * np.pi / tp
    A, B = 5 / 16 * hs**2 * wp**4, 1.25 * wp**4
    f = lambda w: bretschneider(w, hs, tp)
    print("Bretschneider Hs=2 Tp=8")
    for n in (0, 1, 2):
        print(f"  m{n}: numeric {moment(f, n):.6f}  analytic {(A/4) * B**((n-4)/4) * special.gamma(1 - n/4):.6f}")
    m0, m1, m2 = (moment(f, n) for n in (0, 1, 2))
    print(f"  Hs from m0 {4*np.sqrt(m0):.4f}; T1/Tp {2*np.pi*m0/m1/tp:.4f}; Tz/Tp {2*np.pi*np.sqrt(m0/m2)/tp:.4f}")
    # truncated grid as an implementation would use it
    w = np.linspace(0.2 * wp, 5 * wp, 400)
    print(f"  m0 on [0.2wp, 5wp] trapezoid, 400 pts: {np.trapz(f(w), w):.6f} (exact {hs**2/16:.6f})")

    for gamma in (1.0, 3.3):
        sj, N = jonswap(w, hs, tp, gamma)
        print(f"JONSWAP gamma={gamma}: N/goda {N:.4f} / {1-0.287*np.log(gamma):.4f}; peak at w={w[np.argmax(sj)]:.4f} (wp {wp:.4f})")

    # flat RAO -> significant amplitude Hs/2 at any speed
    print("Flat RAO: significant amplitude =", 2 * np.sqrt(m0), "(Hs/2 =", hs / 2, ")")

    # a resonant RAO: H = 1/sqrt((1-(we/wn)^2)^2 + (2 z we/wn)^2), speed U
    U, wn, z = 3.0, 1.2, 0.2
    we = lambda w: w + w * w * U / G
    H2 = lambda w: 1 / ((1 - (we(w) / wn) ** 2) ** 2 + (2 * z * we(w) / wn) ** 2)
    for n in (0, 2, 4):
        print(f"  resonant RAO U={U} wn={wn} z={z}: m{n}^R = {integrate.quad(lambda w: H2(w) * we(w)**n * f(w), 1e-3, 50, limit=800)[0]:.6f}")

    print("MSI (2 h): a/g, w -> %")
    for a, wv in ((0.1, 1.0), (0.05, 1.0), (0.2, 1.0), (0.1, 0.5), (0.1, 2.0), (0.02, 1.2)):
        print(f"  {a:5.2f} {wv:4.1f} -> {msi(a, wv):7.3f}")
