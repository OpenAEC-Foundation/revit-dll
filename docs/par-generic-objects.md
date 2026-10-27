# Spoor C: elk object opvraagbaar via het schema (branch `par/generic-objects`)

Datum: 2026-10-27. Basis: `open-research` op b13d5bc (stap 2308).

## Wat er is

Elk element heeft in de partities drie records (stap 2301): kanaal 101 de elementkop
(`ElementHeader`), kanaal 102 de objectgraaf van het element zelf (wortelklasse = de klasse van
het element: `SWall`, `Level`, `MaterialElem`, `FamilyInstance`, …) en kanaal 103 de saved
graphics. De generieke laag decodeert kanaal 102 (en de kop uit 101) met het klassenschema van
het bestand en levert een serialiseerbaar object met klassenaam, ouderketen en getypeerde,
benoemde velden. Er zit geen enkele byte-offset in: alles komt uit het schema.

### API (rvt-read)

```rust
let doc = rvt_read::Document::open("model.rvt")?;
doc.object(198694)                 // Option<ObjectView>: één element
doc.objects_of_class("VWall")      // Vec<ObjectView>, inclusief subklassen (SWall, ArcWall)
doc.object_ids_of_class("Element") // ids, oplopend
doc.classes()                      // BTreeMap<klasse, aantal>
doc.object_coverage()              // dekking per kanaal (zie onder)
doc.object_store()                 // ruwe toegang: schema, bodies, ES-catalogus, decode()
```

De store wordt bij het eerste gebruik gelezen (`OnceLock`). Bij `Document::open` wordt het
bestand daarvoor opnieuw geopend; bij `Document::from_bytes` blijft de `Cfb` (de bestandsbytes)
daarvoor in het document bewaard.

Typen in `rvt-model::object`: `ObjectView`, `NestedObject`, `FieldView`, `FieldValue`,
`Coverage`/`ChannelCoverage`/`FailingClass` (serde Serialize + Deserialize). Hulpjes:
`ObjectView::get`, `is_a`, `deref` (pointer → doelobject), `path(&["m_pSurface", "m_origin"])`;
`FieldValue::as_f64/as_i64/as_str/element/as_object/as_list`.

### JSON-vorm (stabiele veldnamen)

- object: `id`, `class`, `parents` (dichtstbijzijnde eerst), `category` (uit de kop, anders de
  elementtabel), `complete`, `error` (alleen bij een fout), `fields`, `objects`
  (pointerdoelen in decodeervolgorde), `header` (de `ElementHeader`, pointers ingevuld);
- veld: `name`, `type`, `owner` (de klasse die het veld declareert), `value`;
- `type`: `bool u8 i16 i32 u32 f32 f64 string guid class i64`, de klassenaam van een inline
  object, `ptr` of `ext_ptr`; `T[n]` voor een vaste array, `vec<T>` voor een vector;
- waarden: JSON-scalars; `ElementId` → `{"element": id}`; pointer → `{"ref": n}` (index in
  `objects`), null-pointer → `null`; klasse-referentie (basis 10) → `{"class_ref": "Naam"}`;
  inline object → `{"class": …, "fields": [...]}`; GUID → tekst `xxxxxxxx-xxxx-…`
  (eerste drie groepen little-endian);
- extensible storage: een `ESEntity::m_blob` wijst naar een object van klasse `ESBlob` met
  `es_schema`, `es_guid` en daarna de ES-velden op naam (type `?`, eigenaar leeg: ze staan niet
  in Formats/Latest).

Voorbeeld (racbasic 2026, wand 198694, ingekort; `rvt object <file> 198694 --pretty`):

