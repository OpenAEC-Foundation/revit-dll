//! A family's intelligence: formulas, parameters, forms, reference planes, labelled dimensions,
//! nested families and what drives what.
//!
//! Formula codes were derived by recomputing 2 534 formulas of 256 families from the values stored
//! per family type. Function 8 occurs once and stays unknown; `>` and `<` could also be `>=` and
//! `<=` as far as the numbers tell.

use std::collections::HashMap;

use rvt_core::{Chain, Element, Node};
use rvt_model::builtins::builtin_parameter;
use rvt_model::{Curve, DrivenValue, DrivenValueTerm, FamilyDefinition, FamilyDimension, FamilyDimensionSegment, FamilyLink, FamilyParameter, FamilyReferencePlane, FamilySolid, FamilyTypeDefinition, NestedFamily};

use crate::catalog::{first_guid, guid_text};
use crate::Document;

const P_EXTRUSION_START: i64 = -1001800;
const P_EXTRUSION_END: i64 = -1001801;
const P_REVOLUTION_START: i64 = -1001802;
const P_REVOLUTION_END: i64 = -1001803;
const P_VISIBLE: i64 = -1006205;

/// A formula as stored (`m_oExpression`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    Bin(i64, Box<Expr>, Box<Expr>),
    Un(i64, Box<Expr>),
    Fn(i64, Vec<Expr>),
    Param(i64),
    Paren(Box<Expr>),
    /// Value in internal units, with its spec.
    Num(f64, Option<String>),
    Str(String),
    Other(String),
}

pub(crate) fn parse(n: &Node) -> Expr {
    let sub = |f: &str| Box::new(n.ptr(f).map_or(Expr::Other("?".into()), |x| parse(&x)));
    match n.class() {
        "BinaryOperatorExpression" => Expr::Bin(n.int("m_binaryOperator").unwrap_or(-1), sub("m_pLeftSubexpression"), sub("m_pRightSubexpression")),
        "UnaryOperatorExpression" => Expr::Un(n.int("m_unaryOperator").unwrap_or(-1), sub("m_pSubexpression")),
        "ParenExpression" => Expr::Paren(sub("m_pSubexpression")),
        "FunctionExpression" => Expr::Fn(n.int("m_function").unwrap_or(-1), n.items("m_subexpressions").iter().map(parse).collect()),
        "ParameterExpression" | "CalculatedParameterExpression" => Expr::Param(n.id("m_paramId").unwrap_or(-1)),
        "NumberConstantExpression" => Expr::Num(n.f64("m_value").unwrap_or(0.0), n.child("m_specTypeId").and_then(|s| s.str("m_typeId").map(String::from))),
        "StringConstantExpression" => Expr::Str(n.str("m_value").unwrap_or_default().to_string()),
        "StringVariableExpression" => Expr::Other(n.str("m_str").unwrap_or_default().to_string()),
        other => Expr::Other(other.to_string()),
    }
}

