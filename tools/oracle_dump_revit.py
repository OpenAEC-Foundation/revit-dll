# -*- coding: utf-8 -*-
"""Read-only oracle export; runs inside Revit (IronPython 2.7, pyRevit route), sent by revit_oracle.py.

Opens a file without UI, writes public-API output (one file per domain) to OUTPUT_DIR and closes
the document without saving. Nothing here changes a model; no transaction is started.
INPUT_PATH and OUTPUT_DIR are prepended by the client; `uiapp` and `DB` come from the route.
"""
import io
import os
import time
import traceback

app = uiapp.Application
started = time.time()


def eid(x):
    return None if x is None else int(x.Value)


def attempt(f, default=None):
    try:
        return f()
    except Exception:
        return default


def elem_name(e):
    try:
        return e.Name
    except Exception:
        return attempt(lambda: DB.Element.Name.GetValue(e))


def xyz(p):
    return None if p is None else [p.X, p.Y, p.Z]


def transform(t):
    return None if t is None else {"O": xyz(t.Origin), "X": xyz(t.BasisX), "Y": xyz(t.BasisY), "Z": xyz(t.BasisZ)}


def box(b):
    return None if b is None else {"min": xyz(b.Min), "max": xyz(b.Max), "T": transform(b.Transform)}


def param_row(p):
    d = p.Definition
    st = str(p.StorageType)
    r = {"n": d.Name, "st": st, "ro": p.IsReadOnly, "has": p.HasValue, "id": eid(p.Id)}
    if p.IsShared:
        r["guid"] = str(p.GUID)
    bip = attempt(lambda: d.BuiltInParameter)
    if bip is not None and str(bip) != "INVALID":
        r["bip"] = str(bip)
    if p.HasValue:
        if st == "Double":
            r["v"] = p.AsDouble()
        elif st == "Integer":
            r["v"] = p.AsInteger()
        elif st == "String":
            r["v"] = p.AsString()
        elif st == "ElementId":
            r["v"] = eid(p.AsElementId())
        s = attempt(p.AsValueString)
        if s is not None:
            r["s"] = s
    return r


def params_of(e):
    out = []
    for p in e.Parameters:
        r = attempt(lambda: param_row(p))
        if r is not None:
            out.append(r)
    out.sort(key=lambda r: (r["n"], r["id"]))
    return out


def base_row(e):
    cat = attempt(lambda: e.Category)
    tid = attempt(lambda: e.GetTypeId())
    return {
        "id": eid(e.Id),
        "uid": e.UniqueId,
        "class": e.GetType().Name,
        "cat": eid(cat.Id) if cat is not None else None,
        "cat_name": cat.Name if cat is not None else None,
        "name": elem_name(e),
        "owner_view": eid(attempt(lambda: e.OwnerViewId)),
        "type": eid(tid),
    }


def esc(s):
    out = []
    for c in s:
        n = ord(c)
        if c == '"' or c == "\\":
            out.append("\\" + c)
        elif n < 32 or n > 126:
            out.append("\\u%04x" % n)
        else:
            out.append(c)
    return '"' + "".join(out) + '"'


def enc(o):
    """JSON text; IronPython's json cannot encode .NET strings with non-ASCII characters."""
    if o is None:
        return "null"
    if o is True:
        return "true"
    if o is False:
        return "false"
    if isinstance(o, float):
        return "NaN" if o != o else repr(o)
    if isinstance(o, (int, long)):
        return str(o)
    if isinstance(o, basestring):
        return esc(o)
    if isinstance(o, dict):
        return "{" + ",".join(esc(k) + ":" + enc(o[k]) for k in sorted(o.keys())) + "}"
    try:
        return "[" + ",".join(enc(x) for x in o) + "]"
    except TypeError:
        return esc(str(o))


def write_json(name, obj):
    with io.open(os.path.join(OUTPUT_DIR, name), "w", encoding="utf-8") as f:
        f.write(unicode(enc(obj)))


