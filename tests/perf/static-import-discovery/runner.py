#!/usr/bin/env python3
r"""Check that unused builtin imports do not trigger dynamic semantic analysis.

Requires Python 3.9+ and built Tinymist executables; uses only the standard library.
Run from the repository root:

    python3 tests/perf/static-import-discovery/runner.py \
        --binary target/release/tinymist

Compare a candidate against an unmodified binary, preserving semantic token data:

    python3 tests/perf/static-import-discovery/runner.py \
        --binary target/release/tinymist --baseline /path/to/baseline/tinymist \
        --require-baseline-trace

Each fixture starts a fresh server with semantic tokens enabled. The unused
function in main.typ is never called. The second line deliberately causes costly,
non-convergent layout to amplify accidental trace compilation; it is not a model
document or a preview benchmark. control.typ replaces import with a local binding.

The candidate must return nonempty semantic tokens and report zero analyze_expr
calls in the global server statistics. Missing/unparseable statistics fail closed.
When a baseline is supplied, the same fixture's complete token array and legend
must match; optional --require-baseline-trace confirms main.typ exercised the old
dynamic-analysis path. Baseline statistics are otherwise descriptive, allowing
comparison with older versions that predate the regression.

Elapsed times are reported, not used as pass/fail criteria by default, since debug
builds and shared machines differ. Use --max-seconds to enforce a candidate budget.
Opening the document may also schedule normal diagnostic compilation. The trace
counter measures dynamic analysis, not those ordinary compiler jobs; elapsed time
is end-to-end LSP latency under that load, not isolated analysis CPU time.
No editor installation or settings are modified. Logs, replies and summary.json
are written to --output-dir (default: ignored results/ alongside this script).
Servers have bounded request waits and are always terminated and reaped.
"""

import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
import time


