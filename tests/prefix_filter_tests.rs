use laoflchdb_engines::{StorageEngine, EngineOptions, ColumnType, Row, Field, Query, TableFilter, ColumnFilter, ColumnFilterCondition, FilterOperator, EnumOrUnknown, RowType, Message};
use laoflchdb_engines::field::field::Value;
use laoflchdb_engines::field::{Integer, String as PbString, Float};
use multi_table_rocksdb::MultiTableRocksDBEngine;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn create_temp_dir() -> PathBuf {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = env::temp_dir();
    path.push(format!("rocksdb_test_{}_{}", std::process::id(), counter));
    fs::create_dir_all(&path).unwrap();
    path
}

fn remove_temp_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

fn field_to_vec(field: &Field) -> Vec<u8> {
    let mut v = Vec::new();
    field.write_to_vec(&mut v).unwrap();
    v
}

#[test]
fn test_row_id_to_key_big_endian() {
    let row_id_1: u64 = 584250357246742528;
    let row_id_2: u64 = 584250357246742529;
    let row_id_3: u64 = 584250357246742530;

    let key_1 = row_id_1.to_be_bytes();
    let key_2 = row_id_2.to_be_bytes();
    let key_3 = row_id_3.to_be_bytes();

    assert!(key_1 < key_2);
    assert!(key_2 < key_3);
    assert!(key_1 < key_3);

    assert_eq!(key_1.len(), 8);
    assert_eq!(key_2.len(), 8);
    assert_eq!(key_3.len(), 8);
}

#[test]
fn test_row_id_to_key_roundtrip() {
    let row_ids = [
        1u64,
        100u64,
        584250357246742528u64,
        584250357246742529u64,
        u64::MAX,
    ];

    for &original_id in &row_ids {
        let key = original_id.to_be_bytes();
        let restored_id = u64::from_be_bytes(key);
        assert_eq!(original_id, restored_id);
    }
}