def view_row(v):
    r = base_row(v)
    r["view_type"] = str(attempt(lambda: v.ViewType))
    r["is_template"] = attempt(lambda: v.IsTemplate)
    r["template"] = eid(attempt(lambda: v.ViewTemplateId))
    r["scale"] = attempt(lambda: v.Scale)
    r["detail_level"] = str(attempt(lambda: v.DetailLevel))
    r["discipline"] = str(attempt(lambda: v.Discipline))
    r["display_style"] = str(attempt(lambda: v.DisplayStyle))
    r["crop_active"] = attempt(lambda: v.CropBoxActive)
    r["crop_visible"] = attempt(lambda: v.CropBoxVisible)
    r["crop"] = attempt(lambda: box(v.CropBox))
    r["origin"] = attempt(lambda: xyz(v.Origin))
    r["right"] = attempt(lambda: xyz(v.RightDirection))
    r["up"] = attempt(lambda: xyz(v.UpDirection))
    r["dir"] = attempt(lambda: xyz(v.ViewDirection))
    r["level"] = eid(attempt(lambda: v.GenLevel.Id))
    r["primary"] = eid(attempt(lambda: v.GetPrimaryViewId()))
    r["is_callout"] = attempt(lambda: v.IsCallout)
    r["title"] = attempt(lambda: v.Title)
    if isinstance(v, DB.ViewPlan):
        rng = attempt(lambda: v.GetViewRange())
        if rng is not None:
            r["view_range"] = {}
            for k in ("TopClipPlane", "CutPlane", "BottomClipPlane", "ViewDepthPlane"):
                pl = getattr(DB.PlanViewPlane, k)
                r["view_range"][k] = {"level": eid(attempt(lambda: rng.GetLevelId(pl))), "offset": attempt(lambda: rng.GetOffset(pl))}
    if isinstance(v, DB.View3D):
        r["perspective"] = attempt(lambda: v.IsPerspective)
        r["section_box_active"] = attempt(lambda: v.IsSectionBoxActive)
        r["section_box"] = attempt(lambda: box(v.GetSectionBox()))
        o = attempt(lambda: v.GetOrientation())
        if o is not None:
            r["orientation"] = {"eye": xyz(o.EyePosition), "up": xyz(o.UpDirection), "forward": xyz(o.ForwardDirection)}
    if isinstance(v, DB.ViewSheet):
        r["sheet_number"] = attempt(lambda: v.SheetNumber)
        r["placeholder"] = attempt(lambda: v.IsPlaceholder)
        r["viewports"] = [eid(x) for x in attempt(lambda: list(v.GetAllViewports()), [])]
        r["placed_views"] = [eid(x) for x in attempt(lambda: list(v.GetAllPlacedViews()), [])]
    if isinstance(v, DB.ViewSchedule):
        d = attempt(lambda: v.Definition)
        if d is not None:
            r["schedule_category"] = eid(attempt(lambda: d.CategoryId))
            r["itemized"] = attempt(lambda: d.IsItemized)
            fields = []
            for i in range(attempt(lambda: d.GetFieldCount(), 0)):
                f = attempt(lambda: d.GetField(i))
                if f is not None:
                    fields.append(
                        {
                            "name": attempt(f.GetName),
                            "heading": attempt(lambda: f.ColumnHeading),
                            "hidden": attempt(lambda: f.IsHidden),
                            "parameter": eid(attempt(lambda: f.ParameterId)),
                        }
                    )
            r["fields"] = fields
    r["params"] = params_of(v)
    return r


