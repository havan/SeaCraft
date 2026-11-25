"""Tests for the pure parts of build.py: python3 site/test_build.py

They cover how build.py reads seacraft's output, never a number of its own: the numbers come from the
program, and its own tests check them.
"""

import importlib.util
import json
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

    def test_seacrafts_warnings_are_kept(self):
        result = heel_result({"kind": "stable", "state": STATE}, "s")
        result["warnings"] = ["the hull is open (no deck faces) and its open edge is below the waterline"]
        self.assertEqual(build.heel_step(result, 0.0)["warnings"], result["warnings"])
        self.assertEqual(build.heel_step(heel_result({"kind": "cannot_float"}, "s"), 0.0)["warnings"], [])

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


class TestScreenshots(unittest.TestCase):
    def test_a_missing_screenshot_stops_the_build(self):
        with tempfile.TemporaryDirectory() as scratch:
            source, out = Path(scratch) / "assets", Path(scratch) / "out"
            source.mkdir()
            out.mkdir()
            (source / "gui.png").write_bytes(b"png")
            with self.assertRaises(SystemExit) as stop:
                build.copy_screenshots(source, out)
            self.assertIn("gui-waves.png", str(stop.exception.code))

    def test_every_screenshot_is_copied(self):
        with tempfile.TemporaryDirectory() as scratch:
            source, out = Path(scratch) / "assets", Path(scratch) / "out"
            source.mkdir()
            out.mkdir()
            for name in build.SCREENSHOTS:
                (source / name).write_bytes(b"png")
            build.copy_screenshots(source, out)
            self.assertEqual(sorted(p.name for p in out.iterdir()), sorted(build.SCREENSHOTS))


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


def fake_waves(scratch, warnings):
    """A stand-in for `seacraft waves` that logs each call, writes its -o file and prints a result: JSON
    with `--json`, otherwise each warning as the Turkish catalog prints it."""
    log = Path(scratch) / "calls.log"
    result = json.dumps(
        {
            "grid_points": [10, 5],
            "regime": {"key": "rising", "sentence": "A regime."},
            "max_abs_zeta_m": 0.1,
            "warnings": warnings,
        }
    )
    printed = "".join(f"echo 'uyarı: {w} (tr)' >&2\n" for w in warnings)
    fake = Path(scratch) / "seacraft"
    fake.write_text(
        "#!/bin/sh\n"
        f"echo \"$*\" >> '{log}'\n"
        'out=""; json=""\n'
        'while [ $# -gt 0 ]; do [ "$1" = "-o" ] && out="$2"; [ "$1" = "--json" ] && json=1; shift; done\n'
        '[ -n "$out" ] && printf png > "$out"\n'
        f"if [ -n \"$json\" ]; then echo '{result}'; else\n{printed}:; fi\n",
        encoding="utf-8",
    )
    fake.chmod(0o755)
    return fake, log


class TestWaveSteps(unittest.TestCase):
    def setUp(self):
        self.shrink = build.shrink_png
        build.shrink_png = lambda path: None  # The fake's files are not PNGs.

    def tearDown(self):
        build.shrink_png = self.shrink

    def test_a_fitting_hull_computes_each_speed_once(self):
        with tempfile.TemporaryDirectory() as scratch:
            fake, log = fake_waves(scratch, [])
            waves = build.wave_steps(fake, "wigley", Path(scratch), 10.0)
            calls = log.read_text(encoding="utf-8").splitlines()
            self.assertTrue(waves["fits"])
            self.assertEqual(len(waves["steps"]), len(build.WAVE_FROUDES))
            # The regime's words in Turkish come from the program's catalog, by the regime's key.
            self.assertEqual(waves["steps"][0]["regime"]["en"], "A regime.")
            self.assertEqual(waves["steps"][0]["regime"]["tr"], build.program_string("tr", "wave.regime.rising"))
            self.assertEqual(len(calls), len(build.WAVE_FROUDES))
            self.assertTrue(all("--spacing" in c for c in calls))
            self.assertTrue((Path(scratch) / "waves" / "fn-0.35.png").exists())

    def test_a_hull_seacraft_warns_about_keeps_its_words_and_no_images(self):
        with tempfile.TemporaryDirectory() as scratch:
            fake, log = fake_waves(scratch, ["beam/length 0.40 is not slender"])
            waves = build.wave_steps(fake, "barge", Path(scratch), 10.0)
            reason = {"en": ["beam/length 0.40 is not slender"], "tr": ["beam/length 0.40 is not slender (tr)"]}
            self.assertEqual(waves, {"fits": False, "steps": [], "reason": reason})
            self.assertFalse((Path(scratch) / "waves").exists())
            # Once for the numbers, once more for each other language's words.
            self.assertEqual(len(log.read_text(encoding="utf-8").splitlines()), len(build.LANGS))


