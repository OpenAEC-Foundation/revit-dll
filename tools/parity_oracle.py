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

# Differences explained and accepted, per field: why the file cannot give the API's value.
KNOWN = {
    # the two browser views have the placeholder name "???" in the file, the API gives a label
    "views.name": 2,
    # elevations whose type the API takes from elsewhere; not found in the file yet
    "views.type": 4,
    # internal dimension styles and similar: the API gives a label instead of the stored name
    "types.name": 48,
    # family-type parameters: the API derives the spec from the category
    "parameters.spec": 8,
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

    def field(self, domain, name, pairs):
        n = bad = 0
        first = []
        for key, want, got in pairs:
            n += 1
            if not close(want, got):
                bad += 1
                if len(first) < 3:
                    first.append((key, want, got))
        allowed = KNOWN.get(f"{domain}.{name}", 0)
        status = "ok" if bad == 0 else ("known" if bad <= allowed else "FAIL")
        self.fails += status == "FAIL"
        print(f"{domain}.{name}: {n - bad}/{n} {status}" + (f" {first}" if bad else ""), flush=True)


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

    pi_want = load(oracle, "meta.json")["project_info"]
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
