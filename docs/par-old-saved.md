# Spoor B: saved graphics in oudere Revit-bestanden (open-research par B)

Branch `par/old-saved` (vanaf open-research `b13d5bc`, stap 2308). Notities voor de notebook.

## Vraag

Welke Revit-versies (2014–2026) bevatten saved graphics (kanaal-103-records met root `GElement`, gedecodeerd
met het schema uit `Formats/Latest`), in welke vorm, en kunnen we ze lezen? Breng ze naar de versies waar de
decodering faalt of niets oplevert. Verwachting vooraf: ≤ 2020 faalt, bv. Technical school (Revit 2015) heeft
volgens de notebook "geen saved graphics".

## Waarneming

### 1. Alle versies 2014–2026 hebben saved graphics, in dezelfde vorm

Gemeten met het nieuwe voorbeeld `rvt-read/examples/saved_survey.rs`: per bestand de schema-parse, het aantal
records per kanaal, hoeveel kanaal-103-records exact decoderen en hoeveel een `GElement`-root hebben. Gedaan op
67 bestanden: alle 63 web-/Autodesk-samples (.rvt) en rvt100 001–004.

- De partitie-indeling (SegmentMarker/ContentMarker, kanaal 101/102/103, groepen van records) is vanaf
  Revit 2014 gelijk aan 2024+. Het enige verschil is de id-breedte: u32 t/m 2023, u64 vanaf 2024; die
  keuze zat al in `file_channel` (stap 2307).
- Elk element heeft in elk kanaal een record (101 = 102 = 103 in aantal). Kanaal 103 bevat `GElement`
  (de geometrie) of `SerializedDummy` (2 B, geen geometrie); daarnaast is er één leeg record met id −1.
- Klassenamen en de velden die saved.rs leest (GElement, GInstance/InstanceInfo, Geometry/Face/EdgeLoop/Edge,
  Plane, CylSurf, GPolyMesh, …) bestaan al in het 2014-schema (3619 klassen). De decoder is schema-gestuurd,
  dus veldverschillen tussen versies (bv. `m_id` tegen `m_id64`) vangt hij zelf op. Dat is niet veld voor
  veld vergeleken.
- Kanaal 103 decodeert in elk bestand volledig, op het record met id −1 na. Uitzondering vóór deze tak:
  Revit 2018, zie punt 2.

Technical school 2015 heeft dus wél saved graphics: 8452 `GElement`-records. Sinds `b13d5bc` (stap 2308:
saved-pad ook voor 32-bit-bestanden) tekent de viewer ze al voor 2014–2017 en 2019–2023. "Geen saved graphics"
in de notebook (stap 2306) gold voor de 64-bit-only-stand van toen.

### 2. Revit 2018: schema-parse faalde, nu opgelost

Alle vier 2018-samples (rac basic/advanced, rme basic, rst basic) faalden met `schema name at 351466 out of
range`. Daardoor waren er geen saved graphics en tekende de viewer voor 2018 alleen het oude pad.

Oorzaak, gevonden met een Python-spiegel van de parser (byte-observatie):

- Veld `SiteSurface.m_facets` is genest (`0x500d`, een vector zonder vaste telling).
- Het binnenveld is een vaste array van 3 inline-objecten (`0x100e`, count 3).
- Daarna staan **twee** klasserefs (`14 00 14 00`); de parser las er één (de herhaling volgde de buitentelling,
  en die is 0 bij een vector).
- Revit 2019+ heeft hier een i32-array (`0x1004`), dus zonder klasseref; 2014–2017 hebben het veld niet.

