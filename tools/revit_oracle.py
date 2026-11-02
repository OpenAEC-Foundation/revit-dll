"""Export a file's public-API facts from a running Revit session (read-only oracle, see README).

Usage: python -I revit_oracle.py --port <port> --pid <pid> <file.rvt> <output dir>
       python -I revit_oracle.py --port <port> --pid <pid> --code <script.py> [file output]

The session is addressed explicitly and checked before anything is sent: it must report the given
process id and have no open documents, so a session someone works in is never touched. The output
is derived from a model and belongs in `exports/` (ignored by git).

The session's generic code route runs on the web server's thread, where opening a document fails
or takes Revit down. So the first call only registers a route of our own (pure Python, no API
calls); pyRevit runs a route that asks for `uiapp` on Revit's main thread through an external
event, and the route checks that before it runs anything.
"""
import argparse
import json
import os
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))

BOOTSTRAP = r'''
import json
import sys
import traceback
from StringIO import StringIO

import clr
from pyrevit import routes

clr.AddReference("PresentationFramework")
clr.AddReference("WindowsBase")
from System.Windows import Application as _WpfApp

_api = routes.API("rvt_oracle")


@_api.route("/run/", methods=["POST"])
def _run(uiapp, request):
    body = json.loads(request.data) if isinstance(request.data, (str, unicode)) else request.data
    if not _WpfApp.Current.Dispatcher.CheckAccess():
        return routes.make_response(data={"error": "not on the Revit main thread"}, status=500)
    from pyrevit import DB
    buf = StringIO()
    old = sys.stdout
    sys.stdout = buf
    try:
        exec(body["code"], {"uiapp": uiapp, "DB": DB, "__name__": "oracle"})
        return routes.make_response(data={"output": buf.getvalue()})
    except Exception:
        return routes.make_response(data={"error": traceback.format_exc(), "output": buf.getvalue()}, status=500)
    finally:
        sys.stdout = old


print("registered")
'''


def call(port, path, body=None, timeout=60):
    data = None if body is None else json.dumps(body).encode("utf-8")
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", "replace")
        try:
            d = json.loads(text)
            text = f"{d.get('error')}\n{d.get('traceback', '')}\n{d.get('output', '')}"
        except ValueError:
            pass
        raise SystemExit(f"{path}: HTTP {e.code}: {text[:6000]}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--timeout", type=int, default=3600)
    ap.add_argument("--code", help="run this script instead of the export")
    ap.add_argument("--opens-nothing", action="store_true", help="the script touches no document: run it next to open documents")
    ap.add_argument("file", nargs="?")
    ap.add_argument("out", nargs="?")
    a = ap.parse_args()

    health = call(a.port, "/codex_bridge/health/")
    if health.get("pid") != a.pid:
        raise SystemExit(f"port {a.port} belongs to process {health.get('pid')}, not {a.pid}")
    docs = call(a.port, "/codex_bridge/documents/").get("documents")
    if docs and not (a.code and a.opens_nothing):
        raise SystemExit(f"session has open documents, refusing: {[d.get('title') for d in docs]}")

    call(a.port, "/revit_mcp/execute_code/", {"code": BOOTSTRAP, "description": "register oracle route"})
    with open(a.code or os.path.join(HERE, "oracle_dump_revit.py"), encoding="utf-8") as f:
        script = f.read()
    head = ""
    if a.file:
        head += f"INPUT_PATH = u{json.dumps(os.path.abspath(a.file))}\n"
    if a.out:
        head += f"OUTPUT_DIR = u{json.dumps(os.path.abspath(a.out))}\n"
    r = call(a.port, "/rvt_oracle/run/", {"code": head + script}, timeout=a.timeout)
    print(r.get("output", r))


if __name__ == "__main__":
    main()
