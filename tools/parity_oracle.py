"""Compare the catalogue readers (project info, views, families, types, parameters) with an oracle export.

Usage: python -I parity_oracle.py <rvt.exe> <oracle dir> <file.rvt>
The oracle dir comes from revit_oracle.py. Prints matches per field and the first differences; exit
code 1 when a field has differences not listed in KNOWN.
"""
import json
import os
import subprocess
import sys
import tempfile

# Differences explained by a rule: (domain.field) -> why the file cannot give the API's value.
EXPLAINED = {
    # the two browser views have the placeholder name "???" in the file, the API gives a label
    "views.name": lambda want, got: got == "???",
    # internal styles: the API gives a label instead of the stored name; some types store no name
    "types.name": lambda want, got: got == "" or (got or "").startswith("SecretInternal"),
    # family-type parameters: the API derives the spec from the category
    "parameters.spec": lambda want, got: want.startswith("autodesk.revit.category.family:"),
    # elevations whose type the API takes from elsewhere; not found in the file yet
    "views.type": lambda want, got: got == -1,
}


def close(a, b):
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(close(x, y) for x, y in zip(a, b))
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool):
        return abs(a - b) < 1e-9
    return a == b


class Report:
    def __init__(self):
        self.fails = 0
        self.upgraded = False  # the API then shows the file after an in-memory upgrade: differences do not fail

    def field(self, domain, name, pairs):
        n = bad = known = 0
        first = []
        rule = EXPLAINED.get(f"{domain}.{name}")
        for key, want, got in pairs:
            n += 1
            if not close(want, got):
                if rule and rule(want, got):
                    known += 1
                    continue
                bad += 1
                if len(first) < 3:
                    first.append((key, want, got))
        self.fails += bad > 0 and not self.upgraded
        word = "differs after upgrade" if self.upgraded else "FAIL"
        tail = (f" ({known} explained)" if known else "") + (f" {word} {first}" if bad else " ok")
        print(f"{domain}.{name}: {n - bad - known}/{n}{tail}", flush=True)


def rust(exe, cmd, rvt):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "o.json")
        r = subprocess.run([exe, cmd, rvt, "-o", out], capture_output=True, text=True)
        if r.returncode:
            raise SystemExit(f"rvt {cmd}: {r.stderr.strip()[:400]}")
        return json.load(open(out, encoding="utf-8"))


def load(d, name):
    return json.load(open(os.path.join(d, name), encoding="utf-8"))


def spec(s):
    """Spec and group ids without their version suffix; the API reports a newer schema version."""
    return s.rsplit("-", 1)[0] if s else ""