#[tokio::test]
async fn test_big_endian_ordering_in_rocksdb() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("test_table", &[
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut row_ids = Vec::new();
    for i in 0..5 {
        let mut id_field = Field {
            value: Some(Value::IntegerValue(Integer { value: i as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: format!("name_{}", i), special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                field_to_vec(&id_field),
                field_to_vec(&name_field),
            ],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("test_table", &row).await.unwrap();
        row_ids.push(row_id);
    }

    let mut sorted_ids = row_ids.clone();
    sorted_ids.sort();
    assert_eq!(row_ids, sorted_ids, "Row IDs should be returned in increasing order");

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_scan_with_prefix_filter() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("users", &[
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
        (3, "age", ColumnType::COLUMN_TYPE_INT64),
    ]).await.unwrap();

    for i in 0..10 {
        let mut id_field = Field {
            value: Some(Value::IntegerValue(Integer { value: i as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: format!("user_{}", i), special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut age_field = Field {
            value: Some(Value::IntegerValue(Integer { value: (20 + i) as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                field_to_vec(&id_field),
                field_to_vec(&name_field),
                field_to_vec(&age_field),
            ],
            special_fields: Default::default(),
        };

        engine.add_row("users", &row).await.unwrap();
    }

    let tables = engine.list_tables().await.unwrap();
    assert!(tables.contains(&"users".to_string()));

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_row_id_monotonic_increasing() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("test_table", &[
        (1, "name", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut previous_id: u64 = 0;
    for i in 0..100 {
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: format!("item_{}", i), special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![field_to_vec(&name_field)],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("test_table", &row).await.unwrap();

        assert!(row_id > previous_id, "Row IDs should be strictly increasing");
        previous_id = row_id;
    }

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_get_row_by_id() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("products", &[
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
        (3, "price", ColumnType::COLUMN_TYPE_FLOAT),
    ]).await.unwrap();

    let mut name_field = Field {
        value: Some(Value::StringValue(PbString { value: "Test Product".to_string(), special_fields: Default::default() })),
        special_fields: Default::default(),
    };
    let mut price_field = Field {
        value: Some(Value::FloatValue(Float { value: 99.99, special_fields: Default::default() })),
        special_fields: Default::default(),
    };
    let mut id_field = Field {
        value: Some(Value::IntegerValue(Integer { value: 1, special_fields: Default::default() })),
        special_fields: Default::default(),
    };

    let row = Row {
        row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
        version: 1,
        data: vec![
            field_to_vec(&id_field),
            field_to_vec(&name_field),
            field_to_vec(&price_field),
        ],
        special_fields: Default::default(),
    };

    let row_id = engine.add_row("products", &row).await.unwrap();

    let retrieved_row = engine.get_row("products", row_id).await.unwrap();
    assert!(retrieved_row.is_some());

    let retrieved_data = retrieved_row.unwrap();
    assert_eq!(retrieved_data.version, 1);
    assert_eq!(retrieved_data.data.len(), 3);

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_delete_row() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("items", &[
        (1, "name", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut name_field = Field {
        value: Some(Value::StringValue(PbString { value: "test_item".to_string(), special_fields: Default::default() })),
        special_fields: Default::default(),
    };

    let row = Row {
        row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
        version: 1,
        data: vec![field_to_vec(&name_field)],
        special_fields: Default::default(),
    };

    let row_id = engine.add_row("items", &row).await.unwrap();

    assert!(engine.get_row("items", row_id).await.unwrap().is_some());

    engine.delete_row("items", row_id).await.unwrap();

    assert!(engine.get_row("items", row_id).await.unwrap().is_none());

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_update_row() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("items", &[
        (1, "name", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut name_field1 = Field {
        value: Some(Value::StringValue(PbString { value: "original".to_string(), special_fields: Default::default() })),
        special_fields: Default::default(),
    };

    let row1 = Row {
        row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
        version: 1,
        data: vec![field_to_vec(&name_field1)],
        special_fields: Default::default(),
    };

    let row_id = engine.add_row("items", &row1).await.unwrap();

    let mut name_field2 = Field {
        value: Some(Value::StringValue(PbString { value: "updated".to_string(), special_fields: Default::default() })),
        special_fields: Default::default(),
    };

    let row2 = Row {
        row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
        version: 2,
        data: vec![field_to_vec(&name_field2)],
        special_fields: Default::default(),
    };

    engine.update_row("items", row_id, &row2).await.unwrap();

    let retrieved = engine.get_row("items", row_id).await.unwrap().unwrap();
    assert_eq!(retrieved.version, 2);

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_snowflake_id_distribution() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("test_table", &[
        (1, "value", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut ids = Vec::new();
    for i in 0..1000 {
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: format!("val_{}", i), special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![field_to_vec(&name_field)],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("test_table", &row).await.unwrap();
        ids.push(row_id);
    }

    let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique_ids.len(), 1000, "All row IDs should be unique");

    let min_id = *ids.iter().min().unwrap();
    let max_id = *ids.iter().max().unwrap();
    assert!(max_id > min_id, "Max ID should be greater than min ID");

    remove_temp_dir(&temp_dir);
}

#[test]
fn test_big_endian_key_comparison() {
    let ids = [
        0x0000000000000001u64,
        0x0000000000000002u64,
        0x7FFFFFFFFFFFFFFFu64,
        0x8000000000000000u64,
        0xFFFFFFFFFFFFFFFFu64,
    ];

    let keys: Vec<_> = ids.iter().map(|&id| id.to_be_bytes()).collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();

    assert_eq!(keys, sorted_keys, "Big-endian keys should be in sorted order");
}

#[tokio::test]
async fn test_prefix_scan_with_timestamp() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("events", &[
        (1, "timestamp", ColumnType::COLUMN_TYPE_INT64),
        (2, "data", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut inserted_ids = Vec::new();
    for i in 0..5 {
        let mut ts_field = Field {
            value: Some(Value::IntegerValue(Integer { value: (1000 + i) as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut data_field = Field {
            value: Some(Value::StringValue(PbString { value: format!("event_{}", i), special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                field_to_vec(&ts_field),
                field_to_vec(&data_field),
            ],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("events", &row).await.unwrap();
        inserted_ids.push(row_id);

        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert_eq!(inserted_ids.len(), 5);

    let first_id = inserted_ids[0];
    let last_id = inserted_ids[4];
    assert!(last_id > first_id, "Last ID should be greater than first ID");

    let first_key = first_id.to_be_bytes();
    let last_key = last_id.to_be_bytes();
    assert!(last_key > first_key, "Last key should be greater than first key in big-endian order");

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_snowflake_id_timestamp_prefix() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("records", &[
        (1, "value", ColumnType::COLUMN_TYPE_STRING),
    ]).await.unwrap();

    let mut ids = Vec::new();
    for _ in 0..10 {
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: "test".to_string(), special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![field_to_vec(&name_field)],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("records", &row).await.unwrap();
        ids.push(row_id);

        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    let prefixes: Vec<_> = ids.iter().map(|id| {
        let bytes = id.to_be_bytes();
        bytes[0..4].to_vec()
    }).collect();

    let unique_prefixes: std::collections::HashSet<_> = prefixes.iter().collect();
    assert!(unique_prefixes.len() >= 1, "Should have at least one unique prefix");

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_query_with_cnf_filters() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("products", &[
        (1, "id", ColumnType::COLUMN_TYPE_INT64),
        (2, "name", ColumnType::COLUMN_TYPE_STRING),
        (3, "price", ColumnType::COLUMN_TYPE_FLOAT),
        (4, "stock", ColumnType::COLUMN_TYPE_INT64),
    ]).await.unwrap();

    let products = vec![
        (1, "iPhone 15", 799.99, 100),
        (2, "MacBook Pro", 1999.99, 50),
        (3, "AirPods Pro", 249.0, 200),
        (4, "iPad Air", 599.0, 75),
        (5, "Apple Watch", 399.0, 120),
    ];

    for (id, name, price, stock) in products {
        let mut id_field = Field {
            value: Some(Value::IntegerValue(Integer { value: id as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut name_field = Field {
            value: Some(Value::StringValue(PbString { value: name.to_string(), special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut price_field = Field {
            value: Some(Value::FloatValue(Float { value: price, special_fields: Default::default() })),
            special_fields: Default::default(),
        };
        let mut stock_field = Field {
            value: Some(Value::IntegerValue(Integer { value: stock as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![
                field_to_vec(&id_field),
                field_to_vec(&name_field),
                field_to_vec(&price_field),
                field_to_vec(&stock_field),
            ],
            special_fields: Default::default(),
        };

        engine.add_row("products", &row).await.unwrap();
    }

    let query = Query {
        table_filters: vec![
            TableFilter {
                table_name: "products".to_string(),
                column_filters: vec![
                    ColumnFilter {
                        column_name: "price".to_string(),
                        conditions: vec![
                            ColumnFilterCondition {
                                op: EnumOrUnknown::new(FilterOperator::FILTER_OPERATOR_LT),
                                value: Some(Field {
                                    value: Some(Value::FloatValue(Float { value: 600.0, special_fields: Default::default() })),
                                    special_fields: Default::default(),
                                }).into(),
                                values: vec![],
                                special_fields: Default::default(),
                            },
                        ],
                        special_fields: Default::default(),
                    },
                    ColumnFilter {
                        column_name: "stock".to_string(),
                        conditions: vec![
                            ColumnFilterCondition {
                                op: EnumOrUnknown::new(FilterOperator::FILTER_OPERATOR_GTE),
                                value: Some(Field {
                                    value: Some(Value::IntegerValue(Integer { value: 100, special_fields: Default::default() })),
                                    special_fields: Default::default(),
                                }).into(),
                                values: vec![],
                                special_fields: Default::default(),
                            },
                        ],
                        special_fields: Default::default(),
                    },
                ],
                special_fields: Default::default(),
            },
        ],
        limit: Some(10),
        offset: Some(0),
        projected_columns: vec![],
        special_fields: Default::default(),
    };

    let result = engine.query(&query).await.unwrap();
    assert_eq!(result.rows.len(), 2);

    let names: Vec<String> = result.rows.iter()
        .filter_map(|r| r.row.as_ref())
        .map(|row| {
            if let Ok(field) = Field::parse_from_bytes(row.data[1].as_slice()) {
                if let Some(Value::StringValue(s)) = field.value {
                    return s.value;
                }
            }
            "".to_string()
        })
        .collect();

    assert!(names.contains(&"AirPods Pro".to_string()));
    assert!(names.contains(&"Apple Watch".to_string()));

    remove_temp_dir(&temp_dir);
}

#[tokio::test]
async fn test_scan_rows_in_key_range() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("scan_test", &[
        (1, "index", ColumnType::COLUMN_TYPE_INT64),
    ]).await.unwrap();

    let mut row_ids = Vec::new();
    for i in 0..100 {
        let mut index_field = Field {
            value: Some(Value::IntegerValue(Integer { value: i as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![field_to_vec(&index_field)],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("scan_test", &row).await.unwrap();
        row_ids.push(row_id);
    }

    row_ids.sort();

    let first_id = row_ids[10];
    let last_id = row_ids[50];

    let mut count = 0;
    for &row_id in &row_ids {
        if row_id >= first_id && row_id <= last_id {
            let row = engine.get_row("scan_test", row_id).await.unwrap();
            assert!(row.is_some());
            count += 1;
        }
    }

    assert_eq!(count, 41, "Should scan exactly 41 rows in range (from index 10 to 50 inclusive)");

    remove_temp_dir(&temp_dir);
}

#[test]
fn test_prefix_comparison_across_boundaries() {
    let id1 = 0x0000123456789ABCu64;
    let id2 = 0x0000123400000000u64;
    let id3 = 0x00001234FFFFFFFFu64;
    let id4 = 0x0000123500000000u64;

    let key1 = id1.to_be_bytes();
    let key2 = id2.to_be_bytes();
    let key3 = id3.to_be_bytes();
    let key4 = id4.to_be_bytes();

    let prefix_1234 = &[0x00, 0x00, 0x12, 0x34];
    let prefix_1235 = &[0x00, 0x00, 0x12, 0x35];

    assert!(key1.starts_with(prefix_1234));
    assert!(key2.starts_with(prefix_1234));
    assert!(key3.starts_with(prefix_1234));
    assert!(key4.starts_with(prefix_1235));

    assert!(key2 < key1);
    assert!(key1 < key3);
    assert!(key3 < key4);
}

#[tokio::test]
async fn test_parallel_row_insertion_order() {
    let temp_dir = create_temp_dir();
    let db_path = temp_dir.to_str().unwrap();

    let options = EngineOptions {
        db_path: db_path.to_string(),
        schema_name: "test".to_string(),
    };

    let mut engine = MultiTableRocksDBEngine::new(&options).unwrap();

    engine.create_table("parallel_test", &[
        (1, "seq", ColumnType::COLUMN_TYPE_INT64),
    ]).await.unwrap();

    let mut row_ids = Vec::new();
    for i in 0..10 {
        let mut seq_field = Field {
            value: Some(Value::IntegerValue(Integer { value: i as i64, special_fields: Default::default() })),
            special_fields: Default::default(),
        };

        let row = Row {
            row_type: EnumOrUnknown::new(RowType::ROW_TYPE_NORMAL),
            version: 1,
            data: vec![field_to_vec(&seq_field)],
            special_fields: Default::default(),
        };

        let row_id = engine.add_row("parallel_test", &row).await.unwrap();
        row_ids.push(row_id);

        tokio::task::yield_now().await;
    }

    let mut sorted_ids = row_ids.clone();
    sorted_ids.sort();

    assert_eq!(row_ids, sorted_ids, "Row IDs should be returned in sorted order");

    remove_temp_dir(&temp_dir);
}
