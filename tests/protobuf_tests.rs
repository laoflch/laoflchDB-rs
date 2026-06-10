use laoflchdb_engines::{TableMeta, ColumnMeta, ColumnType, SpecialFields, Message};

#[test]
fn test_protobuf_table_meta_encode_decode() {
    let table_id: u64 = 1000;
    let original = TableMeta {
        table_id,
        table_name: "user".to_string(),
        comment: "test table".to_string(),
        column_count: 2,
        next_auto_inc_column_id: 3,
        special_fields: SpecialFields::default(),
    };

    let mut encoded = Vec::new();
    original.write_to_vec(&mut encoded).unwrap();
    assert!(!encoded.is_empty());

    let decoded = TableMeta::parse_from_bytes(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.table_name, "user");
    assert_eq!(decoded.column_count, 2);
    assert_eq!(decoded.next_auto_inc_column_id, 3);
}

#[test]
fn test_protobuf_column_meta_encode_decode() {
    let table_id: u64 = 1000;
    let original = ColumnMeta {
        table_id,
        column_id: 42,
        column_name: "user_id".to_string(),
        column_type: ColumnType::COLUMN_TYPE_INT64.into(),
        comment: "test column".to_string(),
        special_fields: SpecialFields::default(),
    };

    let mut encoded = Vec::new();
    original.write_to_vec(&mut encoded).unwrap();
    assert!(!encoded.is_empty());

    let decoded = ColumnMeta::parse_from_bytes(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.column_id, 42);
    assert_eq!(decoded.column_name, "user_id");
    assert_eq!(
        decoded.column_type.enum_value(),
        Ok(ColumnType::COLUMN_TYPE_INT64)
    );
}