Toets op de schema's van 2014–2026 (`refcount.py`, buiten de repo): de regel "vaste buitentelling → die telling,
vector over vaste inline-array → 2, anders 1" parseert alle 13 versies. Ze leest exact dezelfde velden als
voorheen, behalve dit ene 2018-veld. rvt-rs (Apache-2.0, `src/schema_registry.rs`, issue #421) beschrijft
dezelfde vorm. De code is eigen code; de rvt-rs-kopie in `C:\Users\AIA12\ext` is alleen gelezen.

Na de fix (commit `4eec7ee`):

- 2018: 4024 klassen;
- kanaal 103: rac basic 8010/8011 records, rme 27458/27459, rst 14461/14462, rac advanced 17197/17198
  (telkens alle records behalve dat met id −1);
- kanaal 102 (rac basic): 46 records falen, in dezelfde klassen (VarSketch*, RegenSketchCurvesGStep) als in
  2019. Dat is een al bestaand, versie-onafhankelijk punt.

### 3. Wapening ≤ 2022: geen bruikbare saved graphics (hard onderbouwd)

rst 2020/2018 tekenen in het saved-pad geen wapening; rst 2023–2026 tekenen 268,8 m².

- `RebarInstCache` (de opgeslagen staafhartlijnen die rebar.rs gebruikt) bestaat pas vanaf het 2023-schema.
  Revit 2020–2022 heeft hem niet, en ook geen `RebarSystemLayer.m_uncutCurves`.
- Het diametersveld heet t/m 2021 `RebarBarType.m_barDiameter` en vanaf 2022 `m_barModelDiameter`.
- Kanaal 103 van **elke** wapening in rst 2020 (122 Rebar + 224 RebarInSystem, `rebar_probe` met
  `PROBE_103=1`) bevat precies één `GLine`, wat het aantal staven ook is (1…163). Die lijn loopt exact van
  m_bBox-min naar m_bBox-max:
  - 1372483: oorsprong (31,74; −11,50; −9,69) + 55,49 × richting = (76,03; 21,93; −9,67) = de bbox-hoeken;
  - 1316567 idem.

  Het is dus een diagonaal door de bbox (een placeholder), geen staafgeometrie.
- Conclusie: Revit ≤ 2022 bewaart voor wapening alleen parameters (vorm-id, `m_shapeInstTrf`, aantal posities,
  afstanden, gastvlakken). Tekenen kan alleen door de staafvorm te regenereren; dat valt buiten saved graphics.
  Het oude pad tekende die wapening ook niet, dus dit is geen regressie.

### 4. Revit 2008-bestand (batchprint_2026.rvt, versietag 2008)

- Het schema heeft een andere opbouw: klassen zonder het blok `u32 m + m × 16 B`. Een parser zonder dat blok
  leest 2494 klassen exact tot de 8 nulbytes.
- Het schema kent geen `SegmentMarker`/`ContentMarker`, dus de partities zijn niet in kanalen ingedeeld.
  Saved graphics zijn op deze manier niet bereikbaar.
- Buiten het bereik 2014–2026, niet ingebouwd.

## Versietabel

Representatief bestand per versie (rac basic, plus Technical school). Kolommen:

- **rec./kanaal**: aantal records in elk van kanaal 101, 102 en 103 (gelijk);
- **103 ok**: exact gedecodeerde kanaal-103-records;
- **GElement**: records met root `GElement`;
- **met record / getekend**: elementen van de vervangen batches met een kanaal-103-record / met driehoeken;
- **m²**: oppervlakte van de saved-driehoeken.

| Versie | Bestand | ids | Schema | rec./kanaal | 103 ok | GElement | Batches vervangen | met record / getekend | m² |
|---|---|---|---|---|---|---|---|---|---|
| 2014 | rac_basic_sample_project | 32 | 3619 | 68499 | 68498 | 6151 | 23/23 | 467 / 464 | 12251 |
| 2014 | Technical_school-current_m | 32 | 3619 | 48127 | 48126 | 10660 | 22/22 | 5609 / 5416 | 97787 |
| 2015 | rac_basic_sample_project | 32 | 3725 | 8050 | 8049 | 1240 | 23/23 | 465 / 462 | 12251 |
| 2015 | Technical_school-current_m | 32 | 3725 | 17095 | 17094 | 8452 | 22/22 | 5677 / 5484 | 97795 |
| 2016 | rac_basic_sample_project | 32 | 3792 | 7905 | 7904 | 1221 | 23/23 | 465 / 456 | 12248 |
| 2017 | rac_basic_sample_project | 32 | 3879 | 7946 | 7945 | 1221 | 23/23 | 465 / 456 | 12248 |
| 2018 vóór | rac_basic_sample_project | 32 | **faalt** (byte 351466) | – | – | – | 0 (oude pad) | 0 / 0 | 0 |
| 2018 na | rac_basic_sample_project | 32 | 4024 | 8011 | 8010 | 1221 | 23/23 | 465 / 456 | 12248 |
| 2018 na | rac_advanced / rme / rst basic | 32 | 4024 | 17198 / 27459 / 14462 | alle −1 | 8451 / 12767 / 6577 | 22/22, 19/19, 9/9 | 5677/5484, 7128/6773, 811/451 | 97795, 46936, 2692 |
| 2019 | rac_basic_sample_project | 32 | 4098 | 7966 | 7965 | 1221 | 23/23 | 465 / 456 | 12248 |
| 2020 | racbasicsampleproject | 32 | 4164 | 8009 | 8008 | 1221 | 23/23 | 465 / 456 | 12248 |
| 2021 | racbasicsampleproject | 32 | 4168 | 8082 | 8081 | 1222 | 23/23 | 465 / 456 | 12248 |
| 2022 | racbasicsampleproject | 32 | 4307 | 8156 | 8155 | 1222 | 23/23 | 465 / 456 | 12248 |
| 2023 | racbasicsampleproject | 32 | 4418 | 8277 | 8276 | 1423 | 23/23 | 465 / 456 | 12248 |
| 2024 | racbasicsampleproject | 64 | 4492 | 8289 | 8288 | 1424 | 23/23 | 465 / 456 | 12248 |
| 2025 | racbasicsampleproject | 64 | 4600 | 8328 | 8327 | 1424 | 23/23 | 465 / 456 | 12248 |
| 2026 | racbasicsampleproject | 64 | 4690 | 8402 | 8401 | 1424 | 23/23 | 465 / 456 | 12248 |
| 2008 | batchprint_2026 (tag 2008) | 32 | faalt (ander formaat, zie 4) | geen kanalen | – | – | 0 | 0 | 0 |

"Met record, niet getekend" (bv. 465 tegen 456) zijn elementen waarvan het record geen driehoeken geeft:
lijnen, symbolen, en wapening met alleen een bbox-diagonaal (rst 2018: 811 tegen 451, waarvan 346 wapening).
Rac basic 2014/2015 tekent 464/462 elementen (vanaf 2016: 456) en 3 m² meer; dat is niet per element
uitgezocht.

**Alle overige gemeten samples** (rac/rme/rst basic + advanced 2014–2026, snowdon 2024–2026, dyn,
rvt100 001–004): het schema parseert en kanaal 103 decodeert volledig op het record −1 na. In elk van de
~40 oudere bestanden die door `oldnew.py` gingen, worden alle batches vervangen. Voorbeelden:

- rme basic: 19/19 batches, 7125–7128 elementen;
- rst basic 2016+: 9/9, 811;
- rac advanced: 22/22, 5677.

## Controle

1. **Byte-identiek** (release-build `b13d5bc` tegen `4eec7ee`, `rvt mesh <file> scene.bin`):
   - 2024+: snowdon_arch_2025, rst/rme/racbasic 2026;
   - ouder: racbasic 2020, Technical school 2015, rac basic 2019 en 2014.

   Alle acht zijn identiek. Alleen rac basic 2018 verschilt, zoals verwacht.
2. **Dezelfde modellen over de versies**: saved-uitvoer per batch vergeleken met de 2026-uitvoer van hetzelfde
   model. Die is in stap 2304 tegen Revit-GT getoetst (≈ 1,00 per batch). Geen enkele batch wijkt > 5 % of
   > 0,5 m extent af:
   - racbasic 2014/2015/2019/2023: totaal 12251/12251/12248/12248 m², tegen 12248 in 2026;
   - rmebasic 2014/2015/2018/2020/2023: 46935,7–46935,8 m², tegen 46935,8;
   - rstbasic 2018/2020/2023: per batch gelijk aan 2026, behalve Rebar (≤ 2022 afwezig, zie 3);
   - rac advanced 2014–2019 en Technical school 2015: allemaal 97795,0 m² (TS 2014: 97786,2; dat is een
     andere opslag met 68 elementen minder, alle verschillen in de vliesgevelbatches);
   - rme advanced 2014/2015: 46970,7/46970,4; rst advanced 2014/2015: 21989,8 (beide);
   - rst basic 2014/2015 (een ander model dan 2016+): 5284,3 (beide).
3. **Elementboxen**:
   - Technical school 2015 (`saved_candidates` + `boxcheck.py`): de saved-driehoeken liggen bij elk element
     binnen de opgeslagen element-bbox (Revit's eigen bbox, tolerantie 0,1 ft). De enige uitzondering zijn 3
     ProfileRoofs, die 0,34 ft buiten de bbox vallen.
   - rst basic 2015: idem, zonder uitzonderingen.
4. **Saved tegen het oude pad, op de gewijzigde 2018-bestanden** (`oldnew.py`): rac basic 2018 heeft dezelfde
   verschillen als rac basic 2026 tussen beide paden, en daar is saved tegen GT getoetst:
   - planting 0,62, entourage 0,45, plumbing 0,81: in 2026 was saved = GT, het oude pad zat ernaast;
   - columns 1,56 en stairs 1,53: in 2018 alleen, omdat het oude pad daar kleiner tekent (17,7 tegen 27,6 m²;
     2026 oud 27,6);
   - rst 2018: de batches gelijk aan rst 2020 (geen wapening in beide paden).
5. **Grote verschillen saved/oud in de andere oudere modellen, verklaard**:
   - **Structural framing 0,66 (rst basic/advanced 2014–2015) en 0,88 (TS)**: de saved-balk is door de
     vloer gesneden.
     - Balk 124559: saved bovenkant 11,280 = onderkant vloer 133345 (11,280–12,467). De bbox loopt tot
       12,467; aan beide uiteinden is de balk 0,34 ft ingekort tot de kolom.
     - De opgeslagen faces zonder lus (m_pFirstLoop null, GInfo-vlag 0x8204 zonder 0x80000; randen zonder
       m_next) zijn de ongesneden balk. Die blijven terecht weg.
     - Het oude pad tekent de ongesneden doorsnede, met de overlap in de vloer.
   - **Railings 1,83, stairs 1,63 (TS)**:
     - Saved tekent de echte balusters/posten: per railing tientallen GInstances van 3,37 ft. De 5 trappen
       vullen hun bbox exact.
     - Het oude pad benadert dit; het heeft in 2014/2015 de oude traplay-out.
   - **Slab edges 3,26 (TS)**: saved is Revit's sweep (42 faces), met dezelfde box als de bbox. Het oude pad
     schaalt het profiel heuristisch (stap 2284). Niet verder uitgezocht.
   - **Topography 0,78 (TS)**: saved hoofdterrein 105545 = 43 750 m². Het bbox-grondvlak is 48 400 m²; het
     verschil komt doordat de subregio's/pad als gaten zijn uitgespaard. Waarschijnlijk vult het oude pad het
     terrein zonder die gaten. Niet per driehoek getoetst.
   - **Furniture 0,74, lighting 1,39, windows 1,17 (TS)**: niet uitgezocht; elke saved-box valt binnen de bbox.
   - **TS 2014, oud pad**: het oude pad tekende hier grotendeels dozen (generic models 0,17, lighting 0,14,
     other 0,04). Het saved-pad geeft hetzelfde als TS 2015.

## Conclusie

- Saved graphics zitten in elke Revit-versie van 2014 t/m 2026 in dezelfde vorm (kanaal 103, `GElement`,
  zelfbeschrijvend schema) en zijn leesbaar.
- Het enige leesprobleem was het 2018-schema (één veld met een dubbele klasseref). Dat is opgelost, dus ook
  2018 tekent nu volledig uit saved graphics: 465 elementen in rac basic, 23/23 batches.
- Vóór deze tak waren 2014–2017 en 2019–2023 al door stap 2308 in het saved-pad gekomen.
- Over de versies geeft het saved-pad per model dezelfde uitvoer als de GT-getoetste 2026-versie.
- Wat ontbreekt: wapening ≤ 2022 (alleen een bbox-diagonaal opgeslagen) en het 2008-formaat (geen kanalen).

## Open punten

- Wapening 2016–2022 (rst basic 2020: 346 elementen, in 2023+ 268,8 m²): alleen te tekenen door
  RebarShape + parameters te regenereren. Er is geen saved route.
- Faces zonder lus: ze zijn bij framing en slab edges terecht weggelaten, maar niet op alle 598 (racbasic 2026)
  getoetst.
- TS-verschillen furniture/lighting/windows/slab edges: zonder Revit-GT niet beslist. Saved valt overal binnen
  de bbox. Een GT-meting van Technical school 2015 in Revit (alleen-lezen) zou dit afsluiten.
- 2008-formaat (batchprint): schema-variant zonder GUID-blok is bekend; partities zonder kanalen niet onderzocht.
- Niet getoetst: de NAS-bestanden (Storax; zie stap 2307–2308, andere sessie) en de volledige 166-regressie
  (bewust niet: gedeelde machine). Getoetst zijn alleen de gewijzigde 2018-bestanden, acht byte-identieke
  bestanden en ~40 oudere samples via oldnew.

## Herkomst

- Eigen byte-observatie op de inflated `Formats/Latest` van rac basic 2014–2026.
- Bevestiging van de 2018-vorm: rvt-rs (DrunkOnJava, Apache-2.0), `src/schema_registry.rs` (#421), alleen
  gelezen in de bestaande kopie `C:\Users\AIA12\ext\DrunkOnJava_rvt-rs`. Er is geen code overgenomen.
- Hulpscripts (survey.py, refcount.py, schema_trace.py, cmp.py, oldnew.py, xver.py, boxcheck.py, boxdelta.py,
  ident.py, table.py) staan ongecommit in `.par/` van de worktree.
