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

Building patterns (station moulds, profile and transom) and the printed table of offsets are geometry the
program computed from the hull. They have not been compared with a built boat, and no printed set has been
measured. A builder must print at 100 %, measure the printed calibration rules, and check dimensions against
the design before cutting anything.

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