class TestProgramWords(unittest.TestCase):
    """Reading seacraft's human-readable output in another language, against the program's own catalog."""

    def test_a_template_matches_its_whole_line(self):
        pattern = build.template_pattern("  uyarı: {message}")
        self.assertEqual(pattern.match("  uyarı: L/B 2.79 (x)").group(1), "L/B 2.79 (x)")
        self.assertIsNone(pattern.match("not: uyarı: no"))

    def test_warnings_are_read_in_order_whichever_template_printed_them(self):
        printed = "\n".join([
            "22 m balıkçı teknesi  -  başlık",
            "  uyarı: birinci",
            "uyarı: ikinci",
            "  UYARI: üçüncü",
            "  not: bir not, uyarı değil",
        ])
        self.assertEqual(build.warnings_in(printed, "tr"), ["birinci", "ikinci", "üçüncü"])

    def test_where_the_hull_settles_is_its_row(self):
        printed = "  ağırlıklarla   deplasman 190.875 t\n  denge          sancağa 10.4° meyleder, trim -0.083 m\n"
        self.assertEqual(build.settles_in(printed, "tr"), "sancağa 10.4° meyleder, trim -0.083 m")

    def test_no_settles_row_stops_the_build(self):
        with self.assertRaises(SystemExit):
            build.settles_in("  ağırlık  1 t\n", "tr")

    def test_a_different_count_of_warnings_stops_the_build(self):
        with tempfile.TemporaryDirectory() as scratch:
            fake = Path(scratch) / "seacraft"
            fake.write_text("#!/bin/sh\necho 'uyarı: bir' >&2\necho 'uyarı: iki' >&2\n", encoding="utf-8")
            fake.chmod(0o755)
            with self.assertRaises(SystemExit) as stop:
                build.said(fake, ["only one"], "resistance", "x.fef")
            self.assertIn("1 warnings in --json, 2 in the tr output", str(stop.exception.code))

    def test_a_name_is_the_description_up_to_its_colon(self):
        self.assertEqual(build.hull_name("22 m balıkçı teknesi: Çelik tekne: skeg"), "22 m balıkçı teknesi")

    def test_each_language_has_its_own_document(self):
        self.assertEqual(build.localized_file("report.html", "en"), "report.html")
        self.assertEqual(build.localized_file("lines.svg", "tr"), "lines.tr.svg")


# Keys of the site's catalogs: lowercase ASCII, dots between parts; a hull's key carries its id.
KEY = re.compile(r"^[a-z0-9_.-]+$")
PLACEHOLDER = re.compile(r"\{\{?[a-z_]+\}\}?")
TAG = re.compile(r"</?([a-z]+)[^>]*>")


def catalog_text(lang):
    return (build.SITE_LOCALES / f"{lang}.json").read_text(encoding="utf-8")


