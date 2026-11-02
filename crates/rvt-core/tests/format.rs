//! Schema (`Formats/Latest`) and body decoding, on hand-built bytes: no model data.

use rvt_core::{decode, EsField, EsSchema, EsSchemas, Schema, Value};

/// Builds schema bytes field by field.
#[derive(Default)]
struct B(Vec<u8>);

impl B {
    fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn raw(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    /// `u16 0 | u16 n | name` (a top-level or inline class start).
    fn class(self, name: &str) -> Self {
        self.u16(0).u16(name.len() as u16).raw(name.as_bytes())
    }
    /// `u32 n | name` and the first four type bytes.
    fn field(self, name: &str, ty: [u8; 4]) -> Self {
        self.u32(name.len() as u32).raw(name.as_bytes()).raw(&ty)
    }
}

/// ElementId { Identifier { i64 } } as class ids 12 and 13, then whatever `more` appends.
fn with_element_id(more: impl FnOnce(B) -> B) -> Schema {
    let b = B::default()
        .class("ElementId")
        .u16(0)
        .u32(1)
        .u32(1)
        .field("m_id", [0x0e, 0, 0, 0])
        .u16(0x800d)
        .class("Identifier")
        .u16(0)
        .u32(2)
        .u32(1)
        .field("m_id64", [0x0b, 0, 0, 0])
        .u32(0)
        .u32(0);
    Schema::parse(&more(b).0).unwrap()
}

#[test]
fn class_ids_are_serial_from_12_in_order_of_first_appearance() {
    // A has inline parent B (flagged id 13), so C is 14 although it is the second top-level record.
    let b = B::default()
        .class("A")
        .u16(0x800d)
        .class("B")
        .u16(0)
        .u32(7)
        .u32(0)
        .u32(0)
        .u32(3)
        .u32(0)
        .u32(0)
        .class("C")
        .u16(13)
        .u32(1)
        .u32(0)
        .u32(0);
    let s = Schema::parse(&b.0).unwrap();
    assert_eq!(s.id_of("A"), Some(12));
    assert_eq!(s.id_of("B"), Some(13));
    assert_eq!(s.id_of("C"), Some(14));
    assert_eq!(s.class(12).unwrap().parent, 13);
    assert_eq!(s.class(14).unwrap().parent, 13);
    assert_eq!(s.class(13).unwrap().version, 7);
}

#[test]
fn guid_list_of_a_class_is_skipped() {
    let b = B::default().class("A").u16(0).u32(0).u32(0).u32(2).raw(&[9u8; 32]).class("B").u16(0).u32(0).u32(0).u32(0);
    let s = Schema::parse(&b.0).unwrap();
    assert_eq!(s.id_of("B"), Some(13));
}

#[test]
fn zero_name_length_ends_the_schema() {
    let b = B::default().class("A").u16(0).u32(0).u32(0).u32(0).u16(0).u16(0).u16(0);
    let s = Schema::parse(&b.0).unwrap();
    assert_eq!(s.len(), 1);
}

#[test]
fn garbage_schema_is_an_error_not_a_panic() {
    assert!(Schema::parse(&[0, 0, 0xff, 0xff, 1, 2, 3]).is_err());
    let truncated = B::default().class("A").u16(0).u32(0).u32(5);
    assert!(Schema::parse(&truncated.0).is_err());
}

#[test]
fn fields_decode_root_class_first_and_lengths_are_exact() {
    let s = with_element_id(|b| {
        b.class("Base").u16(0).u32(0).u32(1).field("m_flag", [1, 0, 0, 0]).u32(0)
            .class("Leaf").u16(14).u32(0).u32(2)
            .field("m_name", [8, 0x60, 0, 0])
            .field("m_v", [7, 0x10, 0, 0]).u32(2)
            .u32(0)
    });
    let leaf = s.id_of("Leaf").unwrap();
    let mut body = vec![1u8];
    body.extend_from_slice(&2u32.to_le_bytes());
    body.extend_from_slice(&[b'h', 0, b'i', 0]);
    body.extend_from_slice(&1.5f64.to_le_bytes());
    body.extend_from_slice(&(-2.0f64).to_le_bytes());
    let d = decode(&s, None, &body, 0, leaf).unwrap();
    assert_eq!(d.end, body.len());
    let r = d.root();
    assert_eq!(r.class(), "Leaf");
    assert_eq!(r.bool("m_flag"), Some(true));
    assert_eq!(r.str("m_name"), Some("hi"));
    assert_eq!(r.f64s("m_v"), Some(vec![1.5, -2.0]));
}

#[test]
fn element_id_values_collapse_to_the_id() {
    let s = with_element_id(|b| b.class("X").u16(0).u32(0).u32(1).field("m_ref", [0x0e, 0, 0, 0]).u16(12).u32(0));
    let body = (-7i64).to_le_bytes();
    let d = decode(&s, None, &body, 0, s.id_of("X").unwrap()).unwrap();
    assert_eq!(d.root().id("m_ref"), Some(-7));
}

#[test]
fn thirty_two_bit_identifiers_are_read_as_four_bytes() {
    let b = B::default()
        .class("ElementId").u16(0).u32(1).u32(1).field("m_id", [0x0e, 0, 0, 0]).u16(0x800d)
        .class("Identifier").u16(0).u32(1).u32(1).field("m_id", [4, 0, 0, 0]).u32(0).u32(0)
        .class("X").u16(0).u32(0).u32(1).field("m_ref", [0x0e, 0, 0, 0]).u16(12).u32(0);
    let s = Schema::parse(&b.0).unwrap();
    let body = 42i32.to_le_bytes();
    let d = decode(&s, None, &body, 0, s.id_of("X").unwrap()).unwrap();
    assert_eq!(d.end, 4);
    assert_eq!(d.root().id("m_ref"), Some(42));
}

/// Root with two owned pointers; the first target holds a third pointer.
fn pointer_schema() -> Schema {
    with_element_id(|b| {
        b.class("Inner").u16(0).u32(0).u32(1).field("m_i", [4, 0, 0, 0]).u32(0)
            .class("Holder").u16(0).u32(0).u32(1).field("m_next", [0x0e, 1, 0, 0]).u32(0)
            .class("Root").u16(0).u32(0).u32(3)
            .field("m_a", [0x0e, 1, 0, 0])
            .field("m_b", [0x0e, 1, 0, 0])
            .field("m_null", [0x0e, 2, 0, 0])
            .u32(0)
    })
}

#[test]
fn pointer_targets_follow_the_object_in_first_in_first_out_order() {
    let s = pointer_schema();
    let (inner, holder, root) = (s.id_of("Inner").unwrap(), s.id_of("Holder").unwrap(), s.id_of("Root").unwrap());
    let mut b = Vec::new();
    b.extend_from_slice(&u32::MAX.to_le_bytes()); // m_a -> Holder
    b.extend_from_slice(&holder.to_le_bytes());
    b.extend_from_slice(&5u32.to_le_bytes()); // m_b -> Inner, numbered object
    b.extend_from_slice(&inner.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes()); // m_null, 4 bytes without class
    b.extend_from_slice(&u32::MAX.to_le_bytes()); // Holder.m_next -> Inner, queued after m_b's target
    b.extend_from_slice(&inner.to_le_bytes());
    b.extend_from_slice(&111i32.to_le_bytes()); // m_b's target comes first
    b.extend_from_slice(&222i32.to_le_bytes()); // then Holder.m_next's
    let d = decode(&s, None, &b, 0, root).unwrap();
    assert_eq!(d.end, b.len());
    let r = d.root();
    assert_eq!(r.ptr("m_b").unwrap().int("m_i"), Some(111));
    assert_eq!(r.ptr("m_a").unwrap().ptr("m_next").unwrap().int("m_i"), Some(222));
    assert!(r.ptr("m_null").is_none());
}

#[test]
fn back_pointers_are_four_bytes_and_queue_nothing() {
    let s = with_element_id(|b| b.class("X").u16(0).u32(0).u32(2).field("m_doc", [0x0e, 3, 0, 0]).field("m_i", [4, 0, 0, 0]).u32(0));
    let mut b = 1u32.to_le_bytes().to_vec();
    b.extend_from_slice(&9i32.to_le_bytes());
    let d = decode(&s, None, &b, 0, s.id_of("X").unwrap()).unwrap();
    assert_eq!(d.end, 8);
    assert_eq!(d.root().int("m_i"), Some(9));
    assert!(matches!(d.root().get("m_doc"), Some(Value::Back(1))));
}

#[test]
fn nested_array_of_arrays_keeps_its_shape_and_repeats_a_class_reference_in_the_schema() {
    // (ElementId[2])[2]: the class reference is written twice in the type.
    let s = with_element_id(|b| {
        b.class("X").u16(0).u32(0).u32(1)
            .field("m_ids", [0x0d, 0x10, 0, 0]).u32(2).u32(1).raw(&[0x20]).raw(&[0x0e, 0x10, 0, 0]).u32(2).u16(12).u16(12)
            .u32(0)
    });
    let mut b = Vec::new();
    for v in [1i64, 2, 3, 4] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let d = decode(&s, None, &b, 0, s.id_of("X").unwrap()).unwrap();
    assert_eq!(d.end, 32);
    let Some(Value::List(rows)) = d.root().get("m_ids") else { panic!("list") };
    assert_eq!(rows.len(), 2);
    assert!(matches!(&rows[1], Value::List(c) if matches!(c[1], Value::Id(4))));
}

#[test]
fn vector_count_is_read_and_truncated_body_is_an_error() {
    let s = with_element_id(|b| b.class("X").u16(0).u32(0).u32(1).field("m_v", [4, 0x50, 0, 0]).u32(0));
    let x = s.id_of("X").unwrap();
    let mut b = 2u32.to_le_bytes().to_vec();
    b.extend_from_slice(&1i32.to_le_bytes());
    b.extend_from_slice(&2i32.to_le_bytes());
    assert_eq!(decode(&s, None, &b, 0, x).unwrap().end, 12);
    assert!(decode(&s, None, &b[..10], 0, x).is_err());
    assert!(decode(&s, None, &u32::MAX.to_le_bytes(), 0, x).is_err());
}

#[test]
fn entity_blob_is_decoded_by_its_storage_schema_in_entry_order() {
    let s = with_element_id(|b| {
        b.class("ESEntity").u16(0).u32(0).u32(1).field("m_blob", [0x0e, 1, 0, 0]).u32(0)
            .class("X").u16(0).u32(0).u32(1).field("m_e", [0x0e, 0, 0, 0]).u16(14).u32(0)
    });
    let guid = [7u8; 16];
    let field = |name: &'static str, type_name: &'static str, entry_index, container| EsField { name: name.into(), type_name: type_name.into(), entry_index, container };
    let es = EsSchemas::new(vec![(
        guid,
        EsSchema { name: "S".into(), fields: vec![field("second", "int", 1, 1), field("first", "TCHAR", 0, 0)] },
    )]);
    let mut b = u32::MAX.to_le_bytes().to_vec();
    b.extend_from_slice(&guid);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&[b'z', 0]);
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&6i32.to_le_bytes());
    let d = decode(&s, Some(&es), &b, 0, s.id_of("X").unwrap()).unwrap();
    assert_eq!(d.end, b.len());
    let e = d.root().child("m_e").unwrap().ptr("m_blob").unwrap();
    assert_eq!(e.class(), "S");
    assert_eq!(e.str("first"), Some("z"));
    assert!(matches!(e.get("second"), Some(Value::List(v)) if matches!(v[0], Value::Int(6))));
    // without storage schemas the blob's size is unknown
    assert!(decode(&s, None, &b, 0, s.id_of("X").unwrap()).is_err());
}
