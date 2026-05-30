use laoflchDB_rust::{META_COLUMN_PREFIX, META_TABLE_PREFIX, MAX_TABLE_ID_LENGTH};

#[test]
fn test_table_meta_key_format() {
    let table_name = "user";
    let table_id: u64 = 0;
    let key = format!("{}:{}:{}", META_TABLE_PREFIX, table_name, table_id);

    assert!(key.starts_with("META-TABLE:"));
    assert!(key.contains(':'));
    assert!(key.ends_with(":0"));
}

#[test]
fn test_column_meta_key_format() {
    let table_id: u64 = 0;
    let col_id: u64 = 0;
    let formatted_table_id = format!("{:0>width$}", table_id, width = MAX_TABLE_ID_LENGTH);
    let key = format!("{}:{}:{}:{}:Int64", META_COLUMN_PREFIX, formatted_table_id, "user_id", col_id);

    assert!(key.starts_with("META-COL:"));
    assert!(key.contains(':'));
    assert!(key.ends_with(":Int64"));
    assert_eq!(formatted_table_id, "00000000000000000000");
}

#[test]
fn test_max_table_id_length() {
    assert_eq!(MAX_TABLE_ID_LENGTH, 20);
}
