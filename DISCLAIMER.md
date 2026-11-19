# Disclaimer

This disclaimer covers the software, models, data, documentation and every other file in this
repository and in anything built from it (together, "the files").

## Results are not validated

Most of SeaCraft's simulation calculations have not been validated against tank tests, sea trials,
full-scale measurements or approved naval-architecture software. That includes hydrostatics,
stability, damage stability, the weather criterion, resistance, planing and seakeeping.

The automated tests mostly check closed-form cases, such as a box barge, and scripts written for the
project. Many of those reference numbers are not published values, and some simply freeze SeaCraft's
own earlier output to catch changes. [`docs/oracles.md`](docs/oracles.md) says where each one came
from. Passing those tests shows that the code agrees with those cases, not that its results are right
for a real vessel.

One result has been compared with published tank tests: Michell wave resistance for the Wigley parabolic hull
(L/B 10, B/T 1.6) at Froude numbers 0.28–0.37, against Kajitani et al. (1983). It is not a match: Michell
overpredicts the resistance hump near Fn 0.32 by 18–54 % and places it about 0.02 too early, while its hollow
near Fn 0.35 is at the measured speed. The test holds the program to those bands, read from the published
figures ([`docs/oracles.md`](docs/oracles.md) §39). It covers that hull and speed range only.

The wave pattern (`seacraft waves`, the MCP tool `wave_pattern`, the Wave pattern view and Plan view contours in the
window, the design report) is far-field thin-ship theory: free waves only, without the local disturbance, so near the hull
it is not the hull wave profile. It is computed upright at the draft; heel and trim are ignored. It has been
checked against its own far-field asymptotics and Kelvin's wedge geometry ([`docs/oracles.md`](docs/oracles.md)
§40), not against a measured wave pattern: none was found for the Wigley hull. Transom sterns are computed and
flagged, not refused. The wash height and the catamaran interference are thin-ship estimates in deep water: the
wash height is not a wash assessment against any rule or limit, and the interference has not been compared with
a published catamaran result or a towing test ([`docs/oracles.md`](docs/oracles.md) §41). The speed-regime
sentences use SeaCraft's own bands, not published thresholds.

Building patterns (station moulds, profile and transom) and the printed table of offsets are geometry the
program computed from the hull. They have not been compared with a built boat, and no printed set has been
measured. A builder must print at 100 %, measure the printed calibration rules, and check dimensions against
the design before cutting anything.

Plywood panel patterns are the least-stretch flattening of the modelled surface. Their stretch and seam drift
are geometry measured against the model, not a bending test of plywood: a panel the program says lies flat may
still not take the shape, and no plywood thickness is allowed for. The 2 mm and 0.1 % limits are the project's
provisional judgement, not a published rule, and no boat has yet been built from these patterns.

Results can be wrong, including results that look plausible. A qualified person must check them
independently before anyone relies on them for a design, construction, operating or safety decision.

## Not an approval

SeaCraft does not approve anything. A `PASS` against an IMO, SOLAS or other criterion is arithmetic
on the program's own numbers. It does not show compliance, and it does not replace the approval of a
classification society, flag state, or other authority. A design report produced by SeaCraft is not
a stability booklet or a certificate.

## No warranty

The files are provided "as is", without warranty of any kind, express or implied, including warranties
of accuracy, merchantability, fitness for a particular purpose and non-infringement.

## No liability

To the fullest extent permitted by law, the SeaCraft contributors are not liable for any claim, damage,
loss, injury or other liability arising from the files or from their use, modification or distribution,
whether in contract, tort or otherwise.

## No indemnity

No contributor indemnifies, defends or holds harmless anyone who uses, modifies or distributes the
files, including against claims that the files infringe someone else's rights.

## Your responsibility

Whoever uses the files is solely responsible for how they use them, for what they make with them, and
for complying with the laws and rules that apply. The contributors do not authorise, endorse or accept
responsibility for any harmful or unlawful use.

## No endorsement

Using, modifying or distributing the files does not mean that any contributor endorses you, your
product, your vessel or your results. Do not use a contributor's name to suggest such an endorsement
without written permission.

## Relation to the licence

This disclaimer adds to the warranty and liability terms of the licence the files are distributed under
([`LICENSE`](LICENSE), sections 15 and 16). It does not restrict any right that licence grants; where
the two differ, the licence governs.
