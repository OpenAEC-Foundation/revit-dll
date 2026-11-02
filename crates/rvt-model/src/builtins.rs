//! Names of built-in parameters and categories, from the public API (`tools/gen_builtins.py`).
//! The file stores only the numbers.

mod data;

/// A public API enum member: its value, name and English label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Builtin {
    pub id: i32,
    pub name: &'static str,
    pub label: Option<&'static str>,
}

fn find(table: &'static [Builtin], id: i64) -> Option<&'static Builtin> {
    let id = i32::try_from(id).ok()?;
    table.binary_search_by_key(&id, |b| b.id).ok().map(|i| &table[i])
}

pub fn builtin_parameter(id: i64) -> Option<&'static Builtin> {
    find(data::PARAMETERS, id)
}

pub fn builtin_category(id: i64) -> Option<&'static Builtin> {
    find(data::CATEGORIES, id)
}