```json
{
  "id": 198694, "class": "SWall", "parents": ["VWall", "HostObj", "Element"],
  "category": -2000011, "complete": true,
  "fields": [
    {"name": "m_pParamValueSetDouble", "type": "ptr", "owner": "Element", "value": {"ref": 0}},
    {"name": "m_id", "type": "ElementId", "owner": "Element", "value": {"element": 198694}},
    {"name": "m_assocLevelId", "type": "ElementId", "owner": "Element", "value": {"element": 245423}},
    {"name": "m_pCurveDriver", "type": "ptr", "owner": "VWall", "value": {"ref": 5}},
    {"name": "m_pRefFaces", "type": "vec<ptr>", "owner": "VWall", "value": [{"ref": 6}, {"ref": 7}]},
    {"name": "m_WallAttributesId", "type": "ElementId", "owner": "VWall", "value": {"element": 428955}},
    {"name": "m_upToLevelId", "type": "ElementId", "owner": "VWall", "value": {"element": 196629}},
    {"name": "m_curtainAuxData", "type": "CurtainAuxData", "owner": "VWall",
     "value": {"class": "CurtainAuxData", "fields": [
       {"name": "m_mullionTdFlags", "type": "i32", "owner": "CurtainAuxData", "value": 128}]}},
    {"name": "m_wallStructuralUsage", "type": "i32", "owner": "VWall", "value": 1},
    {"name": "m_isFlipped", "type": "bool", "owner": "VWall", "value": false}
  ],
  "objects": [
    {"class": "ParamValueSetDouble", "fields": [{"name": "m_paramSet", "type": "vec<ParamValueDouble>", "owner": "ParamValueSetDouble",
      "value": [{"class": "ParamValueDouble", "fields": [
        {"name": "m_value", "type": "f64", "owner": "ParamValueDouble", "value": -0.984251968503937},
        {"name": "m_paramId", "type": "ElementId", "owner": "ParamValueDouble", "value": {"element": -1001108}}]}]}]},
    "…",
    {"class": "GLine", "fields": [
      {"name": "m_endParams", "type": "f64[2]", "owner": "GLine", "value": [-0.331365, 20.0164]},
      {"name": "m_origin", "type": "f64[3]", "owner": "GLine", "value": [43.1855, 9.18112, 9.84252]},
      {"name": "m_dirVec", "type": "f64[3]", "owner": "GLine", "value": [-1.88e-15, -1.0, -0.0]}]}
  ],
  "header": {"class": "ElementHeader", "fields": ["… m_categroryId, m_familyId, m_ownerViewId, m_pBBox …"]}
}
```

### CLI

```
rvt object <file> <id>                       één element (JSON)
rvt classes <file>                           {"classes": {klasse: aantal}}
rvt objects <file> --class <naam> [--limit n] {"class", "total", "objects": [...]}
rvt coverage <file>                          dekking per kanaal
```
`--pretty` springt in; `-o uit.json` schrijft een bestand.

### C ABI (rvt-ffi, `rvt.dll`)

`rvt_open(path) → RvtDocument*` (NULL bij fout), `rvt_query(doc, json) → char*`,
`rvt_free(char*)`, `rvt_close(doc)`. Verzoeken:
`{"query": "info"}`, `{"query": "object", "id": 123}`, `{"query": "classes"}`,
`{"query": "objects", "class": "VWall", "limit": 10, "offset": 0}`. Antwoord
`{"ok": true, "result": …}` of `{"ok": false, "error": "…"}`. Panics worden opgevangen
(`catch_unwind`). De oude stub was leeg (alleen een doc-commentaar).

## Decoderfouten hersteld (rvt-core/src/graph.rs)

1. **Modifier 0x54** (een vector van pointers van soort 4: `VarSketchObj::m_params`,
   `VarFunction::m_args`) werd als één pointer gelezen: het aantal werd als token gelezen en
   alles daarna verschoof. Bytes (racbasic 2026 VarSketch 176803 @1658): `04 00 00 00` en dan
   4 × (u32 token + u16 klasse 0x1142 = VarParam). Nu `0x50..=0x54` = vector. Effect: alle
   `VarSketch`-records (racbasic 2026: 45 van 46 faalden), 207 records in rvt100+racbasic, 1154
   in de rme/rst/snowdon-set.
