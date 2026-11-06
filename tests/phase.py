"""Run with python3 -m unittest discover -s tests -p phase.py."""
import copy
import importlib.util
import math
import json
import tempfile
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("phase", Path(__file__).parents[1] / "tools/phase.py")
phase = importlib.util.module_from_spec(spec)
spec.loader.exec_module(phase)


class PhaseTests(unittest.TestCase):
    def setUp(self):
        self.summary = {"fit": {"media": "cd", "sector_bits": 57624,
                               "read_tail_frames": 108, "u": 89491.8, "v": 5.4067},
                        "setup": {"experiment_range": [122000, 356000]}}
        self.manifest = {"format": "discimage-extra-v1", "plan": {
            "estimated_art_start_sector": 130520, "estimated_art_end_sector": 359697,
            "disc": {"capacity_sectors": 359847, "inner_radius_mm": 24.5,
                     "outer_radius_mm": 57.2275, "velocity": 1183.1773}}}
        self.model = phase.Model(self.summary, self.manifest)

    def samples(self, shift, rotation):
        # Independent cumulative-angle form of the constant-pitch spiral.
        # Deliberately includes an arbitrary photo rotation of several turns.
        u, v = self.model.u, self.model.v
        samples = []
        for i in range(40):
            s = 150000 + 4900 * i
            measured = 2 * math.pi / v * (math.sqrt(u*u + v * (s*57624 + shift*588) / math.pi) - u)
            r = math.sqrt(24.5**2 + s / 359847 * (57.2275**2 - 24.5**2))
            generated = 2 * 1183.1773 * (359847/75) / (57.2275**2 - 24.5**2) * (r - 24.5)
            samples.append((s, math.degrees(measured-generated) + rotation))
        return samples

    def test_recovers_known_shift_and_holds_out_another_orientation(self):
        groups = {"first": self.samples(92, 720), "second": self.samples(92, -35)}
        result = phase.analyze(self.model, groups)
        self.assertAlmostEqual(result["joint"]["shift_frames"], 92, places=4)
        self.assertAlmostEqual(result["joint"]["forward_offset_frames"], 19, places=4)
        for held in result["holdouts"]:
            self.assertLess(held["max_deg"], 1e-7)
            self.assertFalse(held["shift_refitted"])
        self.assertFalse(result["physical_validation"])
        self.assertFalse(result["profile_applied"])

    def test_disagreeing_curves_are_visible_in_holdouts(self):
        result = phase.analyze(self.model, {"a": self.samples(80, 0), "b": self.samples(150, 0)})
        self.assertAlmostEqual(result["individual"]["a"]["shift_frames"], 80, places=4)
        self.assertGreater(result["holdouts"][0]["rms_deg"], 1)

    def test_unknown_completion_origin_and_dvd_are_rejected(self):
        for key, value in [("media", "dvd"), ("read_tail_frames", 111), ("u", float("nan"))]:
            changed = copy.deepcopy(self.summary)
            changed["fit"][key] = value
            with self.assertRaises(ValueError):
                phase.Model(changed, self.manifest)

    def test_points_must_match_bundle_range_and_continuous_edge(self):
        points = {"format": "discimage-phase-v1", "bundle_sha256": "abc",
                  "evidence_sha256": "0" * 64, "convention": "photo-minus-design-clockwise",
                  "samples": [{"curve": "a", "sector": s, "phase_deg": p}
                              for s, p in self.samples(100, 0)]}
        self.assertEqual(len(phase.curves(points, self.model, "abc")["a"]), 40)
        with self.assertRaises(ValueError):
            phase.curves(points, self.model, "wrong")
        for key, value in [("sector", 360000), ("phase_deg", points["samples"][0]["phase_deg"] + 360)]:
            changed = copy.deepcopy(points)
            changed["samples"][0][key] = value
            with self.assertRaises(ValueError):
                phase.curves(changed, self.model, "abc")

    def test_profile_geometry_takes_precedence_and_bounds_are_not_a_fit(self):
        changed = copy.deepcopy(self.manifest)
        changed["plan"]["disc"]["calibration"] = {
            "inner_radius_mm": 25, "outer_radius_mm": 58, "reference_sectors": 350000}
        model = phase.Model(self.summary, changed)
        self.assertEqual(model.inner, 25)
        self.assertEqual(model.capacity, 350000)
        with self.assertRaises(ValueError):
            phase.fit(self.model, {"outside": self.samples(1500, 0)})

    def test_external_offset_is_checked_without_being_refitted(self):
        groups = {"a": self.samples(92, 10), "b": self.samples(92, -80)}
        result = phase.analyze(self.model, groups, 19)
        self.assertEqual(result["offset_source"], "external-measurement")
        self.assertEqual(result["joint"]["shift_frames"], 92)
        self.assertLess(result["joint"]["rms_deg"], 1e-7)
        self.assertTrue(all(row["shift_frames"] == 92 for row in result["holdouts"]))
        wrong = phase.analyze(self.model, groups, 50)
        self.assertGreater(wrong["joint"]["rms_deg"], 0.1)

    def test_mac_summary_uses_actual_targets_not_track_bounds(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "summary.json"
            summary = copy.deepcopy(self.summary)
            summary["setup"] = {"readable_lba_range": [0, 359000]}
            summary["fit"]["reference_lba"] = 239000
            path.write_text(json.dumps(summary))
            csv = path.with_name("targets.csv")
            csv.write_text("lba,used\n" + "".join(f"{150000+i*10000},32\n" for i in range(8)))
            loaded = phase.load_timing(path)
            self.assertEqual(loaded["setup"]["experiment_range"], [150000, 239001])
            self.assertNotIn("experiment_range", json.loads(path.read_text())["setup"])
            summary["setup"]["readable_lba_range"] = [200000, 359000]
            path.write_text(json.dumps(summary))
            with self.assertRaises(ValueError):
                phase.load_timing(path)


if __name__ == "__main__":
    unittest.main()
