//! C ABI of the RVT reader (`rvt.dll`).
//!
//! ```c
//! RvtDocument *doc = rvt_open("model.rvt");          // NULL on failure
//! char *json = rvt_query(doc, "{\"query\": \"classes\"}");
//! /* ... */
//! rvt_free(json);
//! rvt_close(doc);
//! ```
//!
//! `rvt_query` takes a JSON request and returns a JSON answer, `{"ok": true, "result": …}` or
//! `{"ok": false, "error": "…"}`; the caller frees it with `rvt_free`. Requests (par C):
//! - `{"query": "info"}`: document facts;
//! - `{"query": "object", "id": 123}`: one element as its schema class with typed fields
//!   (`rvt_model::ObjectView`);
//! - `{"query": "classes"}`: class name → number of elements;
//! - `{"query": "objects", "class": "VWall", "limit": 10, "offset": 0}`: elements of a class and its
//!   subclasses, `{"class", "total", "objects"}`.

use std::ffi::{c_char, CStr, CString};

use serde_json::{json, Value};

/// An open document behind the C ABI.
pub struct RvtDocument(rvt_read::Document);

/// Answers one JSON request on a document (the safe core of `rvt_query`).
pub fn query(doc: &rvt_read::Document, request: &str) -> Value {
    match answer(doc, request) {
        Ok(v) => json!({"ok": true, "result": v}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

fn answer(doc: &rvt_read::Document, request: &str) -> Result<Value, String> {
    let req: Value = serde_json::from_str(request).map_err(|e| format!("request is not JSON: {e}"))?;
    let q = req.get("query").and_then(Value::as_str).ok_or("request has no \"query\"")?;
    let no_store = || "the file has no readable object records".to_string();
    let to = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match q {
        "info" => to(serde_json::to_value(doc.info())),
        "object" => {
            let id = req.get("id").and_then(Value::as_u64).ok_or("\"object\" needs a numeric \"id\"")?;
            doc.object_store().ok_or_else(no_store)?;
            let o = doc.object(id).ok_or_else(|| format!("no object record for element {id}"))?;
            to(serde_json::to_value(o))
        }
        "classes" => {
            doc.object_store().ok_or_else(no_store)?;
            to(serde_json::to_value(doc.classes()))
        }
        "objects" => {
            let class = req.get("class").and_then(Value::as_str).ok_or("\"objects\" needs a \"class\"")?;
            let limit = req.get("limit").and_then(Value::as_u64).map_or(usize::MAX, |n| n as usize);
            let offset = req.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            doc.object_store().ok_or_else(no_store)?;
            let ids = doc.object_ids_of_class(class);
            let objects: Vec<_> = ids.iter().skip(offset).take(limit).filter_map(|&id| doc.object(id)).collect();
            Ok(json!({"class": class, "total": ids.len(), "objects": objects}))
        }
        other => Err(format!("unknown query {other:?} (info, object, classes, objects)")),
    }
}

fn c_string(v: &Value) -> *mut c_char {
    // serde_json escapes control characters, so the text has no interior NUL
    CString::new(v.to_string()).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Opens a `.rvt` file (UTF-8 path). Returns NULL when the path is NULL or the file cannot be read.
///
/// # Safety
/// `path` must be NULL or a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn rvt_open(path: *const c_char) -> *mut RvtDocument {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(p) = CStr::from_ptr(path).to_str() else { return std::ptr::null_mut() };
    match std::panic::catch_unwind(|| rvt_read::Document::open(p)) {
        Ok(Ok(d)) => Box::into_raw(Box::new(RvtDocument(d))),
        _ => std::ptr::null_mut(),
    }
}

/// Answers a JSON request (see the crate documentation). The result is freed with `rvt_free`;
/// NULL only when the answer could not be allocated.
///
/// # Safety
/// `doc` must be NULL or a pointer from `rvt_open` not yet closed; `request` NULL or a
/// NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn rvt_query(doc: *const RvtDocument, request: *const c_char) -> *mut c_char {
    if doc.is_null() || request.is_null() {
        return c_string(&json!({"ok": false, "error": "null document or request"}));
    }
    let Ok(r) = CStr::from_ptr(request).to_str() else { return c_string(&json!({"ok": false, "error": "request is not UTF-8"})) };
    let d = &(*doc).0;
    let v = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| query(d, r))).unwrap_or_else(|_| json!({"ok": false, "error": "internal error"}));
    c_string(&v)
}

/// Frees a string returned by `rvt_query`.
///
/// # Safety
/// `s` must be NULL or a pointer from `rvt_query` not yet freed.
#[no_mangle]
pub unsafe extern "C" fn rvt_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Closes a document from `rvt_open`.
///
/// # Safety
/// `doc` must be NULL or a pointer from `rvt_open` not yet closed.
#[no_mangle]
pub unsafe extern "C" fn rvt_close(doc: *mut RvtDocument) {
    if !doc.is_null() {
        drop(Box::from_raw(doc));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> rvt_read::Document {
        rvt_read::Document::from_partitions(Vec::new())
    }

    #[test]
    fn errors_are_json() {
        let d = empty();
        assert_eq!(query(&d, "nonsense")["ok"], false);
        assert_eq!(query(&d, "{}")["ok"], false);
        assert_eq!(query(&d, r#"{"query": "nope"}"#)["ok"], false);
        assert_eq!(query(&d, r#"{"query": "object"}"#)["ok"], false);
        // a document without object records
        let v = query(&d, r#"{"query": "classes"}"#);
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("object records"));
        assert_eq!(query(&d, r#"{"query": "info"}"#)["ok"], true);
    }

    #[test]
    fn c_abi_round_trip() {
        unsafe {
            assert!(rvt_open(std::ptr::null()).is_null());
            let missing = CString::new("no/such/file.rvt").unwrap();
            assert!(rvt_open(missing.as_ptr()).is_null());
            let req = CString::new(r#"{"query": "info"}"#).unwrap();
            let s = rvt_query(std::ptr::null(), req.as_ptr());
            assert!(CStr::from_ptr(s).to_str().unwrap().contains("\"ok\":false"));
            rvt_free(s);
            let doc = Box::into_raw(Box::new(RvtDocument(empty())));
            let s = rvt_query(doc, req.as_ptr());
            assert!(CStr::from_ptr(s).to_str().unwrap().contains("\"ok\":true"));
            rvt_free(s);
            rvt_close(doc);
        }
    }

    /// With `RVT_SAMPLE=<file.rvt>`: the three object queries on a real file.
    #[test]
    fn sample_file() {
        let Some(p) = std::env::var_os("RVT_SAMPLE") else { return };
        let d = rvt_read::Document::open(p).unwrap();
        let c = query(&d, r#"{"query": "classes"}"#);
        assert_eq!(c["ok"], true);
        let o = query(&d, r#"{"query": "objects", "class": "Element", "limit": 2}"#);
        assert_eq!(o["ok"], true);
        let id = o["result"]["objects"][0]["id"].as_u64().unwrap();
        let one = query(&d, &format!(r#"{{"query": "object", "id": {id}}}"#));
        assert_eq!(one["result"]["id"].as_u64(), Some(id));
    }
}