2. **Extensible storage** (`ESEntity::m_blob`): na de `i32` (0 = afwezig, −1 = aanwezig) staat
   de 16-byte GUID van het ES-schema in plaats van een klassetag; het doel wordt in de
   wachtrij gedecodeerd als de velden van dat ES-schema (op `m_entryIndex`, zonder kop).
   De ES-schema's zijn `ESSchema`-objecten in de documentgraaf: `Global/Latest` (na de
   page-strip 8 B prefix + één gzip-lid) decodeert met dezelfde decoder als een
   `ADocument`-graaf met 4 bytes over (rst 2026: 1 586 246 B). Getoetst op rst 2026
   DataStorage 1382860 (schema 5d9588ee… "DaylightingAnalysisInfo": `AnalysisId` TCHAR +
   `ResultsInvalid` int = 8 nulbytes), rvt100 003 ProjectInfo 76675 en 087 fitting 8145.
   Nieuwe ingangen, `Graph::decode` zelf is ongewijzigd:
   `Graph::decode_es(schema, body, &EsCatalog)` en `Graph::decode_prefix(schema, body)`;
   `rvt_read::objects::es_catalog(cfb, schema)` bouwt de catalogus. Effect: 5471 records
   (vooral `FamilyInstance` met MEP-gegevens in rme, `RebarShape`, `DataStorage`,
   `ProjectInfo`).

Regressietoets: `examples/graph_digest.rs` schrijft per record (kanaal, id) een hash van de
gedecodeerde graaf; `tools/digest_compare.py` vergelijkt oud en nieuw. Uitkomst over 117
bestanden (rvt100, racbasic 2020/2022–2026, rme/rst 2023–2026, snowdon hvac 2025, structural
2026, electrical 2024): **1 576 493 eerder geslaagde records gelijk, 0 gewijzigd, 0 nu fout**;
6832 eerder falende records slagen nu. (Een lege vector en een null-pointer hashen gelijk, omdat
een leeg 0x54-veld vroeger als null-pointer werd gelezen; verder byte-identiek in Debug-vorm.)
Unit-tests met een synthetisch schema: `rvt-core` graph::tests (0x54, ES, prefix),
`rvt-read` objects::tests (view, typen, JSON-vorm), `rvt-ffi` tests (verzoeken, C ABI).

Bronnen: rvt-rs (github DrunkOnJava/rvt-rs, Apache-2.0), `src/native_parameters.rs`: functionele
beschrijving van `entity_pointer`/`es_value` (via WebFetch gelezen, geen code overgenomen, niets
gedownload of uitgevoerd); het rvt-rs-bestand zegt niet waar de catalogus vandaan komt, dat
(Global/Latest) is eigen byte-observatie. Geen Autodesk-binaries, geen Revit gebruikt.

## Dekking (records volledig gedecodeerd: hele body verbruikt, geen fout)

Per kanaal is er 1 leeg record per bestand (id −1, body 0 B); dat telt niet mee.

| Versie | Bestanden | Records per kanaal | 101 | 102 | 103 |
|---|---|---|---|---|---|
| 2026 | 104 (rvt100 ×100, racbasic, rme, rst, snowdon structural) | 329 261 | 100 % | 100 % | 100 % |
| 2025 | 4 (racbasic, rme, rst, snowdon hvac) | 62 978 | 100 % | 100 % | 100 % |
| 2024 | 4 (racbasic, rme, rst, snowdon electrical) | 69 640 | 100 % | 100 % | 100 % |
| 2023 (32-bit) | 3 (racbasic, rme, rst) | 49 848 | 100 % | 100 % | 100 % |
| 2022 / 2020 | 1 + 1 (racbasic) | 8 156 / 8 009 | 100 % | 100 % | 100 % |
| steekproef 2014, 2015, 2017, 2019, 2021 | rst basic (+ rme 2019) | 13 811 – 36 581 | 100 % | 100 % | 100 % |

