//! Schedules (branch open-research, beta): the definition of every `ViewSchedule` and the rows
//! Revit would show, read through the generic object view (par C), named schema fields only.
//!
//! Definition (`DBViewSchedule::m_pSchema` → `ScheduleSchema`; read on rvt100 045 and
//! snowdon arch 2025, 96 schedules):
//! - `m_categoryId` the scheduled category, `m_bInstanceSchedule` = "Itemize every instance",
//!   `m_bMaterialSchedule` a material takeoff, `m_bRefTable` a key schedule,
//!   `m_titleBlockInstId` ≠ −1 a revision schedule inside a title block;
//! - `m_pFields` → `ScheduleField`: `m_columnHeading`, `m_paramElemId` (built-in parameter < 0,
//!   project/shared parameter > 0, −1 for a formula), `m_sourceType` (0 instance, 1 type,
//!   2 count, 5 formula, 7 room, 8 from room, 11 material, 12 material quantity; 4 is used by
//!   'No.' of revisions and 'Area' of rooms and spaces), `m_hidden`, `m_displayType` 1 =
//!   "Calculate totals", `m_pFormatOptions` (`m_bUseDefault` = project units);
//!   `m_fieldIds[i]` is the id of field i (filters and sorting refer to ids);
//! - `m_filterArr` → `ScheduleFilter`: `m_fieldId`, `m_filterType`, `m_value` (`ParamStorage`:
//!   `m_storageType` 1 integer, 2 double, 3 string, 4 element id). The operator numbering is the
//!   API's `ScheduleFilterType` from 2 = Equal (hypothesis, consistent with 045 'Length > 1000'
//!   = 4 and snowdon's two NotEqual family filters = 3; Revit check open);
//! - `m_sortFieldArr` → `ScheduleSortField`: `m_fieldId`, `m_sortOrder` 1 = descending,
//!   `m_groupingFlags` (2 header, 4 footer, 16 blank line; others open);
//! - `m_showGrandTotal`, `m_showGrandTotalTitle`, `m_showGrandTotalCount`, `m_grandTotalTitle`,
//!   `m_altScheduleTitle`;
//! - the schedule's phase: its stored parameter `VIEW_PHASE` (−1012102); project units:
//!   `UnitsElem::m_units` (`AUnits::m_formatOptionsMap`, spec → `FormatOptions`).
//!
//! The rows are not in the file (only the title cell is): Revit computes them from the elements
//! of the category. They are computed here the same way: the elements of the category that
//! exist in the schedule's phase (created in it or before; rooms of that phase), filters,
//! sorting, and for schedules that are not itemized one row per group of equal sort values
//! (count and "Calculate totals" columns summed, differing values blank). Values come from the
//! element's stored parameters (`Element::m_pParamValueSet{Double,Int,AString,ElementId}`), its
//! type's stored parameters, and a table of built-in parameters read from named fields or the
//! typed readers (wall length, area and volume from the location line, height and type
//! thickness, without joins and openings). A cell without a readable value is `None` and
//! counted in `unresolved`. Not computed yet: material takeoffs, key schedules, formulas, room
//! and material fields, group headers and footers, phase filters (all elements of the phase are
//! shown, as with "Show All").

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rvt_model::object::{FieldValue, NestedObject, ObjectView};
use rvt_read::Document;
use serde::Serialize;

