# rvt-reader

Native library (DLL) that reads Autodesk Revit `.rvt` files without Revit, for use by
[Open BIM Studio](https://github.com/OpenAEC-Foundation/open-bim-studio) and other tools.

**Status:** private, pre-alpha. Do not make this repository public without a decision by the
maintainers (see "Legal" below).

## Scope

- Input: `.rvt` project files (Revit 2014–2026).
- Output: a neutral model (levels, walls, floors, ceilings, roofs, columns, beams, family
  instances, MEP elements, rooms, views, sheets, annotations) exposed through a C ABI, plus a
  JSON export.
- The importer that maps this model to native Open BIM Studio objects lives in Open BIM Studio
  (DynLex), not here.

## Clean-room rules

The reader is built only from black-box analysis of files and of Revit's public API output.

- Never open, disassemble, or decompile Autodesk, ODA, or other third-party binaries; no memory
  dumps.
- Never consult third-party Revit parsers, SDK headers, or leaked or licensed format
  documentation.
- Contributors who have seen such material do not write reader code.
- Revit is used only as a read-only oracle: nothing is saved, documents are closed without saving.

## Test data

Never commit `.rvt`, `.rfa`, `.rte`, or `.rft` files, Autodesk sample models, or exports derived
from them. Test fixtures must come from our own models. `.gitignore` enforces the file types.

## Legal

Revit is a registered trademark of Autodesk, Inc. This project is not affiliated with or endorsed
by Autodesk. Before any public release: legal review, settle the licence of the Revit installation
used as oracle, and a patent freedom-to-operate check.
