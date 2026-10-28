#!/usr/bin/env python3
"""Offline regression checks for private judged sampling."""
import collections
import json
from pathlib import Path
import runpy
import unittest

ROOT = Path(__file__).resolve().parents[1]
PREP = runpy.run_path(str(ROOT / "scripts/prepare_cognee_judged_eval.py"))


class PreparationTests(unittest.TestCase):
    def test_bounded_excerpt_preserves_unicode_boundary(self):
        self.assertEqual(PREP["bounded"]("a😀z", 2), "a")
        self.assertEqual(PREP["bounded"]("a😀z", 3), "a😀")

    def test_reuses_heldout_scrubber_and_parser(self):
        base = PREP["BASE"]
        scrubber = base["Scrubber"]()
        raw = '\n'.join(json.dumps(row) for row in [
            {"type": "assistant", "timestamp": "2026-10-01T00:00:00Z", "message": {"role": "assistant", "content": "previous text"}},
            {"type": "user", "timestamp": "2026-10-01T00:01:00Z", "message": {"role": "user", "content": [{"type": "tool_result", "content": "tool text"}]}},
            {"type": "user", "timestamp": "2026-10-01T00:02:00Z", "message": {"role": "user", "content": "contact fixture@example.org"}},
        ]).encode()
        session, _ = base["normalize"]("claude", raw, "fixture", scrubber)
        self.assertEqual([turn["role"] for turn in session["turns"]], ["assistant", "tool", "user"])
        self.assertIn("[REDACTED_EMAIL]", session["turns"][-1]["text"])

    def test_prepared_sample_invariants_if_available(self):
        path = ROOT / "target/judged-eval/sample.json"
        if not path.exists():
            self.skipTest("private sample has not been prepared")
        rows = json.loads(path.read_text())
        self.assertEqual(len(rows), 200)
        self.assertLessEqual(max(collections.Counter(row["session"] for row in rows).values()), 3)
        self.assertNotIn("icers", {row["source"] for row in rows})
        self.assertTrue(all(len(row["previous_assistant"].encode("utf-16-le")) <= 1600 for row in rows))
        self.assertTrue(all(row["timestamp"] is not None for row in rows))


if __name__ == "__main__":
    unittest.main()