Vóór dit spoor, kanaal 102: racbasic 2026 8356/8401 (alle VarSketch fout); rvt100 209 fouten;
rme 2023–2026 ±1170 per bestand (MEP-families met ES-data), rst ±7, snowdon hvac 2025 754.
Top-10 falende klassen na herstel: leeg (geen enkele fout in de gemeten set).

Meten: `rvt coverage <file>` per bestand (`tools/run_coverage.ps1` loopt een lijst één voor één
af) en `tools/coverage_summary.py <map>` telt per versie.

## Controle tegen de handlezers (`examples/object_parity.rs`)

Vergelijking van waarden uit de objectvelden met de getypeerde lezers (zelfde id):

| Lijst | Bestanden | Records | Velden (alle gelijk) |
|---|---|---|---|
| Levels | 127 | 332 | naam (`m_text`), hoogte en oorsprong (`m_pSurface` → Plane `m_origin`), uv-box (`m_Envelope`), type (`m_attrId`): 332/332 elk |
| Walls | 127 | 1622 | type, base/top level, base/top offset (parameters −1001108/−1001109): 1622/1622; p0/p1 uit `VWallDriver::m_pCrv` (GLine, ≤ 1e-9 ft): 1619/1619 (3 boogwanden niet vergeleken) |
| Materials (rvt100 e.a.) | 120 | 544 | elk laagmateriaal van de compound types is een `MaterialElem`-object: 544/544 |
| Materials tegen referentie-export | 7 (dyn 2025/2026, snowdon hvac/facades 2025/2026, structural 2026) | 763 | naam, rgb (`m_color` 0xBBGGRR), glans, gladheid, transparantie, appearance asset, snij-/oppervlaktepatroon: 763/763; `mat_category` 751/763, `mat_class` 724/763 |

De 39 `mat_class`-verschillen: in 31 gevallen is de waarde van de referentie de
Description-parameter van het materiaal (ALL_MODEL_DESCRIPTION −1010103, bv. "Air" bij klasse
"Gas"), in 8 (en de 12 `mat_category`-verschillen) is de referentie `null` waar het object een
niet-ASCII-tekst heeft ("未指定"). In het object zijn `m_sMaterialClass` en `m_sCategory` bij al
deze materialen gelijk. Dit wijst op de heuristiek van de referentie, maar is niet met Revit
getoetst (Revit-gebruik was voor dit spoor niet toegestaan).

Draaien: `object_parity <file> [referentie.json]` per bestand, `tools/parity_objects_summary.py`.

## Open punten / niet gedaan

- Niet gemeten: snowdon arch 2025 (93 MB, RAM op de gedeelde machine), snowdon facades als
  dekking (alleen materiaalcontrole), Storax/NAS-bestanden (niet in de samplemap) en `.rfa`.
- De ES-velden hebben in de JSON geen type (`?`); de ES-typenaam staat wel in de catalogus.
  Een ES-veld met een onbekende typenaam of container geeft een fout (niet gezien).
- `Global/Latest` (de documentgraaf) wordt alleen voor de ES-catalogus gelezen; zijn andere
  objecten (projectinstellingen, `ADocument::m_appInfoArr`) zijn nog niet via de API
  opvraagbaar, en de 4 resterende bytes zijn niet verklaard.
- Records buiten contentblok 1 (ingebedde familiedocumenten) staan niet in de store
  (`file_channel` leest alleen blok 1).
- Kanaal 103 (saved graphics) is alleen in de dekking meegenomen, niet in `ObjectView`.
- Geheugen: de store houdt alle kanaal-101/102-bodies vast naast de partities.
- De gedeelde `CARGO_TARGET_DIR` (rvt-par-target) bleek onbruikbaar voor twee worktrees
  tegelijk: cargo hasht werkruimteleden zonder pad, dus de rlib van rvt-read van een andere
  worktree overschreef de mijne ("Fresh" maar zonder mijn functies). Daarom gebouwd in
  `C:\Users\AIA12\AppData\Local\Temp\rvt-par-target-c`.