/// One schedule: its definition and its computed rows.
#[derive(Debug, Clone, Serialize)]
pub struct Schedule {
    pub id: u64,
    pub name: String,
    /// The title above the table (the name unless an alternative title is set).
    pub title: String,
    pub is_template: bool,
    /// The scheduled category (e.g. −2000011 walls).
    pub category: i64,
    pub kind: ScheduleKind,
    /// "Itemize every instance": one row per element; otherwise one row per group of equal sort
    /// values.
    pub itemize: bool,
    pub grand_total: bool,
    pub columns: Vec<Column>,
    pub filters: Vec<Filter>,
    pub sort: Vec<SortField>,
    /// Whether `rows` are computed (not for material takeoffs and key schedules).
    pub computed: bool,
    pub rows: Vec<Row>,
    /// Cells of visible columns of the elements' rows without a readable value.
    pub unresolved: usize,
    #[serde(skip)]
    phase: Option<i64>,
    #[serde(skip)]
    grand_total_label: Option<String>,
    #[serde(skip)]
    grand_total_count: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ScheduleKind {
    Elements,
    MaterialTakeoff,
    Key,
    /// A revision schedule inside a title block.
    TitleBlockRevisions,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    /// The field id (filters and sorting refer to it).
    pub field: i64,
    pub heading: String,
    /// Built-in parameter (< 0), project or shared parameter (> 0), −1 for a formula.
    pub parameter: i64,
    pub source: FieldSource,
    pub hidden: bool,
    /// "Calculate totals".
    pub totals: bool,
    /// The field's own number format (`None`: project units).
    #[serde(skip)]
    format: Option<Format>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FieldSource {
    Instance,
    Type,
    Count,
    Formula,
    Room,
    FromRoom,
    Material,
    MaterialQuantity,
    Other(i64),
}

impl FieldSource {
    fn from_raw(n: i64) -> Self {
        match n {
            0 => FieldSource::Instance,
            1 => FieldSource::Type,
            2 => FieldSource::Count,
            5 => FieldSource::Formula,
            7 => FieldSource::Room,
            8 => FieldSource::FromRoom,
            11 => FieldSource::Material,
            12 => FieldSource::MaterialQuantity,
            n => FieldSource::Other(n),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Filter {
    pub field: i64,
    pub op: FilterOp,
    pub value: FilterValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FilterOp {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Contains,
    NotContains,
    BeginsWith,
    NotBeginsWith,
    EndsWith,
    NotEndsWith,
    Other(i64),
}

impl FilterOp {
    fn from_raw(n: i64) -> Self {
        use FilterOp::*;
        match n {
            2 => Equal,
            3 => NotEqual,
            4 => GreaterThan,
            5 => GreaterThanOrEqual,
            6 => LessThan,
            7 => LessThanOrEqual,
            8 => Contains,
            9 => NotContains,
            10 => BeginsWith,
            11 => NotBeginsWith,
            12 => EndsWith,
            13 => NotEndsWith,
            n => Other(n),
        }
    }

    /// Operators that hold for an element without a value.
    fn negated(self) -> bool {
        matches!(self, FilterOp::NotEqual | FilterOp::NotContains | FilterOp::NotBeginsWith | FilterOp::NotEndsWith)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum FilterValue {
    Integer(i64),
    /// Internal units (feet, square feet, ...).
    Number(f64),
    Text(String),
    Element(i64),
}

#[derive(Debug, Clone, Serialize)]
pub struct SortField {
    pub field: i64,
    pub descending: bool,
    pub header: bool,
    pub footer: bool,
    pub blank_line: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// The elements of the row: one when itemized, the group's otherwise; none for the grand
    /// total.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<u64>,
    /// Elements in the row (1 when itemized).
    pub count: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub total: bool,
    /// One cell per column (hidden columns included), as Revit displays it.
    pub cells: Vec<Option<String>>,
}

/// Every schedule of the project (templates included, revision schedules of title blocks
/// left out), sorted by name, with computed rows. Empty when the file has no readable object
/// records.
pub fn schedules(doc: &Document) -> Vec<Schedule> {
    let mut out: Vec<Schedule> = doc
        .object_ids_of_class("DBViewSchedule")
        .into_iter()
        .filter_map(|id| doc.object(id))
        .filter_map(|o| definition(&o))
        .filter(|s| s.kind != ScheduleKind::TitleBlockRevisions)
        .collect();
    let mut cx = Context::new(doc);
    for s in &mut out {
        let t = std::time::Instant::now();
        compute(&mut cx, s);
        if std::env::var_os("RVT_SCHEDULE_TIMING").is_some() {
            eprintln!("schedule {:>7.2}s {} ({} rows)", t.elapsed().as_secs_f64(), s.name, s.rows.len());
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    out
}

// ---------------------------------------------------------------------------------------------
// Definition

fn definition(o: &ObjectView) -> Option<Schedule> {
    let schema = o.deref(o.get("m_pSchema")?)?;
    let name = o.get("m_viewName").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let flag = |n: &str| boolean(schema.get(n)).unwrap_or(false);
    let ids: Vec<i64> = list(schema.get("m_fieldIds")).iter().filter_map(|v| v.as_object()?.get("m_id")?.as_i64()).collect();
    let mut columns = Vec::new();
    for (i, f) in list(schema.get("m_pFields")).iter().enumerate() {
        let Some(f) = o.deref(f) else { continue };
        columns.push(Column {
            field: ids.get(i).copied().unwrap_or(i as i64),
            heading: f.get("m_columnHeading").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            parameter: f.get("m_paramElemId").and_then(|v| v.element()).unwrap_or(-1),
            source: FieldSource::from_raw(f.get("m_sourceType").and_then(|v| v.as_i64()).unwrap_or(0)),
            hidden: boolean(f.get("m_hidden")).unwrap_or(false),
            totals: f.get("m_displayType").and_then(|v| v.as_i64()) == Some(1),
            format: f.get("m_pFormatOptions").and_then(|v| o.deref(v)).filter(|fo| boolean(fo.get("m_bUseDefault")) == Some(false)).and_then(Format::read),
        });
    }
    let filters = list(schema.get("m_filterArr"))
        .iter()
        .filter_map(|f| {
            let f = o.deref(f)?;
            let v = f.get("m_value")?.as_object()?;
            let value = match v.get("m_storageType")?.as_i64()? {
                1 => FilterValue::Integer(v.get("m_int")?.as_i64()?),
                2 => FilterValue::Number(v.get("m_dbl")?.as_f64()?),
                3 => FilterValue::Text(v.get("m_str")?.as_object()?.get("m_str")?.as_str()?.to_string()),
                4 => FilterValue::Element(v.get("m_elemId")?.element()?),
                _ => return None,
            };
            Some(Filter { field: field_id(f.get("m_fieldId"))?, op: FilterOp::from_raw(f.get("m_filterType")?.as_i64()?), value })
        })
        .collect();
    let sort = list(schema.get("m_sortFieldArr"))
        .iter()
        .filter_map(|s| {
            let s = o.deref(s)?;
            let flags = s.get("m_groupingFlags").and_then(|v| v.as_i64()).unwrap_or(0);
            Some(SortField {
                field: field_id(s.get("m_fieldId"))?,
                descending: s.get("m_sortOrder").and_then(|v| v.as_i64()) == Some(1),
                header: flags & 2 != 0,
                footer: flags & 4 != 0,
                blank_line: flags & 16 != 0,
            })
        })
        .collect();
    let kind = if schema.get("m_titleBlockInstId").and_then(|v| v.element()).is_some_and(|e| e >= 0) {
        ScheduleKind::TitleBlockRevisions
    } else if flag("m_bRefTable") {
        ScheduleKind::Key
    } else if flag("m_bMaterialSchedule") {
        ScheduleKind::MaterialTakeoff
    } else {
        ScheduleKind::Elements
    };
    let alt = schema.get("m_altScheduleTitle").and_then(|v| v.as_str()).unwrap_or_default();
    let label = schema.get("m_grandTotalTitle").and_then(|v| v.as_str()).unwrap_or("Grand total").to_string();
    Some(Schedule {
        id: o.id,
        title: if alt.is_empty() { name.clone() } else { alt.to_string() },
        name,
        is_template: boolean(o.get("m_isTemplate")).unwrap_or(false),
        category: schema.get("m_categoryId").and_then(|v| v.element()).unwrap_or(-1),
        kind,
        itemize: flag("m_bInstanceSchedule"),
        grand_total: flag("m_showGrandTotal"),
        columns,
        filters,
        sort,
        computed: false,
        rows: Vec::new(),
        unresolved: 0,
        phase: stored(o, -1012102).and_then(|v| match v {
            Stored::Element(e) => Some(e),
            _ => None,
        }),
        grand_total_label: flag("m_showGrandTotalTitle").then_some(label),
        grand_total_count: flag("m_showGrandTotalCount"),
    })
}

fn field_id(v: Option<&FieldValue>) -> Option<i64> {
    v?.as_object()?.get("m_id")?.as_i64()
}

// ---------------------------------------------------------------------------------------------
// Rows

/// A cell value before display.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Text(String),
    Int(i64),
    /// Internal units.
    Num(f64, Spec),
    Elem(i64, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spec {
    Length,
    Area,
    Volume,
    Angle,
    Number,
}

fn compute(cx: &mut Context, s: &mut Schedule) {
    if s.kind != ScheduleKind::Elements {
        return;
    }
    s.computed = true;
    let ids = cx.category_elements(s.category);
    let mut items: Vec<(u64, Vec<Option<Val>>)> = Vec::new();
    for id in ids {
        let Some(o) = cx.object(id) else { continue };
        if !cx.in_phase(&o, s.phase) {
            continue;
        }
        let vals: Vec<Option<Val>> = s.columns.iter().map(|c| cx.value(&o, c)).collect();
        if s.filters.iter().all(|f| s.columns.iter().position(|c| c.field == f.field).is_none_or(|i| passes(f, vals[i].as_ref(), cx))) {
            items.push((id, vals));
        }
    }
    s.unresolved = items
        .iter()
        .map(|(_, v)| s.columns.iter().zip(v).filter(|(c, v)| !c.hidden && c.source != FieldSource::Count && v.is_none()).count())
        .sum();

    let keys: Vec<(usize, bool)> = s.sort.iter().filter_map(|k| Some((s.columns.iter().position(|c| c.field == k.field)?, k.descending))).collect();
    let order = |a: &Vec<Option<Val>>, b: &Vec<Option<Val>>, cx: &Context| {
        for &(i, desc) in &keys {
            let o = compare(a[i].as_ref(), b[i].as_ref(), cx);
            if o != Ordering::Equal {
                return if desc { o.reverse() } else { o };
            }
        }
        Ordering::Equal
    };
    items.sort_by(|a, b| order(&a.1, &b.1, cx).then(a.0.cmp(&b.0)));

    let total = items.len();
    if s.itemize {
        s.rows = items.iter().map(|(id, v)| Row { elements: vec![*id], count: 1, total: false, cells: cells(cx, s, v, 1) }).collect();
    } else {
        let mut groups: Vec<(Vec<u64>, Vec<&Vec<Option<Val>>>)> = Vec::new();
        for (id, v) in &items {
            match groups.last_mut() {
                Some((ids, g)) if order(g[0], v, cx) == Ordering::Equal => {
                    ids.push(*id);
                    g.push(v);
                }
                _ => groups.push((vec![*id], vec![v])),
            }
        }
        s.rows = groups
            .into_iter()
            .map(|(elements, g)| Row { count: g.len(), total: false, cells: merged(cx, s, &g), elements })
            .collect();
    }
    if s.grand_total {
        let all: Vec<&Vec<Option<Val>>> = items.iter().map(|(_, v)| v).collect();
        let mut cells: Vec<Option<String>> = s
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if c.source == FieldSource::Count {
                    Some(total.to_string())
                } else if c.totals {
                    sum(&all, i).map(|v| cx.display(&v, c))
                } else {
                    None
                }
            })
            .collect();
        let label = match (&s.grand_total_label, s.grand_total_count) {
            (Some(l), true) => Some(format!("{l}: {total}")),
            (Some(l), false) => Some(l.clone()),
            (None, true) => Some(total.to_string()),
            (None, false) => None,
        };
        if let (Some(label), Some(first)) = (label, s.columns.iter().position(|c| !c.hidden)) {
            cells[first] = Some(label);
        }
        s.rows.push(Row { elements: Vec::new(), count: total, total: true, cells });
    }
}

fn cells(cx: &Context, s: &Schedule, v: &[Option<Val>], count: usize) -> Vec<Option<String>> {
    s.columns
        .iter()
        .zip(v)
        .map(|(c, v)| if c.source == FieldSource::Count { Some(count.to_string()) } else { v.as_ref().map(|v| cx.display(v, c)) })
        .collect()
}

/// A row for a group of elements: equal values kept, counts and totals summed, the rest blank.
fn merged(cx: &Context, s: &Schedule, g: &[&Vec<Option<Val>>]) -> Vec<Option<String>> {
    s.columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if c.source == FieldSource::Count {
                return Some(g.len().to_string());
            }
            if c.totals {
                return sum(g, i).map(|v| cx.display(&v, c));
            }
            // differing values: blank; unknown only when no element has a value
            let first = g[0][i].as_ref().map(|v| cx.display(v, c));
            if g.iter().all(|v| v[i].as_ref().map(|v| cx.display(v, c)) == first) {
                first
            } else if g.iter().all(|v| v[i].is_none()) {
                None
            } else {
                Some(String::new())
            }
        })
        .collect()
}

fn sum(g: &[&Vec<Option<Val>>], i: usize) -> Option<Val> {
    let mut spec = None;
    let mut t = 0.0;
    for v in g {
        match &v[i] {
            Some(Val::Num(x, sp)) => {
                t += x;
                spec = Some(*sp);
            }
            Some(Val::Int(x)) => {
                t += *x as f64;
                spec.get_or_insert(Spec::Number);
            }
            _ => {}
        }
    }
    spec.map(|sp| Val::Num(t, sp))
}

fn passes(f: &Filter, v: Option<&Val>, cx: &Context) -> bool {
    let Some(v) = v else { return f.op.negated() };
    let ord = match (&f.value, v) {
        (FilterValue::Number(x), Val::Num(y, _)) => cmp_f64(*y, *x),
        (FilterValue::Number(x), Val::Int(y)) => cmp_f64(*y as f64, *x),
        (FilterValue::Integer(x), Val::Int(y)) => Some(y.cmp(x)),
        (FilterValue::Integer(x), Val::Num(y, _)) => cmp_f64(*y, *x as f64),
        (FilterValue::Element(x), Val::Elem(y, _)) => Some(if x == y { Ordering::Equal } else { y.cmp(x) }),
        (FilterValue::Text(t), v) => {
            let (a, b) = (cx.text(v).to_lowercase(), t.to_lowercase());
            return match f.op {
                FilterOp::Contains => a.contains(&b),
                FilterOp::NotContains => !a.contains(&b),
                FilterOp::BeginsWith => a.starts_with(&b),
                FilterOp::NotBeginsWith => !a.starts_with(&b),
                FilterOp::EndsWith => a.ends_with(&b),
                FilterOp::NotEndsWith => !a.ends_with(&b),
                op => ordered(op, natural(&a, &b)),
            };
        }
        _ => None,
    };
    match ord {
        Some(o) => ordered(f.op, o),
        None => f.op.negated(),
    }
}

fn ordered(op: FilterOp, o: Ordering) -> bool {
    match op {
        FilterOp::Equal => o == Ordering::Equal,
        FilterOp::NotEqual => o != Ordering::Equal,
        FilterOp::GreaterThan => o == Ordering::Greater,
        FilterOp::GreaterThanOrEqual => o != Ordering::Less,
        FilterOp::LessThan => o == Ordering::Less,
        FilterOp::LessThanOrEqual => o != Ordering::Greater,
        _ => true,
    }
}

/// Numbers equal within 1e-9 relative (internal units differ in the last bits).
fn cmp_f64(a: f64, b: f64) -> Option<Ordering> {
    if (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0) {
        Some(Ordering::Equal)
    } else {
        a.partial_cmp(&b)
    }
}

fn compare(a: Option<&Val>, b: Option<&Val>, cx: &Context) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(Val::Num(x, _)), Some(Val::Num(y, _))) => cmp_f64(*x, *y).unwrap_or(Ordering::Equal),
        (Some(Val::Int(x)), Some(Val::Int(y))) => x.cmp(y),
        (Some(a), Some(b)) => natural(&cx.text(a).to_lowercase(), &cx.text(b).to_lowercase()),
    }
}

/// Text order with digit runs compared as numbers ("W2" before "W10").
fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Element values

/// Built-in parameters read from named fields or typed readers (ids from the API's
/// `BuiltInParameter`).
mod bip {
    pub const TYPE: i64 = -1002050; // ELEM_TYPE_PARAM
    pub const FAMILY: i64 = -1002051; // ELEM_FAMILY_PARAM
    pub const FAMILY_AND_TYPE: i64 = -1002052; // ELEM_FAMILY_AND_TYPE_PARAM
    pub const SYMBOL_NAME: i64 = -1010103; // the type's own name ('Type' as a type field)
    pub const MARK: i64 = -1001203; // ALL_MODEL_MARK
    pub const LENGTH: i64 = -1004005; // CURVE_ELEM_LENGTH
    pub const HOST_AREA: i64 = -1012805; // HOST_AREA_COMPUTED
    pub const HOST_VOLUME: i64 = -1012806; // HOST_VOLUME_COMPUTED
    pub const WALL_BASE_CONSTRAINT: i64 = -1001107;
    pub const WALL_TOP_CONSTRAINT: i64 = -1001103; // WALL_HEIGHT_TYPE
    pub const WALL_BASE_OFFSET: i64 = -1001108;
    pub const WALL_TOP_OFFSET: i64 = -1001109;
    pub const WALL_WIDTH: i64 = -1001000; // WALL_ATTR_WIDTH_PARAM
    pub const COMMENTS: i64 = -1010106; // ALL_MODEL_INSTANCE_COMMENTS
    pub const TYPE_MARK: i64 = -1001405; // ALL_MODEL_TYPE_MARK
    pub const DATUM_TEXT: i64 = -1008000; // level and grid name
    pub const LEVEL_ELEV: i64 = -1007102;
    pub const SHEET_NAME: i64 = -1007400;
    pub const SHEET_NUMBER: i64 = -1007401;
    pub const VIEW_NAME: i64 = -1005112;
    pub const REVISION_DESCRIPTION: i64 = -1011952;
    pub const REVISION_DATE: i64 = -1011953;
    pub const VIEW_SCALE: i64 = -1005150;
    pub const VIEW_TEMPLATE: i64 = -1005199;
    pub const VIEW_DESCRIPTION: i64 = -1005114; // 'Title on Sheet'
    pub const PLAN_VIEW_LEVEL: i64 = -1005166; // 'Associated Level'
    pub const VIEWPORT_SHEET_NUMBER: i64 = -1005206;
    pub const VIEWPORT_SHEET_NAME: i64 = -1005207;
    pub const VIEW_DETAIL_LEVEL: i64 = -1011002;
    pub const VIEW_DISCIPLINE: i64 = -1005163;
    /// Scope box of a view or a datum: "None" until one is assigned.
    pub const SCOPE_BOXES: [i64; 2] = [-1012202, -1012201];
    /// 'Level' of rooms, areas and spaces, of planting and parking, of groups.
    pub const LEVELS: [i64; 3] = [-1006916, -1002062, -1133500];
    pub const AREAS: [i64; 4] = [HOST_AREA, -1006902, -1114820, -1114247];
    pub const VOLUMES: [i64; 2] = [HOST_VOLUME, -1114821];
    /// Text parameters Revit stores only once they are set (not stored = empty): mark,
    /// comments, type mark, door frame type, finish and material, fire rating, assembly code,
    /// room finishes, occupancy and unit type.
    pub const TEXTS: [i64; 15] =
        [MARK, COMMENTS, TYPE_MARK, -1001209, -1001208, -1001210, -1001206, -1002500, -1006903, -1006904, -1006905, -1006906, -1006907, -1006909, -1001405];
}

/// The kind of a parameter that is not built in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    YesNo,
    Int,
    Number,
    Element,
}

/// `ViewDiscipline` names.
fn discipline(x: i64) -> String {
    match x {
        1 => "Architectural",
        2 => "Structural",
        4 => "Mechanical",
        8 => "Electrical",
        16 => "Plumbing",
        4095 => "Coordination",
        _ => return x.to_string(),
    }
    .to_string()
}

fn yes_no(i: i64) -> String {
    if i != 0 { "Yes" } else { "No" }.to_string()
}

/// A stored parameter value.
enum Stored {
    Double(f64),
    Int(i64),
    Text(String),
    Element(i64),
    /// A family type's value (`FamilySymbol::m_pParams` → `NamedParam`): all storages at once,
    /// the parameter's kind decides.
    Named { text: String, number: f64, element: i64, int: i64 },
}

/// The value of parameter `p` stored on the element (`Element::m_pParamValueSet*`) or, for a
/// family type, in its type parameters (`m_pParams.m_params`).
fn stored(o: &ObjectView, p: i64) -> Option<Stored> {
    if let Some(params) = o.get("m_pParams").and_then(|v| o.deref(v)) {
        for np in list(params.get("m_params")) {
            let Some(np) = np.as_object() else { continue };
            if np.get("m_paramId").and_then(|v| v.element()) == Some(p) {
                return Some(Stored::Named {
                    text: np.get("m_str").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    number: np.get("m_value").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    element: np.get("m_elemId").and_then(|v| v.element()).unwrap_or(-1),
                    int: np.get("m_int").and_then(|v| v.as_i64()).unwrap_or(0),
                });
            }
        }
    }
    for set in ["m_pParamValueSetDouble", "m_pParamValueSetInt", "m_pParamValueSetAString", "m_pParamValueSetElementId"] {
        let Some(s) = o.get(set).and_then(|v| o.deref(v)) else { continue };
        for pv in list(s.get("m_paramSet")) {
            let Some(pv) = pv.as_object() else { continue };
            if pv.get("m_paramId").and_then(|v| v.element()) != Some(p) {
                continue;
            }
            let v = pv.get("m_value")?;
            return match v {
                FieldValue::Float(x) => Some(Stored::Double(*x)),
                FieldValue::Int(x) if set == "m_pParamValueSetDouble" => Some(Stored::Double(*x as f64)),
                FieldValue::Int(x) => Some(Stored::Int(*x)),
                FieldValue::Bool(b) => Some(Stored::Int(*b as i64)),
                FieldValue::Str(t) => Some(Stored::Text(t.clone())),
                FieldValue::Element { element } => Some(Stored::Element(*element)),
                FieldValue::Object(n) => text_of(n).map(Stored::Text),
                _ => None,
            };
        }
    }
    None
}

/// A string kept in a wrapper object (`AStringWrapper { m_str }`).
fn text_of(n: &NestedObject) -> Option<String> {
    n.fields.iter().find_map(|f| f.value.as_str().map(str::to_string))
}

/// Number formats from the project units.
#[derive(Debug, Clone)]
struct Format {
    factor: f64,
    /// Accuracy in display units (e.g. 1 mm, 0.01 m²).
    accuracy: f64,
    symbol: String,
    space: bool,
    trim: bool,
    feet_inches: bool,
}

impl Format {
    fn read(fo: &NestedObject) -> Option<Format> {
        let unit = fo.get("m_unitTypeId")?.as_object()?.get("m_typeId")?.as_str()?;
        let symbol = fo.get("m_symbolTypeId").and_then(|v| v.as_object()?.get("m_typeId")?.as_str()).unwrap_or_default();
        let name = |id: &str| id.split(':').nth(1).and_then(|s| s.split('-').next()).unwrap_or_default().to_string();
        let unit = name(unit);
        let factor = match unit.as_str() {
            "millimeters" => 304.8,
            "centimeters" => 30.48,
            "decimeters" => 3.048,
            "meters" | "metersCentimeters" => 0.3048,
            "inches" | "fractionalInches" => 12.0,
            "feet" | "feetFractionalInches" | "usSurveyFeet" => 1.0,
            "squareMillimeters" => 92_903.04,
            "squareCentimeters" => 929.0304,
            "squareMeters" => 0.092_903_04,
            "squareInches" => 144.0,
            "squareFeet" => 1.0,
            "cubicMillimeters" => 28_316_846.592,
            "cubicCentimeters" => 28_316.846_592,
            "cubicMeters" => 0.028_316_846_592,
            "liters" => 28.316_846_592,
            "cubicInches" => 1728.0,
            "cubicFeet" => 1.0,
            "cubicYards" => 1.0 / 27.0,
            "degrees" => 180.0 / std::f64::consts::PI,
            "radians" => 1.0,
            _ => return None,
        };
        let symbol = match name(symbol).as_str() {
            "" => String::new(),
            "mSup2" => "m²".into(),
            "mSup3" => "m³".into(),
            "ftSup2" => "ft²".into(),
            "ftSup3" => "ft³".into(),
            "inSup2" => "in²".into(),
            "inSup3" => "in³".into(),
            "cmSup2" => "cm²".into(),
            "mmSup2" => "mm²".into(),
            "cmSup3" => "cm³".into(),
            "mmSup3" => "mm³".into(),
            "degree" => "°".into(),
            "footSingleQuote" => "'".into(),
            "inchDoubleQuote" => "\"".into(),
            "l" => "L".into(),
            s => s.to_string(),
        };
        Some(Format {
            factor,
            accuracy: fo.get("m_accuracy").and_then(|v| v.as_f64()).filter(|a| *a > 0.0).unwrap_or(0.01),
            space: symbol != "°" && boolean(fo.get("m_bSuppressSpaces")) != Some(true),
            symbol,
            trim: boolean(fo.get("m_bSuppressTrailingZeros")) == Some(true),
            feet_inches: unit == "feetFractionalInches",
        })
    }

    fn show(&self, internal: f64, comma: bool) -> String {
        if self.feet_inches {
            return feet_inches(internal, self.accuracy);
        }
        let v = internal * self.factor;
        let decimals = (-self.accuracy.log10()).round().max(0.0) as usize;
        let rounded = (v / self.accuracy).round() * self.accuracy;
        let mut t = format!("{:.*}", decimals, if rounded == 0.0 { 0.0 } else { rounded });
        if self.trim && t.contains('.') {
            t = t.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        if comma {
            t = t.replace('.', ",");
        }
        if !self.symbol.is_empty() {
            if self.space {
                t.push(' ');
            }
            t += &self.symbol;
        }
        t
    }
}

/// Feet and fractional inches (`12' - 6 1/2"`); the accuracy in feet (1/256 inch = 1/3072).
fn feet_inches(feet: f64, accuracy: f64) -> String {
    let den = ((1.0 / (accuracy * 12.0)).round() as i64).clamp(1, 256);
    let total = (feet.abs() * 12.0 * den as f64).round() as i64;
    let (ft, rest) = (total / (12 * den), total % (12 * den));
    let (inch, mut num) = (rest / den, rest % den);
    let mut d = den;
    while num > 0 && num % 2 == 0 && d % 2 == 0 {
        num /= 2;
        d /= 2;
    }
    let sign = if feet < 0.0 && total > 0 { "-" } else { "" };
    if num == 0 {
        format!("{sign}{ft}' - {inch}\"")
    } else {
        format!("{sign}{ft}' - {inch} {num}/{d}\"")
    }
}

/// The project's display units per spec.
struct Units {
    length: Format,
    area: Format,
    volume: Format,
    angle: Format,
    comma: bool,
}

impl Units {
    fn read(doc: &Document) -> Units {
        let mm = |f, s: &str, a| Format { factor: f, accuracy: a, symbol: s.to_string(), space: true, trim: false, feet_inches: false };
        let mut u = Units { length: mm(304.8, "", 1.0), area: mm(0.092_903_04, "m²", 0.01), volume: mm(0.028_316_846_592, "m³", 0.01), angle: mm(180.0 / std::f64::consts::PI, "°", 0.01), comma: false };
        let Some(o) = doc.object_ids_of_class("UnitsElem").first().and_then(|&id| doc.object(id)) else { return u };
        let Some(units) = o.get("m_units").and_then(|v| v.as_object()) else { return u };
        u.comma = units.get("m_decimalSymbol").and_then(|v| v.as_i64()) == Some(1);
        for pair in list(units.get("m_formatOptionsMap")) {
            let Some(pair) = pair.as_object() else { continue };
            let Some(spec) = pair.get("first").and_then(|v| v.as_object()?.get("m_typeId")?.as_str()) else { continue };
            let Some(fo) = pair.get("second").and_then(|v| v.as_object()).and_then(Format::read) else { continue };
            match spec.split('-').next().unwrap_or_default() {
                "autodesk.spec.aec:length" => u.length = fo,
                "autodesk.spec.aec:area" => u.area = fo,
                "autodesk.spec.aec:volume" => u.volume = fo,
                "autodesk.spec.aec:angle" => u.angle = fo,
                _ => {}
            }
        }
        u
    }
}

/// Element objects, names and lookups, shared by all schedules of a document.
struct Context<'a> {
    doc: &'a Document,
    objects: HashMap<u64, Option<Rc<ObjectView>>>,
    symbols: HashMap<i64, bool>,
    names: HashMap<i64, Option<String>>,
    kinds: HashMap<i64, Option<Kind>>,
    by_category: Option<HashMap<i64, Vec<u64>>>,
    walls: Option<HashMap<u64, rvt_model::Wall>>,
    phases: Option<HashMap<u64, (i64, Option<i64>)>>,
    phase_order: HashMap<i64, usize>,
    excluded: Option<HashSet<u64>>,
    units: Units,
}

impl<'a> Context<'a> {
    fn new(doc: &'a Document) -> Self {
        let phase_order = doc
            .object_ids_of_class("AllProjectPhases")
            .first()
            .and_then(|&id| doc.object(id))
            .map(|o| list(o.get("m_phaseIds")).iter().enumerate().filter_map(|(i, p)| Some((p.element()?, i))).collect())
            .unwrap_or_default();
        Context {
            doc,
            objects: HashMap::new(),
            symbols: HashMap::new(),
            names: HashMap::new(),
            kinds: HashMap::new(),
            by_category: None,
            walls: None,
            phases: None,
            phase_order,
            excluded: None,
            units: Units::read(doc),
        }
    }

    fn object(&mut self, id: u64) -> Option<Rc<ObjectView>> {
        let doc = self.doc;
        self.objects.entry(id).or_insert_with(|| doc.object(id).map(Rc::new)).clone()
    }

    /// Project elements of a category, without types, group type copies, in-place family
    /// content, elements of secondary design options and view templates.
    fn category_elements(&mut self, category: i64) -> Vec<u64> {
        if self.by_category.is_none() {
            let mut m: HashMap<i64, Vec<u64>> = HashMap::new();
            for id in self.doc.object_ids_of_class("Element") {
                if let Some(e) = self.doc.element(id) {
                    m.entry(e.category as i64).or_default().push(id);
                }
            }
            self.by_category = Some(m);
        }
        if self.excluded.is_none() {
            let mut x = self.doc.secondary_option_elements();
            x.extend(self.doc.group_definition_ids());
            self.excluded = Some(x);
        }
        let ids = self.by_category.as_ref().and_then(|m| m.get(&category)).cloned().unwrap_or_default();
        ids.into_iter()
            .filter(|id| !self.excluded.as_ref().is_some_and(|x| x.contains(id)))
            .filter(|&id| {
                let Some(o) = self.doc.object_root(id) else { return false };
                !o.is_a("Symbol")
                    && boolean(o.get("m_isTemplate")) != Some(true)
                    && o.get("m_famId").and_then(|v| v.element()).is_none_or(|f| f < 0)
                    && o.get("m_unplacedOwnerId").and_then(|v| v.element()).is_none_or(|f| f < 0)
            })
            .collect()
    }

    /// Whether the element exists in the schedule's phase: rooms, areas and spaces belong to
    /// one phase (`m_phaseId`); other elements are shown from their phase created on.
    fn in_phase(&mut self, o: &ObjectView, phase: Option<i64>) -> bool {
        let Some(view) = phase.and_then(|p| self.phase_order.get(&p).copied()) else { return true };
        if let Some(p) = o.get("m_phaseId").and_then(|v| v.element()).filter(|&p| p >= 0) {
            return self.phase_order.get(&p) == Some(&view);
        }
        if self.phases.is_none() {
            self.phases = Some(self.doc.element_phases());
        }
        match self.phases.as_ref().and_then(|m| m.get(&o.id)).and_then(|(c, _)| self.phase_order.get(c)) {
            Some(&created) => created <= view,
            None => true,
        }
    }

    fn is_symbol(&mut self, id: i64) -> bool {
        if id <= 0 {
            return false;
        }
        let doc = self.doc;
        *self.symbols.entry(id).or_insert_with(|| doc.object_is_a(id as u64, "Symbol"))
    }

    /// The element's type: the first referenced element (root fields, then nested objects)
    /// whose class derives from `Symbol`.
    fn type_of(&mut self, o: &ObjectView) -> Option<i64> {
        if o.is_a("Symbol") {
            return Some(o.id as i64);
        }
        let mut ids = Vec::new();
        for f in &o.fields {
            collect_elements(&f.value, &mut ids);
        }
        for n in &o.objects {
            for f in &n.fields {
                collect_elements(&f.value, &mut ids);
            }
        }
        ids.into_iter().filter(|&i| i > 0 && i as u64 != o.id).find(|&i| self.is_symbol(i))
    }

    /// Display name of an element: level and grid text, view, material, type or family name.
    fn name(&mut self, id: i64) -> Option<String> {
        if id < 0 {
            return None;
        }
        if let Some(n) = self.names.get(&id) {
            return n.clone();
        }
        let n = self.object(id as u64).and_then(|o| {
            ["m_text", "m_name", "m_viewName"]
                .iter()
                .find_map(|f| o.get(f).and_then(|v| v.as_str()))
                .or_else(|| o.path(&["m_symbolInfo", "m_name"]).and_then(|v| v.as_str()))
                .or_else(|| o.path(&["m_pMaterial", "m_name"]).and_then(|v| v.as_str()))
                .map(str::to_string)
        });
        self.names.insert(id, n.clone());
        n
    }

    fn family_name(&mut self, ty: i64) -> Option<String> {
        let t = self.object(ty as u64)?;
        match t.get("m_familyId").and_then(|v| v.element()).filter(|&f| f > 0) {
            Some(f) => self.name(f),
            None => Some(crate::props::system_family(&t.class)),
        }
    }

    fn wall(&mut self, id: u64) -> Option<rvt_model::Wall> {
        if self.walls.is_none() {
            self.walls = Some(self.doc.walls().into_iter().map(|w| (w.id, w)).collect());
        }
        self.walls.as_ref()?.get(&id).cloned()
    }

    fn elem(&mut self, id: i64) -> Option<Val> {
        (id >= 0).then(|| Val::Elem(id, self.name(id).unwrap_or_default()))
    }

    fn value(&mut self, o: &ObjectView, c: &Column) -> Option<Val> {
        match c.source {
            FieldSource::Count => Some(Val::Int(1)),
            FieldSource::Instance | FieldSource::Other(4) => self.lookup(o, c.parameter),
            FieldSource::Type => {
                let ty = self.type_of(o)?;
                let t = self.object(ty as u64)?;
                self.lookup(&t, c.parameter)
            }
            _ => None,
        }
    }

    /// Parameter `p` of an element or type: its stored value, else a built-in read from fields.
    fn lookup(&mut self, o: &ObjectView, p: i64) -> Option<Val> {
        if let Some(v) = stored(o, p) {
            let kind = self.kind(p);
            return match v {
                Stored::Double(x) => Some(Val::Num(x, self.spec(p))),
                Stored::Int(x) if kind == Some(Kind::YesNo) => Some(Val::Text(yes_no(x))),
                Stored::Int(x) if p == bip::VIEW_DETAIL_LEVEL => Some(Val::Text(["Undefined", "Coarse", "Medium", "Fine"].get(x as usize).unwrap_or(&"").to_string())),
                Stored::Int(x) if p == bip::VIEW_DISCIPLINE => Some(Val::Text(discipline(x))),
                Stored::Int(x) => Some(Val::Int(x)),
                Stored::Text(t) => Some(Val::Text(t)),
                Stored::Element(e) => self.elem(e),
                Stored::Named { text, number, element, int } => match kind {
                    Some(Kind::Text) => Some(Val::Text(text)),
                    Some(Kind::YesNo) => Some(Val::Text(yes_no(int))),
                    Some(Kind::Int) => Some(Val::Int(int)),
                    Some(Kind::Element) => self.elem(element).or(Some(Val::Text(String::new()))),
                    Some(Kind::Number) => Some(Val::Num(number, self.spec(p))),
                    // a built-in parameter: the storage that holds something
                    None if !text.is_empty() => Some(Val::Text(text)),
                    None if element >= 0 => self.elem(element),
                    None if number != 0.0 => Some(Val::Num(number, self.spec(p))),
                    None if int != 0 => Some(Val::Int(int)),
                    None => None,
                },
            };
        }
        let text = |o: &ObjectView, f: &str| o.get(f).and_then(|v| v.as_str()).map(|s| Val::Text(s.to_string()));
        match p {
            bip::TYPE | bip::SYMBOL_NAME => {
                let t = self.type_of(o)?;
                self.name(t).map(Val::Text)
            }
            bip::FAMILY => {
                let t = self.type_of(o)?;
                self.family_name(t).map(Val::Text)
            }
            bip::FAMILY_AND_TYPE => {
                let t = self.type_of(o)?;
                let (f, n) = (self.family_name(t)?, self.name(t)?);
                Some(Val::Text(format!("{f}: {n}")))
            }
            bip::MARK if o.path(&["m_oFamInstSpec", "m_doorNumber"]).is_some() => {
                o.path(&["m_oFamInstSpec", "m_doorNumber"]).and_then(|v| v.as_str()).map(|s| Val::Text(s.to_string()))
            }
            bip::LENGTH | bip::HOST_AREA | bip::HOST_VOLUME | bip::WALL_BASE_CONSTRAINT | bip::WALL_TOP_CONSTRAINT | bip::WALL_BASE_OFFSET | bip::WALL_TOP_OFFSET => {
                let w = self.wall(o.id)?;
                let length = match &w.curve {
                    Some(a) => a.r * (a.t1 - a.t0).abs(),
                    None => ((w.p1[0] - w.p0[0]).powi(2) + (w.p1[1] - w.p0[1]).powi(2)).sqrt(),
                };
                match p {
                    bip::LENGTH => Some(Val::Num(length, Spec::Length)),
                    bip::HOST_AREA => Some(Val::Num(length * w.height?, Spec::Area)),
                    bip::HOST_VOLUME => Some(Val::Num(length * w.height? * self.doc.type_thickness(w.wall_type)?, Spec::Volume)),
                    bip::WALL_BASE_CONSTRAINT => self.elem(w.base_level),
                    bip::WALL_TOP_CONSTRAINT if w.top_level < 0 => Some(Val::Text("Unconnected".into())),
                    bip::WALL_TOP_CONSTRAINT => {
                        let n = self.name(w.top_level).unwrap_or_default();
                        Some(Val::Elem(w.top_level, format!("Up to level: {n}")))
                    }
                    bip::WALL_BASE_OFFSET => Some(Val::Num(w.base_offset, Spec::Length)),
                    _ => Some(Val::Num(w.top_offset, Spec::Length)),
                }
            }
            bip::WALL_WIDTH => self.doc.type_thickness(o.id as i64).map(|t| Val::Num(t, Spec::Length)),
            bip::DATUM_TEXT => text(o, "m_text"),
            bip::LEVEL_ELEV => self.doc.level_elevation(o.id as i64).map(|z| Val::Num(z, Spec::Length)),
            bip::SHEET_NUMBER => text(o, "m_sheetNumber"),
            bip::SHEET_NAME | bip::VIEW_NAME => text(o, "m_viewName"),
            bip::REVISION_DESCRIPTION => text(o, "m_description"),
            bip::REVISION_DATE => text(o, "m_date"),
            p if bip::LEVELS.contains(&p) => {
                let l = ["m_levelId", "m_assocLevelId"].iter().find_map(|f| o.get(f).and_then(|v| v.element()).filter(|&l| l >= 0))?;
                self.elem(l)
            }
            bip::VIEW_SCALE => {
                let s = o.get("m_scale")?.as_f64().filter(|s| *s > 0.0)?;
                Some(Val::Int((1.0 / s).round() as i64))
            }
            bip::VIEW_TEMPLATE => match o.get("m_viewTemplateId").and_then(|v| v.element()) {
                Some(t) if t >= 0 => self.elem(t),
                _ => Some(Val::Text("<None>".into())),
            },
            bip::VIEW_DESCRIPTION => text(o, "m_viewDescription"),
            bip::PLAN_VIEW_LEVEL => match o.get("m_genElemId").and_then(|v| v.element()) {
                Some(l) if l >= 0 && o.is_a("DBViewPlan") => self.elem(l),
                _ => Some(Val::Text(String::new())),
            },
            bip::VIEWPORT_SHEET_NUMBER | bip::VIEWPORT_SHEET_NAME => {
                let vp = o.get("m_sheetViewportElemId").and_then(|v| v.element()).filter(|&v| v >= 0);
                let sheet = vp.and_then(|v| self.object(v as u64)).and_then(|v| v.get("m_ownerDBViewId")?.element()).filter(|&s| s >= 0);
                let sheet = sheet.and_then(|s| self.object(s as u64));
                let f = if p == bip::VIEWPORT_SHEET_NUMBER { "m_sheetNumber" } else { "m_viewName" };
                Some(Val::Text(sheet.and_then(|s| s.get(f)?.as_str().map(str::to_string)).unwrap_or_default()))
            }
            p if bip::SCOPE_BOXES.contains(&p) => Some(Val::Text("None".into())),
            // parameters Revit only stores once set: not stored = empty
            p if bip::TEXTS.contains(&p) => Some(Val::Text(String::new())),
            p if p > 0 => Some(Val::Text(String::new())),
            _ => None,
        }
    }

    /// The spec of a stored number: areas and volumes by parameter, a project or shared
    /// parameter by the spec its definition names, otherwise a length.
    /// The kind of a project, shared or family parameter from its definition's class
    /// (`ParamElem*::m_pParamDef`: `ParamDefString`, `ParamDefYesNo`, `ParamDefValue`, ...);
    /// `None` for built-in parameters and unknown definitions.
    fn kind(&mut self, p: i64) -> Option<Kind> {
        if p <= 0 {
            return None;
        }
        if let Some(k) = self.kinds.get(&p) {
            return *k;
        }
        let k = self.object(p as u64).and_then(|o| {
            let def = o.deref(o.get("m_pParamDef")?)?;
            let c = def.class.as_str();
            Some(if c.contains("YesNo") {
                Kind::YesNo
            } else if c.contains("String") || c.contains("Text") || c.contains("URL") {
                Kind::Text
            } else if c.contains("Integer") {
                Kind::Int
            } else if c.contains("Value") {
                Kind::Number
            } else if c.contains("Material") || c.contains("Elem") || c.contains("FamilyType") {
                Kind::Element
            } else {
                return None;
            })
        });
        self.kinds.insert(p, k);
        k
    }

    fn spec(&mut self, p: i64) -> Spec {
        if bip::AREAS.contains(&p) {
            return Spec::Area;
        }
        if bip::VOLUMES.contains(&p) {
            return Spec::Volume;
        }
        if p > 0 {
            if let Some(o) = self.object(p as u64) {
                let mut found = None;
                for f in &o.fields {
                    spec_in(&f.value, &mut found);
                }
                for n in &o.objects {
                    for f in &n.fields {
                        spec_in(&f.value, &mut found);
                    }
                }
                if let Some(s) = found {
                    return s;
                }
            }
        }
        Spec::Length
    }

    fn display(&self, v: &Val, c: &Column) -> String {
        match v {
            Val::Num(x, spec) => {
                let fmt = c.format.as_ref().unwrap_or(match spec {
                    Spec::Length => &self.units.length,
                    Spec::Area => &self.units.area,
                    Spec::Volume => &self.units.volume,
                    Spec::Angle => &self.units.angle,
                    Spec::Number => return trimmed(*x),
                });
                fmt.show(*x, self.units.comma)
            }
            v => self.text(v),
        }
    }

    fn text(&self, v: &Val) -> String {
        match v {
            Val::Text(t) => t.clone(),
            Val::Int(i) => i.to_string(),
            Val::Elem(_, n) => n.clone(),
            Val::Num(x, _) => trimmed(*x),
        }
    }
}

fn trimmed(x: f64) -> String {
    let t = format!("{x:.6}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The first `autodesk.spec...` id in a value tree, as a spec.
fn spec_in(v: &FieldValue, out: &mut Option<Spec>) {
    if out.is_some() {
        return;
    }
    match v {
        FieldValue::Str(s) if s.starts_with("autodesk.spec") => {
            let s = s.split('-').next().unwrap_or_default();
            *out = Some(match s {
                "autodesk.spec.aec:length" => Spec::Length,
                "autodesk.spec.aec:area" => Spec::Area,
                "autodesk.spec.aec:volume" => Spec::Volume,
                "autodesk.spec.aec:angle" => Spec::Angle,
                _ => Spec::Number,
            });
        }
        FieldValue::List(l) => l.iter().for_each(|x| spec_in(x, out)),
        FieldValue::Object(o) => o.fields.iter().for_each(|f| spec_in(&f.value, out)),
        _ => {}
    }
}

fn collect_elements(v: &FieldValue, out: &mut Vec<i64>) {
    match v {
        FieldValue::Element { element } => out.push(*element),
        FieldValue::List(l) => l.iter().for_each(|x| collect_elements(x, out)),
        FieldValue::Object(o) => o.fields.iter().for_each(|f| collect_elements(&f.value, out)),
        _ => {}
    }
}

fn list(v: Option<&FieldValue>) -> &[FieldValue] {
    v.and_then(|v| v.as_list()).unwrap_or(&[])
}

fn boolean(v: Option<&FieldValue>) -> Option<bool> {
    match v? {
        FieldValue::Bool(b) => Some(*b),
        FieldValue::Int(i) => Some(*i != 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        assert_eq!(natural("w2", "w10"), Ordering::Less);
        assert_eq!(natural("101a", "101b"), Ordering::Less);
        assert_eq!(natural("a", "a"), Ordering::Equal);
        assert_eq!(natural("b", "a10"), Ordering::Greater);
    }

    #[test]
    fn feet_and_inches() {
        // accuracy in feet: 1/256 inch
        let acc = 1.0 / 3072.0;
        assert_eq!(feet_inches(7.0, acc), "7' - 0\"");
        assert_eq!(feet_inches(0.145_833_333_333_333_34, acc), "0' - 1 3/4\"");
        assert_eq!(feet_inches(3.0 + 1.0 / 24.0, acc), "3' - 0 1/2\"");
    }

    #[test]
    fn number_format() {
        let mm = Format { factor: 304.8, accuracy: 1.0, symbol: String::new(), space: true, trim: false, feet_inches: false };
        assert_eq!(mm.show(19.685_039_370_078_74, false), "6000");
        let m2 = Format { factor: 0.092_903_04, accuracy: 0.01, symbol: "m²".into(), space: true, trim: false, feet_inches: false };
        assert_eq!(m2.show(258.333_333_333_333_3, false), "24.00 m²");
        assert_eq!(m2.show(258.333_333_333_333_3, true), "24,00 m²");
    }

    #[test]
    fn filter_operators() {
        // 045: 'Length > 1000' is stored as 4
        assert_eq!(FilterOp::from_raw(4), FilterOp::GreaterThan);
        assert!(ordered(FilterOp::GreaterThan, cmp_f64(6000.0 / 304.8, 1000.0 / 304.8).unwrap()));
        assert!(!ordered(FilterOp::GreaterThan, cmp_f64(1000.0 / 304.8, 3.280_839_895_013_123_5).unwrap()));
        assert!(ordered(FilterOp::LessThanOrEqual, Ordering::Equal));
        assert!(FilterOp::NotEndsWith.negated() && !FilterOp::Equal.negated());
    }
}