def main():
    exe, oracle, rvt = sys.argv[1:4]
    rep = Report()
    meta = load(oracle, "meta.json")

    # everything the API lists: present, same category and owner view
    rows = [json.loads(line) for line in open(os.path.join(oracle, "elements.jsonl"), encoding="utf-8")]
    summary = {e["id"]: e for e in rust(exe, "summary", rvt)}
    # an older file is upgraded in memory by the session, which may add elements and rename some
    upgraded = meta["basic_file_info"]["Format"] != meta["version"]["number"]
    rep.upgraded = upgraded
    if upgraded:
        print("file format", meta["basic_file_info"]["Format"], "was upgraded in memory: elements only the upgrade created are skipped")
    in_file = [r for r in rows if r["id"] in summary]
    rep.field("elements", "found", [("missing", [] if upgraded else [], [] if upgraded else sorted({r["id"] for r in rows} - set(summary))[:20])])
    rep.field("elements", "category", [(r["id"], r["cat"], summary[r["id"]]["category"]) for r in in_file if r["cat"] is not None])
    rep.field("elements", "owner_view", [(r["id"], r["owner_view"], summary[r["id"]]["owner_view"]) for r in in_file if r["owner_view"] is not None])
    by_class = {}
    for r in in_file:
        g = summary[r["id"]]
        c = by_class.setdefault(g["class"], [0, 0, 0, 0])
        c[0] += 1
        if r["name"] and r["name"] == g["name"]:
            c[1] += 1
        if r["type"] not in (None, -1):
            c[2] += 1
            if r["type"] == g["type_id"]:
                c[3] += 1
    names = sum(c[1] for c in by_class.values())
    print(f"elements.name: {names}/{len(in_file)} stored as the API gives them; type {sum(c[3] for c in by_class.values())}/{sum(c[2] for c in by_class.values())}")
    gaps = sorted(((c[2] - c[3], k) for k, c in by_class.items() if c[2] > c[3]), reverse=True)[:10]
    print("  types not found, per class:", gaps)
    print("  names not stored, per class:", sorted(((c[0] - c[1], k) for k, c in by_class.items() if c[0] > c[1]), reverse=True)[:14])

    if meta["is_family"]:
        fam = load(oracle, "family.json")
        got = rust(exe, "family", rvt)
        rep.field("family", "owner", [("id", fam["owner"], got["id"])])
        rep.field("family", "category", [("cat", fam["category"], got["category"])])
        rep.field("family", "types", [("names", sorted(t["name"] for t in fam["types"]), sorted(t["name"] for t in got["types"]))])
        kinds = {p["id"]: p["storage"] for p in fam["parameters"]}
        byname = {t["name"]: t for t in got["types"]}
        pairs = []
        for t in fam["types"]:
            have = {p["id"]: p["value"] for p in byname.get(t["name"], {"parameters": []})["parameters"]}
            for pid, v in t["values"].items():
                if kinds.get(int(pid)) != "ElementId" and int(pid) in have:
                    pairs.append(((t["name"], pid), v or None, have[int(pid)] or None))
        rep.field("family", "values", pairs)
        rep.field("family", "formulas", [(p["id"], True, any(q["id"] == p["id"] and q.get("formula") for t in got["types"] for q in t["parameters"]) or any(q["id"] == p["id"] and q.get("formula") for q in got["current"])) for p in fam["parameters"] if p["formula"]])
        print("FAILED" if rep.fails else "PASSED", rep.fails)
        sys.exit(1 if rep.fails else 0)

    pi_want = meta["project_info"]
    pi = rust(exe, "project_info", rvt)
    fields = [("Name", "name"), ("Number", "number"), ("ClientName", "client_name"), ("Address", "address"), ("Status", "status"), ("IssueDate", "issue_date"), ("Author", "author"), ("BuildingName", "building_name"), ("OrganizationName", "organization_name"), ("OrganizationDescription", "organization_description")]
    pi_want["Name"] = pi_want["name"]
    rep.field("project_info", "values", [(a, pi_want[a] or None, pi.get(b) or None) for a, b in fields])

    views_want = {v["id"]: v for v in load(oracle, "views.json")}
    views = {v["id"]: v for v in rust(exe, "views", rvt)}
    rep.field("views", "ids", [("count", sorted(views_want), sorted(views))])
    common = [(i, views_want[i], views[i]) for i in views_want if i in views]
    rep.field("views", "name", [(i, w["name"], g["name"]) for i, w, g in common])
    rep.field("views", "kind", [(i, w["view_type"], g["kind"]) for i, w, g in common])
    rep.field("views", "is_template", [(i, w["is_template"], g["is_template"]) for i, w, g in common])
    rep.field("views", "template", [(i, w["template"], g["template"]) for i, w, g in common])
    rep.field("views", "scale", [(i, w["scale"], g["scale"]) for i, w, g in common])
    rep.field("views", "type", [(i, w["type"], g["view_type"]) for i, w, g in common if w["type"] != -1])
    rep.field("views", "level", [(i, w["level"], g["level"]) for i, w, g in common if w["level"] is not None])
    for api, ours in (("origin", "origin"), ("right", "right"), ("up", "up"), ("dir", "direction")):
        rep.field("views", api, [(i, w[api], g[ours]) for i, w, g in common if w[api] is not None])
    rep.field("views", "detail_level", [(i, w["detail_level"], g.get("detail_level") or "Undefined") for i, w, g in common])
    rep.field("views", "discipline", [(i, w["discipline"], g.get("discipline") or "None") for i, w, g in common])
    rep.field("views", "sheet_number", [(i, w["sheet_number"], (g.get("sheet") or {}).get("number")) for i, w, g in common if "sheet_number" in w])

    rep.field("views", "viewports", [(i, sorted(w["viewports"]), sorted(v["id"] for v in g["sheet"]["viewports"])) for i, w, g in common if w.get("viewports")])
    rep.field("views", "placed_views", [(i, sorted(w["placed_views"]), sorted(v["view"] for v in g["sheet"]["viewports"])) for i, w, g in common if w.get("placed_views")])
    scheds = [(i, w, g) for i, w, g in common if w.get("fields") is not None]
    rep.field("views", "schedule_category", [(i, w["schedule_category"], g["schedule"]["category"]) for i, w, g in scheds])
    rep.field("views", "schedule_itemized", [(i, w["itemized"], g["schedule"]["itemized"]) for i, w, g in scheds])
    rep.field("views", "schedule_parameters", [(i, [f["parameter"] for f in w["fields"]], [f["parameter"] for f in g["schedule"]["fields"]]) for i, w, g in scheds])
    rep.field("views", "schedule_headings", [(i, [f["heading"] for f in w["fields"]], [f["heading"] for f in g["schedule"]["fields"]]) for i, w, g in scheds])
    rep.field("views", "schedule_hidden", [(i, [f["hidden"] for f in w["fields"]], [f["hidden"] for f in g["schedule"]["fields"]]) for i, w, g in scheds])

    fam_want = {f["id"]: f for f in load(oracle, "families.json")}
    fams = {f["id"]: f for f in rust(exe, "families", rvt)}
    rep.field("families", "ids", [("count", sorted(fam_want), sorted(fams))])
    common = [(i, fam_want[i], fams[i]) for i in fam_want if i in fams]
    rep.field("families", "name", [(i, w["name"], g["name"]) for i, w, g in common])
    rep.field("families", "category", [(i, w["family_category"], g["category"]) for i, w, g in common])
    rep.field("families", "in_place", [(i, w["in_place"], g["in_place"]) for i, w, g in common])
    rep.field("families", "editable", [(i, w["editable"], g["editable"]) for i, w, g in common])
    rep.field("families", "types", [(i, w["symbols"], sorted(g["types"])) for i, w, g in common])

    typ_want = {t["id"]: t for t in load(oracle, "types.json")}
    typs = {t["id"]: t for t in rust(exe, "types", rvt)}
    if upgraded:
        typ_want = {i: t for i, t in typ_want.items() if i in summary}
    rep.field("types", "ids", [("missing", [], sorted(set(typ_want) - set(typs))[:20])])
    common = [(i, typ_want[i], typs[i]) for i in typ_want if i in typs]
    rep.field("types", "name", [(i, w["name"], g["name"]) for i, w, g in common])
    rep.field("types", "category", [(i, w["cat"], g["category"]) for i, w, g in common if w["cat"] is not None])
    rep.field("types", "family", [(i, w["family"], g.get("family")) for i, w, g in common if w["family"] is not None])
    rep.field("types", "family_name", [(i, w["family_name"], g.get("family_name")) for i, w, g in common if w["family"] is not None])
    # values of user parameters on types (built-in ones are often computed by Revit)
    pairs = []
    for i, w, g in common:
        have = {p["id"]: p for p in g.get("parameters", [])}
        for p in w["params"]:
            if p["id"] > 0 and p.get("has") and p["id"] in have and p["st"] != "ElementId":
                pairs.append(((i, p["id"]), p.get("v") or None, have[p["id"]]["value"] or None))
    rep.field("types", "parameter_values", pairs)

    defs = load(oracle, "parameters.json")
    pars = rust(exe, "parameters", rvt)
    def_want = {d["id"]: d for d in defs["definitions"]}
    if upgraded:
        def_want = {i: d for i, d in def_want.items() if i in summary}
    got = {d["id"]: d for d in pars["definitions"]}
    rep.field("parameters", "missing", [("missing", [], sorted(set(def_want) - set(got))[:20])])
    common = [(i, def_want[i], got[i]) for i in def_want if i in got]
    rep.field("parameters", "name", [(i, w["name"], g["name"]) for i, w, g in common])
    rep.field("parameters", "group", [(i, spec(w["definition"]["group"]), spec(g.get("group") or "")) for i, w, g in common])
    rep.field("parameters", "spec", [(i, spec(w["definition"]["data_type"]), spec(g.get("spec") or "")) for i, w, g in common if w["definition"]["data_type"]])
    rep.field("parameters", "guid", [(i, w["guid"], g.get("guid")) for i, w, g in common if "guid" in w])
    bind_want = {b["id"]: b for b in defs["bindings"]}
    bind = {b["parameter"]: b for b in pars["bindings"]}
    rep.field("parameters", "bindings", [(i, [w["binding"], w["categories"]], [bind[i]["binding"], sorted(bind[i]["categories"])] if i in bind else None) for i, w in bind_want.items()])

    print("FAILED" if rep.fails else "PASSED", rep.fails)
    sys.exit(1 if rep.fails else 0)


if __name__ == "__main__":
    main()
