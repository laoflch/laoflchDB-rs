use multi_table_rocksdb::pb as mt_pb;
use prost::Message;

#[test]
fn test_protobuf_table_meta_encode_decode() {
    use mt_pb::TableMeta;

    let table_id: u64 = 1000;
    let original = TableMeta {
        table_id,
        table_name: "user".to_string(),
        column_count: 2,
        next_auto_inc_column_id: 3,
    };

    let encoded = original.encode_to_vec();
    assert!(!encoded.is_empty());

    let decoded = TableMeta::decode(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.table_name, "user");
    assert_eq!(decoded.column_count, 2);
    assert_eq!(decoded.next_auto_inc_column_id, 3);
}

#[test]
fn test_protobuf_column_meta_encode_decode() {
    use mt_pb::{ColumnMeta, ColumnType};

    let table_id: u64 = 1000;
    let original = ColumnMeta {
        table_id,
        column_id: 42,
        column_name: "user_id".to_string(),
        column_type: ColumnType::Int64.into(),
    };

    let encoded = original.encode_to_vec();
    assert!(!encoded.is_empty());

    let decoded = ColumnMeta::decode(&encoded[..]).unwrap();
    assert_eq!(decoded.table_id, table_id);
    assert_eq!(decoded.column_id, 42);
    assert_eq!(decoded.column_name, "user_id");
    assert_eq!(
        ColumnType::try_from(decoded.column_type),
        Ok(ColumnType::Int64)
    );
}