class TestSiteCatalogs(unittest.TestCase):
    """site/locales: every language says the same things, in the same shape, and says nothing unused."""

    def setUp(self):
        self.catalogs = {lang: build.site_strings(lang) for lang in build.LANGS}

    def test_every_language_has_a_catalog_with_the_same_keys(self):
        english = set(self.catalogs["en"])
        for lang, strings in self.catalogs.items():
            with self.subTest(lang=lang):
                self.assertEqual(set(strings) - english, set(), "keys English does not have")
                self.assertEqual(english - set(strings), set(), "keys missing")

    def test_catalogs_are_sorted_and_formatted(self):
        for lang, strings in self.catalogs.items():
            with self.subTest(lang=lang):
                expected = json.dumps(strings, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
                self.assertEqual(catalog_text(lang), expected)
                for key, text in strings.items():
                    self.assertRegex(key, KEY)
                    self.assertTrue(text.strip() or key.endswith(("noscript_folder", "number.group")), key)

    def test_placeholders_and_tags_match_english(self):
        english = self.catalogs["en"]
        for lang, strings in self.catalogs.items():
            for key, text in strings.items():
                with self.subTest(lang=lang, key=key):
                    self.assertEqual(sorted(PLACEHOLDER.findall(text)), sorted(PLACEHOLDER.findall(english[key])))
                    self.assertEqual(sorted(TAG.findall(text)), sorted(TAG.findall(english[key])))

    def test_strings_in_attributes_hold_no_markup_or_quotes(self):
        template = (build.SRC / "index.html").read_text(encoding="utf-8")
        in_attributes = set(re.findall(r'="[^"]*\{\{t:([a-z0-9._-]+)\}\}[^"]*"', template))
        for lang, strings in self.catalogs.items():
            for key in in_attributes:
                with self.subTest(lang=lang, key=key):
                    self.assertNotRegex(strings[key], r'[<>"]')

    def test_every_key_is_used_and_every_used_key_exists(self):
        template = (build.SRC / "index.html").read_text(encoding="utf-8")
        script = (build.SRC / "app.js").read_text(encoding="utf-8")
        used = set(build.STRING.findall(template))
        used |= set(re.findall(r't\("([a-z0-9._-]+)"', script))
        # Keys app.js builds from a part: a chart's labels, a hull's id, a method's name.
        for prefix in re.findall(r't\(`([a-z0-9._-]+)\.\$\{', script):
            used |= {k for k in self.catalogs["en"] if k.startswith(prefix + ".")}
        used |= {k for k in self.catalogs["en"] if k.startswith(("hull.", "method.", "js.sac.", "js.gz.", "js.resistance."))}
        used |= {k for k in self.catalogs["en"] if k.startswith("js.motions.")}
        used.add("page.lang.name")  # The switch names each language in its own words.
        self.assertEqual(used - set(self.catalogs["en"]), set(), "used but missing")
        self.assertEqual(set(self.catalogs["en"]) - used, set(), "never used")

    def test_every_hull_and_method_has_a_name(self):
        for lang, strings in self.catalogs.items():
            for hull in build.HULLS:
                with self.subTest(lang=lang, hull=hull["id"]):
                    self.assertIn(f"hull.{hull['id']}", strings)
                    self.assertIn(f"method.{hull['resistance']}", strings)


class TestRenderPage(unittest.TestCase):
    def test_a_page_is_filled_and_its_strings_reach_the_script(self):
        values = build.page_values("tr", "0.7.0", "abc1234", "0.7.0")
        page = build.render_page('<html lang="{{lang}}">{{t:page.skip}} {{root}}<script>{{strings}}</script>', "tr", values)
        self.assertTrue(page.startswith('<html lang="tr">İçeriğe geç ../<script>'))
        strings = json.loads(page.split("<script>")[1].split("</script>")[0].replace("<\\/", "</"))
        self.assertEqual(strings["js.sea.hint"], build.site_strings("tr")["js.sea.hint"])
        self.assertNotIn("page.skip", strings)

    def test_a_missing_key_or_value_stops_the_build(self):
        values = build.page_values("en", "0.7.0", "abc1234", "0.7.0")
        with self.assertRaises(SystemExit):
            build.render_page("{{t:page.no_such_key}}", "en", values)
        with self.assertRaises(SystemExit):
            build.render_page("{{no_such_value}}", "en", values)

    def test_the_switch_links_every_other_language(self):
        english = build.page_values("en", "v", "c", "r")
        self.assertIn('href="tr/" hreflang="tr" lang="tr"', english["languages"])
        self.assertIn('hreflang="x-default" href="https://seacraft.app/"', english["alternates"])
        self.assertIn('href="../" hreflang="en"', build.page_values("tr", "v", "c", "r")["languages"])


if __name__ == "__main__":
    unittest.main()