/// A number without floating point noise.
pub(crate) fn number_text(v: f64) -> String {
    let t = format!("{v:.10}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t == "-0" { "0".into() } else { t.to_string() }
}

/// Formula text; lengths in millimetres, angles in degrees. The file keeps the value, not the unit
/// the author typed, so a length typed in metres is written in millimetres here.
pub(crate) fn render(e: &Expr, name: &dyn Fn(i64) -> String) -> String {
    match e {
        Expr::Bin(op, a, b) => {
            let text = match op {
                1 => "+".to_string(),
                2 => "-".to_string(),
                3 => "*".to_string(),
                4 => "/".to_string(),
                5 => "^".to_string(),
                6 => "=".to_string(),
                7 => ">".to_string(),
                8 => "<".to_string(),
                o => format!("op{o}"),
            };
            format!("{} {text} {}", render(a, name), render(b, name))
        }
        Expr::Un(1, a) => format!("-{}", render(a, name)),
        Expr::Un(o, a) => format!("unary{o}({})", render(a, name)),
        Expr::Paren(a) => format!("({})", render(a, name)),
        Expr::Fn(f, args) => {
            let text = match f {
                1 => "sin".to_string(),
                2 => "cos".to_string(),
                3 => "tan".to_string(),
                6 => "atan".to_string(),
                9 => "sqrt".to_string(),
                10 => "if".to_string(),
                11 => "or".to_string(),
                12 => "and".to_string(),
                13 => "not".to_string(),
                16 => "pi".to_string(),
                19 => "roundup".to_string(),
                21 => "size_lookup".to_string(),
                o => format!("function{o}"),
            };
            format!("{text}({})", args.iter().map(|a| render(a, name)).collect::<Vec<_>>().join(", "))
        }
        Expr::Param(i) => name(*i),
        Expr::Num(v, spec) => match spec.as_deref() {
            Some(s) if s.contains(":length-") => format!("{} mm", number_text(v * 304.8)),
            Some(s) if s.contains(":angle-") => format!("{}°", number_text(v.to_degrees())),
            _ => number_text(*v),
        },
        Expr::Str(s) => format!("\"{s}\""),
        Expr::Other(s) => s.clone(),
    }
}

pub(crate) fn line_curve(origin: [f64; 3], dir: [f64; 3], ends: [f64; 2]) -> Curve {
    let at = |t: f64| std::array::from_fn(|i| origin[i] + t * dir[i]);
    Curve { kind: "line", p0: Some(at(ends[0])), p1: Some(at(ends[1])), ..Curve::default() }
}

/// A geometry curve object as a neutral curve; other curve classes keep only their kind.
fn curve_of(n: &Node) -> Curve {
    let ends = n.f64s("m_endParams").filter(|e| e.len() == 2);
    match n.class() {
        "GLine" => match (n.xyz("m_origin"), n.xyz("m_dirVec"), &ends) {
            (Some(o), Some(d), Some(e)) => line_curve(o, d, [e[0], e[1]]),
            _ => Curve { kind: "line", ..Curve::default() },
        },
        "GArc" => Curve {
            kind: "arc",
            center: n.xyz("m_center"),
            x: n.xyz("m_xVec"),
            y: n.xyz("m_yVec"),
            r: n.f64("m_radius"),
            t0: ends.as_ref().map(|e| e[0]),
            t1: ends.as_ref().map(|e| e[1]),
            ..Curve::default()
        },
        _ => Curve { kind: "other", ..Curve::default() },
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Cells of an element's cell list.
fn cells<'a, 's>(n: &Node<'a, 's>) -> Vec<Node<'a, 's>> {
    n.ptr("m_cellList").map_or(Vec::new(), |c| c.items("m_cells"))
}

fn stored_number(n: &Node, id: i64) -> Option<f64> {
    n.ptr("m_pParamValueSetDouble")?.items("m_paramSet").iter().find(|i| i.id("m_paramId") == Some(id))?.f64("m_value")
}

fn stored_int(n: &Node, id: i64) -> Option<i64> {
    n.ptr("m_pParamValueSetInt")?.items("m_paramSet").iter().find(|i| i.id("m_paramId") == Some(id))?.int("m_value")
}

fn solid_kind(class: &str) -> &'static str {
    match class {
        "ExtrusionElem" => "extrusion",
        "BlendElem" => "blend",
        "RevolutionElem" => "revolve",
        "SweepElem" => "sweep",
        "SweptBlendElem" => "swept_blend",
        "FreeFormElement" => "free_form",
        "FormElem" => "form",
        _ => "other",
    }
}

impl Document {
    /// A family's definition. Without `id`: the family this file defines.
    pub fn family_definition(&self, id: Option<u64>) -> Option<FamilyDefinition> {
        let own = self.formats.as_ref()?.owner_family;
        let id = id.or_else(|| own.map(|f| f as u64))?;
        let host = self.decoded(id)?.ok()?;
        // a loaded family points to its embedded document (`m_oFamDoc`, key of the chain); the
        // document's own Family element holds the definition
        let key = host.root().ptr("m_oFamDoc").and_then(|x| first_guid(&x));
        let chain = key.and_then(|k| self.partitions.iter().find_map(|p| p.chains.iter().find(|c| c.key == Some(k))));
        let owner = chain.and_then(|c| {
            let text = guid_text(&key?);
            self.content_documents().into_iter().find(|x| x.key == text)?.owner_family.filter(|o| c.elements.iter().any(|e| e.id as i64 == *o))
        });
        let inner = chain.zip(owner).and_then(|(c, o)| self.decode(c, c.elements.iter().find(|e| e.id as i64 == o)?)?.ok());
        let d = inner.as_ref().unwrap_or(&host);
        let n = d.root();
        let family = owner.unwrap_or(id as i64);
        let mut names = self.parameter_names();
        let mut chain_defs: HashMap<i64, (Option<String>, Option<String>, Option<String>)> = HashMap::new();
        if let (Some(c), Some(base)) = (chain, self.schema().and_then(|s| s.id_of("ParamElem"))) {
            for e in c.elements.iter().filter(|e| self.is_a(e, base)) {
                if let Some(Ok(x)) = self.decode(c, e) {
                    let r = x.root();
                    let def = r.ptr("m_pParamDef");
                    if let Some(caption) = def.and_then(|q| q.str("m_caption")) {
                        names.insert(e.id as i64, caption.to_string());
                    }
                    let forge = |f: &str| def.and_then(|q| q.child(f)).and_then(|q| q.str("m_typeId")).filter(|t| !t.is_empty()).map(String::from);
                    let guid = r.child("m_externalParamKey").and_then(|k| first_guid(&k)).map(|g| guid_text(&g));
                    chain_defs.insert(e.id as i64, (forge("m_specTypeId"), forge("m_groupTypeId"), guid));
                }
            }
        }
        let name_of = |i: i64| names.get(&i).cloned().or_else(|| builtin_parameter(i).map(|b| b.label.unwrap_or(b.name).to_string())).unwrap_or_else(|| format!("parameter {i}"));
        // the types of a loaded family are kept in the project's Family element
        let read_types = |x: &Node| -> Vec<FamilyTypeDefinition> {
            x.ptr("m_pFamilyTypes").map_or(Vec::new(), |t| {
                t.items("m_pairs")
                    .iter()
                    .map(|p| FamilyTypeDefinition {
                        name: p.str("name").unwrap_or_default().to_string(),
                        parameters: p.child("params").map_or(Vec::new(), |q| self.named_values(&q.items("m_params"), &names)),
                    })
                    .collect()
            })
        };
        let mut types = read_types(&n);
        if types.is_empty() {
            types = read_types(&host.root());
        }
        let current = n.ptr("m_familyParams").map_or(Vec::new(), |q| self.named_values(&q.items("m_params"), &names));
        let definitions: HashMap<i64, rvt_model::ParameterDef> = self.parameters().definitions.into_iter().map(|x| (x.id as i64, x)).collect();
        let parameters = n.ptr("m_familyParams").map_or(Vec::new(), |q| {
            q.items("m_params")
                .iter()
                .filter_map(|item| {
                    let pid = item.id("m_paramId")?;
                    let def = definitions.get(&pid);
                    let inner_def = chain_defs.get(&pid);
                    Some(FamilyParameter {
                        id: pid,
                        name: name_of(pid),
                        builtin: builtin_parameter(pid).map(|b| b.name),
                        instance: item.bool("m_instance").unwrap_or(false),
                        reporting: item.bool("m_reporting").unwrap_or(false),
                        formula: item.ptr("m_oExpression").map(|e| render(&parse(&e), &name_of)),
                        spec: inner_def.and_then(|x| x.0.clone()).or_else(|| def.and_then(|x| x.spec.clone())),
                        group: inner_def.and_then(|x| x.1.clone()).or_else(|| def.and_then(|x| x.group.clone())),
                        guid: inner_def.and_then(|x| x.2.clone()).or_else(|| def.and_then(|x| x.guid.clone())),
                    })
                })
                .collect()
        });
        let driven = n.ptr("m_oFamDimConstrMgr").map_or(Vec::new(), |m| {
            m.items("m_paramExprs")
                .iter()
                .filter_map(|pair| {
                    let key = pair.child("first")?;
                    let expr = pair.child("second")?;
                    Some(DrivenValue {
                        element: key.id("m_elementId")?,
                        index: key.int("m_int64").unwrap_or(0),
                        terms: expr.items("m_entries").iter().filter_map(|t| Some(DrivenValueTerm { factor: t.f64("m_coef")?, parameter: t.id("m_paramId")?, name: name_of(t.id("m_paramId")?) })).collect(),
                    })
                })
                .collect()
        });
        let document_chain = chain.map(|c| c.index);

        let mut decoded = Vec::new();
        for (c, e) in self.family_elements(family, chain) {
            if let Some(Ok(x)) = self.decode(c, e) {
                decoded.push((e, x));
            }
        }
        let symbols: HashMap<i64, (String, Option<i64>)> = self
            .host_all("Symbol")
            .iter()
            .map(|(e, x)| (e.id as i64, (x.root().ptr("m_symbolInfo").and_then(|s| s.str("m_name").map(String::from)).unwrap_or_default(), x.root().id("m_familyId"))))
            .collect();
        let family_names: HashMap<i64, String> = self.host_all("Family").iter().filter_map(|(e, x)| Some((e.id as i64, x.root().str("m_name")?.to_string()))).collect();
        let links = |x: &Node| -> Vec<FamilyLink> {
            cells(x)
                .iter()
                .flat_map(|c| c.items("m_paramDrivenData"))
                .filter_map(|p| {
                    let (property, parameter) = (p.id("m_elemPropId")?, p.id("m_famParamId")?);
                    Some(FamilyLink { property, property_name: name_of(property), parameter, parameter_name: name_of(parameter) })
                })
                .collect()
        };
        // sketch -> its curves, through the sketch membership of curve elements
        let mut sketch_curves: HashMap<i64, Vec<Curve>> = HashMap::new();
        for (_, x) in &decoded {
            let r = x.root();
            if r.class() != "CurveElem" {
                continue;
            }
            let sketch = cells(&r).iter().find(|c| c.class() == "SketchMembership").and_then(|c| c.id("m_groupId")).filter(|s| *s > 0);
            let curve = r.ptr("m_pCurveDriver").and_then(|driver| driver.ptr("m_pCrv")).map(|c| curve_of(&c));
            if let (Some(s), Some(c)) = (sketch, curve) {
                sketch_curves.entry(s).or_default().push(c);
            }
        }
        let (mut solids, mut reference_planes, mut dimensions, mut nested) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (e, x) in &decoded {
            let r = x.root();
            let class = r.class();
            let lineage = |base: &str| self.schema().and_then(|s| s.id_of(base)).is_some_and(|b| self.is_a(e, b));
            if lineage("GenSweep") {
                let mut sketches = Vec::new();
                sketches.extend(r.id("m_sketchId"));
                sketches.extend(r.ids("m_sketchIds").unwrap_or_default());
                sketches.extend(r.id("m_pathSketchId"));
                let mut loops: Vec<Vec<Curve>> = Vec::new();
                for c in cells(&r) {
                    sketches.extend(c.id("m_sketchId"));
                    for l in c.items("m_pCurveLoops") {
                        loops.push(l.items("m_curves").iter().map(curve_of).collect());
                    }
                }
                sketches.retain(|s| *s > 0);
                sketches.dedup();
                if loops.is_empty() {
                    loops = sketches.iter().filter_map(|s| sketch_curves.get(s).cloned()).collect();
                }
                let revolve = class == "RevolutionElem";
                solids.push(FamilySolid {
                    id: e.id,
                    class: class.to_string(),
                    kind: solid_kind(class),
                    is_void: r.bool("m_cutting").unwrap_or(false),
                    visible: stored_int(&r, P_VISIBLE).map(|v| v != 0),
                    material: r.id("m_materialId").filter(|m| *m > 0),
                    subcategory: r.id("m_categoryId").filter(|m| *m != -1),
                    visibility_flags: r.child("m_famElemVisibility").and_then(|v| v.int("m_flags")),
                    start: stored_number(&r, if revolve { P_REVOLUTION_START } else { P_EXTRUSION_START }).or(if stored_number(&r, if revolve { P_REVOLUTION_END } else { P_EXTRUSION_END }).is_some() { Some(0.0) } else { None }),
                    end: stored_number(&r, if revolve { P_REVOLUTION_END } else { P_EXTRUSION_END }),
                    sketches,
                    loops,
                    links: links(&r),
                });
            } else if class == "RefPlane" {
                if let Some(p) = r.ptr("m_pSurface") {
                    let (x_axis, y_axis) = (p.xyz("m_xVec"), p.xyz("m_yVec"));
                    reference_planes.push(FamilyReferencePlane {
                        id: e.id,
                        name: r.str("m_text").unwrap_or_default().to_string(),
                        origin: p.xyz("m_origin"),
                        normal: x_axis.zip(y_axis).map(|(a, b)| cross(a, b)),
                        reference_kind: r.int("m_refName").unwrap_or(0),
                        links: links(&r),
                    });
                }
            } else if lineage("Dimension") {
                let segments: Vec<FamilyDimensionSegment> = r
                    .items("m_ArrSegInfo")
                    .iter()
                    .map(|s| {
                        let label = s.id("m_paramId").filter(|l| *l != -1);
                        let value = s.items("m_values").first().and_then(|v| v.f64("m_value"));
                        FamilyDimensionSegment { value: value.or(s.f64("m_lockedValue")), label, label_name: label.map(name_of) }
                    })
                    .collect();
                dimensions.push(FamilyDimension { id: e.id, class: class.to_string(), view: r.id("m_ownerDBViewId").filter(|v| *v != -1), segments });
            } else if class == "FamilyInstance" {
                let symbol = r.id("m_masterSymbolId").unwrap_or(-1);
                let info = symbols.get(&symbol);
                nested.push(NestedFamily {
                    id: e.id,
                    symbol,
                    type_name: info.map(|i| i.0.clone()),
                    family: info.and_then(|i| i.1),
                    family_name: info.and_then(|i| i.1).and_then(|f| family_names.get(&f).cloned()),
                    links: links(&r),
                });
            }
        }
        Some(FamilyDefinition {
            id,
            name: host.root().str("m_name").unwrap_or_default().to_string(),
            category: host.root().id("m_categoryId").unwrap_or(-1),
            parameters,
            types,
            current,
            solids,
            reference_planes,
            dimensions,
            nested,
            driven,
            document_chain,
        })
    }

    /// Elements owned by a family (`m_famId`), in its embedded document if it has one. A document
    /// also holds the content of the families nested in it.
    fn family_elements<'a>(&'a self, family: i64, chain: Option<&'a Chain>) -> Vec<(&'a Chain, &'a Element)> {
        let own = |c: &Chain, e: &Element| matches!(self.decode(c, e), Some(Ok(d)) if d.root().id("m_famId") == Some(family));
        match chain {
            Some(c) => c.elements.iter().filter(|e| own(c, e)).map(|e| (c, e)).collect(),
            None => self.host_elements().filter(|(c, e)| own(c, e)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(id: i64) -> String {
        match id {
            1 => "diameter".into(),
            2 => "Rechts".into(),
            _ => format!("p{id}"),
        }
    }

    fn num(v: f64) -> Expr {
        Expr::Num(v, None)
    }

    fn p(i: i64) -> Expr {
        Expr::Param(i)
    }

    #[test]
    fn binary_operators_keep_order_and_spaces() {
        let e = Expr::Bin(3, Box::new(p(1)), Box::new(num(0.5)));
        assert_eq!(render(&e, &names), "diameter * 0.5");
        for (code, text) in [(1, "+"), (2, "-"), (4, "/"), (5, "^"), (6, "="), (7, ">"), (8, "<")] {
            let e = Expr::Bin(code, Box::new(p(1)), Box::new(p(2)));
            assert_eq!(render(&e, &names), format!("diameter {text} Rechts"));
        }
    }

    #[test]
    fn stored_parentheses_and_unary_minus_are_written() {
        let e = Expr::Bin(3, Box::new(Expr::Paren(Box::new(Expr::Bin(1, Box::new(p(1)), Box::new(num(2.0)))))), Box::new(Expr::Un(1, Box::new(p(2)))));
        assert_eq!(render(&e, &names), "(diameter + 2) * -Rechts");
    }

    #[test]
    fn functions_have_their_names_and_arguments() {
        let e = Expr::Fn(10, vec![Expr::Bin(7, Box::new(p(1)), Box::new(num(1.0))), p(2), Expr::Fn(13, vec![p(2)])]);
        assert_eq!(render(&e, &names), "if(diameter > 1, Rechts, not(Rechts))");
        assert_eq!(render(&Expr::Fn(16, vec![]), &names), "pi()");
        assert_eq!(render(&Expr::Fn(12, vec![p(1), p(2)]), &names), "and(diameter, Rechts)");
        assert_eq!(render(&Expr::Fn(11, vec![p(1), p(2)]), &names), "or(diameter, Rechts)");
    }

    #[test]
    fn unknown_function_codes_stay_visible() {
        assert_eq!(render(&Expr::Fn(8, vec![p(1)]), &names), "function8(diameter)");
        let e = Expr::Bin(99, Box::new(p(1)), Box::new(p(2)));
        assert_eq!(render(&e, &names), "diameter op99 Rechts");
    }

    #[test]
    fn size_lookup_writes_its_table_column_as_text() {
        let e = Expr::Fn(21, vec![p(1), Expr::Str("kolom A".into()), num(0.0), p(2)]);
        assert_eq!(render(&e, &names), "size_lookup(diameter, \"kolom A\", 0, Rechts)");
    }

    #[test]
    fn constants_with_a_unit_are_written_in_that_unit() {
        let len = Expr::Num(0.32808398950131235, Some("autodesk.spec.aec:length-2.0.0".into()));
        assert_eq!(render(&len, &names), "100 mm");
        let ang = Expr::Num(std::f64::consts::FRAC_PI_2, Some("autodesk.spec.aec:angle-2.0.0".into()));
        assert_eq!(render(&ang, &names), "90°");
        let plain = Expr::Num(0.5, Some("autodesk.spec.aec:number-2.0.0".into()));
        assert_eq!(render(&plain, &names), "0.5");
    }

    #[test]
    fn number_text_drops_float_noise() {
        assert_eq!(number_text(0.30000000000000004), "0.3");
        assert_eq!(number_text(1250.0), "1250");
    }

    #[test]
    fn line_ends_follow_from_origin_direction_and_parameters() {
        let c = line_curve([1.0, 2.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 3.0]);
        assert_eq!(c.kind, "line");
        assert_eq!(c.p0, Some([0.0, 2.0, 0.0]));
        assert_eq!(c.p1, Some([4.0, 2.0, 0.0]));
    }
}
