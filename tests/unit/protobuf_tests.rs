use laoflchDB_rust::{generate_column_uuid, generate_table_uuid, pb};
use prost::Message;

#[test]
fn test_protobuf_table_meta_encode_decode() {
    use pb::TableMeta;

    let table_id = generate_table_uuid("user").to_string();
    let original = TableMeta {
        table_id: table_id.clone(),
        table_name: "user".to_string(),
        column_count: 2,
    };

    let encoded = original.encode_to_vec();
    assert!(!encoded.is_empty());

    let decoded = TableMeta::decode(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.table_name, "user");
    assert_eq!(decoded.column_count, 2);
}

#[test]
fn test_protobuf_column_meta_encode_decode() {
    use pb::{ColumnMeta, ColumnType};

    let table_id = generate_table_uuid("user").to_string();
    let col_id = generate_column_uuid("user_id").to_string();
    let original = ColumnMeta {
        table_id: table_id.clone(),
        column_id: col_id.clone(),
        column_name: "user_id".to_string(),
        column_type: ColumnType::Int64.into(),
    };

    let encoded = original.encode_to_vec();
    assert!(!encoded.is_empty());

    let decoded = ColumnMeta::decode(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.column_id, col_id);
    assert_eq!(decoded.column_name, "user_id");
    assert_eq!(
        ColumnType::try_from(decoded.column_type),
        Ok(ColumnType::Int64)
    );
}
