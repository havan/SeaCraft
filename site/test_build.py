"""Tests for the pure parts of build.py: python3 -m unittest site/test_build.py

They cover how build.py reads seacraft's output, never a number of its own: the numbers come from the
program, and its own tests check them.
"""

import importlib.util
import re
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("build", HERE / "build.py")
build = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(build)


class TestReleaseVersion(unittest.TestCase):
    def test_reads_workspace_version(self):
        with tempfile.TemporaryDirectory() as scratch:
            cargo = Path(scratch) / "Cargo.toml"
            cargo.write_text('[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.6.0"\n', encoding="utf-8")
            self.assertEqual(build.release_version(cargo), "0.6.0")

    def test_repo_version_is_plain(self):
        self.assertRegex(build.release_version(build.ROOT / "Cargo.toml"), r"^\d+\.\d+\.\d+$")


STATE = {
    "heel_deg": 7.9557977145582255,
    "b": {"x": 9.194826335303294, "y": 0.23478098670614922, "z": 1.6848628324605275},
    "g": {"x": 9.194826298515899, "y": 0.04933044305402092, "z": 3.011839306332965},
    "lever_m": -4.22914647746353e-10,
    "gz_m": 0.04885564415449278,
    "heeling_arm_m": 0.04885564457740743,
    "draft": 2.626378201037529,
    "trim_m": -0.022015092552717608,
    "displacement_t": 177.37525670341182,
    "lid_submerged": False,
    "waterplane": {"n": {"x": 0.0003, "y": -0.13841, "z": 0.99037}, "d": -2.60112},
}


def heel_result(equilibrium, summary):
    return {"equilibrium": equilibrium, "summary": summary, "warnings": []}


class TestHeelStep(unittest.TestCase):
    def test_stable_step_carries_the_state(self):
        step = build.heel_step(
            heel_result({"kind": "stable", "state": STATE}, "heels 8.0° to starboard, trim -0.022 m, draft 2.626 m"),
            2.5,
        )
        self.assertEqual(step["kind"], "stable")
        self.assertEqual(step["y"], 2.5)
        self.assertEqual(step["heel_deg"], 7.96)
        self.assertEqual(step["b"], [0.2348, 1.6849])
        self.assertEqual(step["g"], [0.0493, 3.0118])
        self.assertEqual(step["gz_m"], 0.0489)
        self.assertEqual(step["heeling_arm_m"], 0.0489)
        self.assertEqual(step["draft"], 2.626)
        self.assertEqual(step["trim_m"], -0.022)
        self.assertEqual(step["waterplane"], [0.0003, -0.13841, 0.99037, -2.60112])
        self.assertEqual(step["summary"], "heels 8.0° to starboard, trim -0.022 m, draft 2.626 m")

    def test_steps_that_do_not_settle_draw_nothing(self):
        cases = [
            {"kind": "capsizes", "last_heel_deg": 41.0},
            {"kind": "loll", "port": STATE, "starboard": STATE},
            {"kind": "cannot_float"},
            {"kind": "unresolved", "reason": "no trim puts the LCB on G"},
        ]
        for equilibrium in cases:
            with self.subTest(kind=equilibrium["kind"]):
                step = build.heel_step(heel_result(equilibrium, f"as seacraft says: {equilibrium['kind']}"), 1.0)
                self.assertEqual(step["kind"], equilibrium["kind"])
                self.assertEqual(step["summary"], f"as seacraft says: {equilibrium['kind']}")
                for key in ("heel_deg", "b", "g", "gz_m", "heeling_arm_m", "draft", "trim_m", "waterplane"):
                    self.assertIsNone(step[key], key)

    def test_a_state_without_draft_keeps_the_gap(self):
        state = dict(STATE, draft=None, trim_m=None)
        step = build.heel_step(heel_result({"kind": "stable", "state": state}, "s"), 0.0)
        self.assertIsNone(step["draft"])
        self.assertIsNone(step["trim_m"])


LB_WARNING = "L/B is 3.4: thin-ship theory assumes a slender hull (L/B above about 6)"


class TestWaveFits(unittest.TestCase):
    def test_no_warnings_at_any_speed_fits(self):
        self.assertTrue(build.wave_fits([{"warnings": []}, {"warnings": []}]))

    def test_one_warning_at_one_speed_does_not_fit(self):
        self.assertFalse(build.wave_fits([{"warnings": []}, {"warnings": [LB_WARNING]}]))

    def test_no_results_do_not_fit(self):
        self.assertFalse(build.wave_fits([]))


class TestSectionTop(unittest.TestCase):
    def test_the_top_of_the_part_that_reaches_the_centreline(self):
        # A trawler's section: the hull from the deck at the centreline round to the keel, and a
        # bulwark standing above the sheer as a separate line. The deck is the hull's top, not the rail's.
        hull = "M0.000,-3.423L3.397,-3.425L3.2,-1.0L0.000,-0.000"
        bulwark = "M3.400,-4.423L3.400,-3.423"
        self.assertEqual(build.section_top(hull + bulwark), 3.425)

    def test_an_open_boat_is_topped_by_its_sheer(self):
        self.assertEqual(build.section_top("M-0.000,-0.001L-0.498,-0.001L-0.716,-0.420"), 0.42)


class TestRun(unittest.TestCase):
    def test_a_failing_run_stops_the_build_naming_the_command(self):
        with tempfile.TemporaryDirectory() as scratch:
            fake = Path(scratch) / "seacraft"
            fake.write_text("#!/bin/sh\necho 'no equilibrium: hull is open' >&2\nexit 2\n", encoding="utf-8")
            fake.chmod(0o755)
            with self.assertRaises(SystemExit) as stop:
                build.heel_steps(fake, "wigley", scratch, 0.4, 1.0, [{"x": 5.0, "body": "M0,0"}])
            message = str(stop.exception.code)
            self.assertIn("seacraft hydro wigley.fef", message)
            self.assertIn("no equilibrium: hull is open", message)
            self.assertFalse((Path(scratch) / "data.json").exists())


if __name__ == "__main__":
    unittest.main()
