"""Exercise the fail-closed statistics check without launching a server."""

import unittest

from runner import trace_count


def info(rows):
    return {"primary": {"stats": {"global": (
        '<table class="analysis-stats">'
        '<tr><th class="query-column">Name</th><th>Count</th></tr>'
        + rows + '</table>'
    )}}}


class TraceCountTests(unittest.TestCase):
    def test_empty_instrumented_table_is_zero(self):
        self.assertEqual(trace_count(info("")), 0)

    def test_aggregate_is_not_double_counted_with_per_file_rows(self):
        result = info(
            '<tr><td>analyze_expr</td><td>2</td></tr>'
            '<tr><td>@ws/p0:0.0.0&quot;/main.typ&quot;:analyze_expr</td><td>2</td></tr>'
        )
        self.assertEqual(trace_count(result), 2)

    def test_missing_instrumentation_is_not_zero(self):
        for result in (None, {}, {"primary": {"stats": {}}},
                       {"primary": {"stats": {"global": ""}}}):
            with self.subTest(result=result), self.assertRaises(ValueError):
                trace_count(result)

    def test_unrecognized_schema_is_not_zero(self):
        with self.assertRaises(ValueError):
            trace_count({"primary": {"stats": {"global":
                        "<table><tr><th>Different</th><th>Count</th></tr></table>"}}})

    def test_shared_global_stats_are_not_summed_across_projects(self):
        result = info('<tr><td>analyze_expr</td><td>1</td></tr>')
        result["secondary"] = result["primary"]
        self.assertEqual(trace_count(result), 1)


if __name__ == "__main__":
    unittest.main()
