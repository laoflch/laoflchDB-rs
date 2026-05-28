use laoflchDB_rust::{
    generate_table_uuid, generate_column_uuid,
    META_COL_PREFIX, META_TABLE_PREFIX,
};

#[test]
fn test_generate_table_uuid_deterministic() {
    let uuid1 = generate_table_uuid("user");
    let uuid2 = generate_table_uuid("user");
    assert_eq!(uuid1, uuid2);
    assert_eq!(
        uuid1.to_string(),
        "9e556479-7003-5916-9cd6-33f4227cec9b"
    );
}

#[test]
fn test_generate_column_uuid_deterministic() {
    let uuid1 = generate_column_uuid("user_id");
    let uuid2 = generate_column_uuid("user_id");
    assert_eq!(uuid1, uuid2);
    assert_eq!(
        uuid1.to_string(),
        "df7021ed-6e66-5581-bd69-d4e9ac1e5ada"
    );

    let pwd_uuid = generate_column_uuid("password");
    assert_eq!(
        pwd_uuid.to_string(),
        "f5c7086f-320b-5b93-bcdc-a2296adbec02"
    );
}

#[test]
fn test_table_meta_key_format() {
    let table_name = "user";
    let table_id = generate_table_uuid(table_name).to_string();
    let key = format!("{}{}_{}", META_TABLE_PREFIX, table_id, table_name);

    assert!(key.starts_with("META-TABLE_"));
    assert!(key.contains('_'));
    assert!(key.ends_with("_user"));
}

#[test]
fn test_column_meta_key_format() {
    let table_id = generate_table_uuid("user").to_string();
    let col_id = generate_column_uuid("user_id").to_string();
    let key = format!("{}{}_{}_{}", META_COL_PREFIX, &table_id, &col_id, "user_id");

    assert!(key.starts_with("META-COL_"));
    assert_eq!(key.matches('_').count(), 4);
    assert!(key.ends_with("_user_id"));
}