class StatsTable(HTMLParser):
    """Extract table cells without depending on generated HTML whitespace."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.rows = []
        self.row = None
        self.cell = None

    def handle_starttag(self, tag, attrs):
        if tag == "tr":
            self.row = []
        elif tag in ("td", "th") and self.row is not None:
            self.cell = []

    def handle_data(self, data):
        if self.cell is not None:
            self.cell.append(data)

    def handle_endtag(self, tag):
        if tag in ("td", "th") and self.cell is not None:
            self.row.append("".join(self.cell).strip())
            self.cell = None
        elif tag == "tr" and self.row is not None:
            self.rows.append(self.row)
            self.row = None


def trace_count(server_info):
    """Read the aggregate counter once (not its per-file duplicate rows)."""
    if not isinstance(server_info, dict) or not server_info:
        raise ValueError("getServerInfo returned no project statistics")
    counts = []
    for info in server_info.values():
        stats = info.get("stats", {}) if isinstance(info, dict) else {}
        html = stats.get("global")
        if not isinstance(html, str):
            raise ValueError("getServerInfo omitted global analysis statistics")
        table = StatsTable()
        table.feed(html)
        if not table.rows or table.rows[0][:2] != ["Name", "Count"]:
            raise ValueError("unrecognized global analysis statistics format")
        count = 0
        for row in table.rows[1:]:
            if row and row[0] == "analyze_expr":
                if len(row) < 2:
                    raise ValueError("analyze_expr counter has no count")
                count += int(row[1])
        counts.append(count)
    # These counters are process-global, repeated in per-project info if present.
    if len(set(counts)) != 1:
        raise ValueError("projects returned inconsistent global trace counts")
    return counts[0]


def resolve_binary(value):
    path = Path(value).expanduser()
    if path.is_file():
        return str(path.resolve())
    found = shutil.which(value)
    if found:
        return found
    raise ValueError(f"Tinymist executable does not exist: {value}")


def read_exact(stream, length):
    chunks = bytearray()
    while len(chunks) < length:
        chunk = stream.read(length - len(chunks))
        if not chunk:
            raise EOFError("server closed stdout inside a message")
        chunks.extend(chunk)
    return bytes(chunks)


class LspClient:
    def __init__(self, binary, root, config, log_path):
        self.config = config
        self.legend = None
        self.messages = queue.Queue()
        self.next_id = 1
        self.log = log_path.open("wb")
        try:
            self.process = subprocess.Popen(
                [binary, "lsp"], cwd=root, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=self.log,
            )
        except BaseException:
            self.log.close()
            raise
        self.reader = threading.Thread(target=self.read_messages, daemon=True)
        self.reader.start()

    def read_messages(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        raise EOFError("server closed stdout")
                    if line in (b"\r\n", b"\n"):
                        break
                    key, value = line.decode("ascii").split(":", 1)
                    headers[key.lower()] = value.strip()
                length = int(headers["content-length"])
                if not 0 < length <= 64 * 1024 * 1024:
                    raise ValueError(f"invalid LSP message size: {length}")
                self.messages.put(json.loads(read_exact(self.process.stdout, length)))
        except Exception as error:
            self.messages.put(error)

    def send(self, message):
        payload = json.dumps({"jsonrpc": "2.0", **message}).encode("utf-8")
        self.process.stdin.write(
            f"Content-Length: {len(payload)}\r\n\r\n".encode("ascii") + payload
        )
        self.process.stdin.flush()

    def notify(self, method, params):
        self.send({"method": method, "params": params})

    def request(self, method, params, timeout):
        request_id = self.next_id
        self.next_id += 1
        started = time.monotonic()
        self.send({"id": request_id, "method": method, "params": params})
        deadline = started + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{method} exceeded {timeout:.1f}s")
            try:
                message = self.messages.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError(f"{method} exceeded {timeout:.1f}s") from error
            if isinstance(message, Exception):
                raise message
            if "method" in message and "id" in message:
                self.answer_server_request(message)
            elif message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message.get("result"), time.monotonic() - started

    def answer_server_request(self, message):
        method = message["method"]
        if method == "client/registerCapability":
            for registration in message.get("params", {}).get("registrations", []):
                if registration.get("method") == "textDocument/semanticTokens":
                    self.legend = registration.get("registerOptions", {}).get("legend")
        if method == "workspace/configuration":
            result = []
            for item in message.get("params", {}).get("items", []):
                section = item.get("section", "")
                if section in ("", "tinymist"):
                    result.append(self.config)
                elif section.startswith("tinymist."):
                    result.append(self.config.get(section[len("tinymist."):]))
                else:
                    result.append(None)
        elif method in (
            "client/registerCapability", "client/unregisterCapability",
            "window/workDoneProgress/create", "workspace/semanticTokens/refresh",
            "workspace/inlayHint/refresh", "workspace/codeLens/refresh",
            "workspace/diagnostic/refresh",
        ):
            result = None
        else:
            self.send({"id": message["id"], "error": {
                "code": -32601, "message": f"Unsupported client method: {method}",
            }})
            return
        self.send({"id": message["id"], "result": result})

    def close(self):
        # Always reap this process, including after request or initialization errors.
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        else:
            self.process.wait()
        self.reader.join(timeout=1)
        self.process.stdin.close()
        self.process.stdout.close()
        self.log.close()


def trial(binary, label, fixture, args):
    root = Path(__file__).resolve().parent
    entry = root / fixture
    config = {
        "rootPath": str(root), "typstExtraArgs": [fixture],
        "semanticTokens": "enable", "syntaxOnly": "disable",
        "exportPdf": "never", "colorTheme": "dark",
    }
    client = LspClient(binary, root, config, args.output_dir / f"{label}-{fixture}.log")
    result = {"binary": binary, "fixture": fixture, "label": label}
    try:
        initialized, _ = client.request("initialize", {
            "processId": None, "rootUri": root.as_uri(),
            "capabilities": {
                "workspace": {"configuration": True},
                "textDocument": {"semanticTokens": {
                    "dynamicRegistration": True,
                    "requests": {"full": {"delta": True}},
                    "tokenTypes": [], "tokenModifiers": [], "formats": ["relative"],
                }},
            },
            "initializationOptions": config,
            "workspaceFolders": [{"uri": root.as_uri(), "name": root.name}],
        }, args.timeout)
        result["server_info"] = initialized.get("serverInfo")
        provider = initialized.get("capabilities", {}).get("semanticTokensProvider")
        if isinstance(provider, dict):
            client.legend = provider.get("legend")
        client.notify("initialized", {})
        client.notify("textDocument/didOpen", {"textDocument": {
            "uri": entry.as_uri(), "languageId": "typst", "version": 1,
            "text": entry.read_text(encoding="utf-8"),
        }})
        tokens, elapsed = client.request("textDocument/semanticTokens/full", {
            "textDocument": {"uri": entry.as_uri()},
        }, args.timeout)
        data = tokens.get("data") if isinstance(tokens, dict) else None
        if not isinstance(data, list) or not data or len(data) % 5:
            raise ValueError("semanticTokens/full returned empty or malformed data")
        if any(type(item) is not int or item < 0 for item in data):
            raise ValueError("semantic token data contains invalid integers")
        if not isinstance(client.legend, dict) or not client.legend.get("tokenTypes"):
            raise ValueError("server did not advertise a semantic token legend")
        result["legend"] = client.legend
        stats, _ = client.request("workspace/executeCommand", {
            "command": "tinymist.getServerInfo", "arguments": [],
        }, args.timeout)
        result.update({
            "seconds": round(elapsed, 6), "token_count": len(data) // 5,
            "token_data": data,
            "token_sha256": hashlib.sha256(json.dumps(data).encode()).hexdigest(),
            "analyze_expr_count": trace_count(stats), "stats": stats,
        })
    except Exception as error:
        result["error"] = f"{type(error).__name__}: {error}"
    finally:
        client.close()
    (args.output_dir / f"{label}-{fixture}.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8",
    )
    print(json.dumps({k: v for k, v in result.items()
                      if k not in ("stats", "token_data", "legend")}), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", required=True, help="candidate Tinymist executable")
    parser.add_argument("--baseline", help="optional unmodified comparison executable")
    parser.add_argument("--require-baseline-trace", action="store_true",
                        help="require main.typ to trigger dynamic analysis on baseline")
    parser.add_argument("--timeout", type=float, default=25,
                        help="per-request deadline in seconds (default: 25)")
    parser.add_argument("--max-seconds", type=float,
                        help="optional semantic-token latency budget for the candidate")
    parser.add_argument("--output-dir", type=Path,
                        default=Path(__file__).resolve().parent / "results")
    args = parser.parse_args()
    if args.timeout <= 0 or (args.max_seconds is not None and args.max_seconds <= 0):
        parser.error("time budgets must be positive")
    if args.require_baseline_trace and not args.baseline:
        parser.error("--require-baseline-trace requires --baseline")
    try:
        binaries = [("candidate", resolve_binary(args.binary))]
        if args.baseline:
            binaries.insert(0, ("baseline", resolve_binary(args.baseline)))
    except ValueError as error:
        parser.error(str(error))
    args.output_dir.mkdir(parents=True, exist_ok=True)
    results = {}
    failures = []
    for label, binary in binaries:
        results[label] = {}
        for fixture in ("main.typ", "control.typ"):
            result = trial(binary, label, fixture, args)
            results[label][fixture] = result
            if "error" in result:
                failures.append(f"{label}/{fixture}: {result['error']}")
                continue
            if label == "candidate":
                if result["analyze_expr_count"] != 0:
                    failures.append(f"{label}/{fixture}: dynamic analyze_expr was invoked")
                if args.max_seconds and result["seconds"] > args.max_seconds:
                    failures.append(f"{label}/{fixture}: exceeded latency budget")
                baseline = results.get("baseline", {}).get(fixture)
                if baseline and "error" not in baseline:
                    for field in ("token_data", "legend"):
                        if baseline[field] != result[field]:
                            failures.append(f"{fixture}: candidate {field} differs from baseline")
            elif (args.require_baseline_trace and fixture == "main.typ"
                  and result["analyze_expr_count"] == 0):
                failures.append("baseline/main.typ: expected regression trace was not observed")
    summary = {"passed": not failures, "failures": failures, "runs": results}
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2) + "\n", encoding="utf-8",
    )
    print("PASS" if not failures else "FAIL", flush=True)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    return int(bool(failures))


if __name__ == "__main__":
    sys.exit(main())