def dump(doc, info):
    out = {"meta": {}}
    meta = out["meta"]
    meta["title"] = doc.Title
    meta["is_family"] = doc.IsFamilyDocument
    meta["is_workshared"] = doc.IsWorkshared
    meta["is_detached"] = doc.IsDetached
    meta["version"] = {"number": app.VersionNumber, "build": app.VersionBuild}
    meta["basic_file_info"] = info
    meta["creation_guid"] = str(attempt(lambda: doc.CreationGUID))
    pi = doc.ProjectInformation
    if pi is not None:
        row = base_row(pi)
        for k in ("Address", "Author", "BuildingName", "ClientName", "IssueDate", "Number", "OrganizationDescription", "OrganizationName", "Status"):
            row[k] = attempt(lambda: getattr(pi, k))
        row["params"] = params_of(pi)
        meta["project_info"] = row
    u = attempt(doc.GetUnits)
    if u is not None:
        meta["units"] = {}
        for k in ("Length", "Area", "Volume", "Angle", "Slope", "Currency", "Number"):
            spec = attempt(lambda: getattr(DB.SpecTypeId, k))
            if spec is not None:
                fo = attempt(lambda: u.GetFormatOptions(spec))
                if fo is not None:
                    meta["units"][k] = {"unit": attempt(lambda: fo.GetUnitTypeId().TypeId), "accuracy": attempt(lambda: fo.Accuracy), "symbol": attempt(lambda: fo.GetSymbolTypeId().TypeId)}
    write_json("meta.json", meta)

    # every element reachable through the public collectors
    all_elems = []
    for flt in (False, True):
        col = DB.FilteredElementCollector(doc)
        col = col.WhereElementIsElementType() if flt else col.WhereElementIsNotElementType()
        all_elems.extend(list(col.ToElements()))
    rows = {}
    with io.open(os.path.join(OUTPUT_DIR, "elements.jsonl"), "w", encoding="utf-8") as f:
        for e in all_elems:
            r = attempt(lambda: base_row(e))
            if r is None:
                continue
            bb = attempt(lambda: e.get_BoundingBox(None))
            if bb is not None:
                r["bbox"] = [bb.Min.X, bb.Min.Y, bb.Min.Z, bb.Max.X, bb.Max.Y, bb.Max.Z]
            rows[r["id"]] = r
            f.write(unicode(enc(r)) + u"\n")
    out["elements"] = len(rows)

    write_json("views.json", [view_row(v) for v in DB.FilteredElementCollector(doc).OfClass(DB.View).ToElements()])

    fams = []
    for fam in DB.FilteredElementCollector(doc).OfClass(DB.Family).ToElements():
        r = base_row(fam)
        r["family_category"] = eid(attempt(lambda: fam.FamilyCategoryId))
        r["in_place"] = attempt(lambda: fam.IsInPlace)
        r["editable"] = attempt(lambda: fam.IsEditable)
        r["placement"] = str(attempt(lambda: fam.FamilyPlacementType))
        r["symbols"] = sorted(eid(x) for x in attempt(lambda: list(fam.GetFamilySymbolIds()), []))
        r["params"] = params_of(fam)
        fams.append(r)
    write_json("families.json", fams)

    types = []
    for t in DB.FilteredElementCollector(doc).WhereElementIsElementType().ToElements():
        r = base_row(t)
        r["family_name"] = attempt(lambda: t.FamilyName)
        fam = attempt(lambda: t.Family)
        r["family"] = eid(fam.Id) if fam is not None else None
        r["params"] = params_of(t)
        types.append(r)
    write_json("types.json", types)

    defs = []
    for pe in DB.FilteredElementCollector(doc).OfClass(DB.ParameterElement).ToElements():
        r = base_row(pe)
        d = attempt(lambda: pe.GetDefinition())
        if d is not None:
            r["definition"] = {
                "name": attempt(lambda: d.Name),
                "data_type": attempt(lambda: d.GetDataType().TypeId),
                "group": attempt(lambda: d.GetGroupTypeId().TypeId),
                "visible": attempt(lambda: d.Visible),
                "variation_across_groups": attempt(lambda: d.VariesAcrossGroups),
            }
        if isinstance(pe, DB.SharedParameterElement):
            r["guid"] = str(attempt(lambda: pe.GuidValue))
        if isinstance(pe, DB.GlobalParameter):
            r["reporting"] = attempt(lambda: pe.IsReporting)
        defs.append(r)
    bindings = []
    it = doc.ParameterBindings.ForwardIterator()
    while it.MoveNext():
        d, b = it.Key, it.Current
        bindings.append(
            {
                "name": attempt(lambda: d.Name),
                "id": eid(attempt(lambda: d.Id)),
                "binding": b.GetType().Name,
                "categories": sorted(eid(c.Id) for c in attempt(lambda: list(b.Categories), [])),
            }
        )
    write_json("parameters.json", {"definitions": defs, "bindings": bindings})

    cats = []

    def walk(c, parent):
        cats.append(
            {
                "id": eid(c.Id),
                "name": c.Name,
                "parent": parent,
                "type": str(attempt(lambda: c.CategoryType)),
                "builtin": str(attempt(lambda: c.BuiltInCategory)),
                "material": eid(attempt(lambda: c.Material.Id)),
                "cuttable": attempt(lambda: c.IsCuttable),
                "line_color": attempt(lambda: [int(c.LineColor.Red), int(c.LineColor.Green), int(c.LineColor.Blue)]),
            }
        )
        for s in attempt(lambda: list(c.SubCategories), []):
            walk(s, eid(c.Id))

    for c in doc.Settings.Categories:
        walk(c, None)
    write_json("categories.json", cats)

    if doc.IsWorkshared:
        write_json(
            "worksets.json",
            [
                {"id": w.Id.IntegerValue, "name": w.Name, "kind": str(w.Kind), "open": w.IsOpen, "owner": w.Owner, "default": w.IsDefaultWorkset}
                for w in DB.FilteredWorksetCollector(doc).ToWorksets()
            ],
        )

    with io.open(os.path.join(OUTPUT_DIR, "element_params.jsonl"), "w", encoding="utf-8") as f:
        for e in all_elems:
            r = attempt(lambda: {"id": eid(e.Id), "params": params_of(e)})
            if r is not None:
                f.write(unicode(enc(r)) + u"\n")
    return out


if not os.path.isdir(OUTPUT_DIR):
    os.makedirs(OUTPUT_DIR)

info = DB.BasicFileInfo.Extract(INPUT_PATH)
info_row = {}
for k in ("Format", "IsWorkshared", "IsCentral", "IsLocal", "IsCreatedLocal", "Username", "CentralPath", "LanguageWhenSaved", "IsLightweight", "AllLocalChangesSavedToCentral"):
    info_row[k] = attempt(lambda: getattr(info, k))
info_row["LanguageWhenSaved"] = str(info_row["LanguageWhenSaved"])

opts = DB.OpenOptions()
opts.Audit = False
if info.IsWorkshared:
    opts.DetachFromCentralOption = DB.DetachFromCentralOption.DetachAndPreserveWorksets
    opts.SetOpenWorksetsConfiguration(DB.WorksetConfiguration(DB.WorksetConfigurationOption.OpenAllWorksets))

opened = None
try:
    opened = app.OpenDocumentFile(DB.ModelPathUtils.ConvertUserVisiblePathToModelPath(INPUT_PATH), opts)
    summary = dump(opened, info_row)
    summary["seconds"] = time.time() - started
    print(enc(summary))
except Exception:
    print(enc({"error": traceback.format_exc()}))
finally:
    if opened is not None:
        opened.Close(False)
