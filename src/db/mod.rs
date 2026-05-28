use crate::pb::{ColumnMeta, ColumnType, TableMeta};
use log::info;
use prost::Message;
use rocksdb::{ColumnFamilyDescriptor, IteratorMode, Options, DB};
use uuid::Uuid;

pub const META_TABLE_PREFIX: &str = "META-TABLE_";
pub const META_COL_PREFIX: &str = "META-COL_";

pub const LAOFLCHDB_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1,
    0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8
]);

pub fn generate_table_uuid(table_name: &str) -> Uuid {
    Uuid::new_v5(&LAOFLCHDB_NAMESPACE, table_name.as_bytes())
}

pub fn generate_column_uuid(column_name: &str) -> Uuid {
    Uuid::new_v5(&LAOFLCHDB_NAMESPACE, column_name.as_bytes())
}

pub struct OltpDB {
    db: DB,
}

impl OltpDB {
    pub fn open(path: &str) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let existing_cfs = DB::list_cf(&opts, path).unwrap_or_default();
        let mut cf_descriptors = Vec::new();
        for cf_name in existing_cfs {
            cf_descriptors.push(ColumnFamilyDescriptor::new(cf_name, Options::default()));
        }
        if cf_descriptors.is_empty() {
            cf_descriptors.push(ColumnFamilyDescriptor::new("default", Options::default()));
        }

        let db = DB::open_cf_descriptors(&opts, path, cf_descriptors)
            .expect("无法打开数据库");

        OltpDB { db }
    }

    pub fn init_laoflch_db(&mut self) {
        self.create_table("user", &[("user_id", ColumnType::Int64), ("password", ColumnType::String)]);
    }

    pub fn create_table(&mut self, table_name: &str, columns: &[(&str, ColumnType)]) -> String {
        let table_id = generate_table_uuid(table_name);
        let table_id_str = table_id.to_string();
        let column_count = columns.len() as u32;

        let table_meta = TableMeta {
            table_id: table_id_str.clone(),
            table_name: table_name.to_string(),
            column_count,
        };

        let table_key = format!("{}{}_{}", META_TABLE_PREFIX, table_id_str, table_name);
        let table_value = table_meta.encode_to_vec();
        let cf = self.db.cf_handle("default").unwrap();
        self.db.put_cf(&cf, table_key.as_bytes(), table_value).expect("写入表元数据失败");
        info!("创建表元数据: key={}, table_uuid={}", table_key, table_id_str);

        for (col_name, col_type) in columns.iter() {
            let column_id = generate_column_uuid(col_name);
            let column_id_str = column_id.to_string();
            let column_meta = ColumnMeta {
                table_id: table_id_str.clone(),
                column_id: column_id_str.clone(),
                column_name: col_name.to_string(),
                column_type: (*col_type).into(),
            };

            let col_key = format!("{}{}_{}_{}", META_COL_PREFIX, table_id_str, column_id_str, col_name);
            let col_value = column_meta.encode_to_vec();
            self.db.put_cf(&cf, col_key.as_bytes(), col_value).expect("写入字段元数据失败");
            info!("创建字段元数据: key={}", col_key);
        }

        let cf_opts = Options::default();
        self.db.create_cf(table_name, &cf_opts).expect("创建表 CF (Column Family) 失败");
        info!("创建表 CF: {}", table_name);

        table_id_str
    }

    pub fn print_metadata(&self) {
        info!("");
        info!("=== 数据库元数据 ===");
        let cf = self.db.cf_handle("default").unwrap();
        info!("[default CF] 遍历所有元数据 key-value:");
        for item in self.db.iterator_cf(&cf, IteratorMode::Start) {
            if let Ok((key, value)) = item {
                let key_str = String::from_utf8_lossy(&key);
                info!("  找到 key: {}", key_str);
                if key_str.starts_with(META_TABLE_PREFIX) {
                    match TableMeta::decode(&value[..]) {
                        Ok(meta) => info!("    TableMeta -> table_id={}, name={}, columns={}",
                            meta.table_id, meta.table_name, meta.column_count),
                        Err(_) => info!("    解码 TableMeta protobuf 失败"),
                    }
                } else if key_str.starts_with(META_COL_PREFIX) {
                    match ColumnMeta::decode(&value[..]) {
                        Ok(meta) => info!("    ColumnMeta -> table_id={}, col_id={}, name={}, type={:?}",
                            meta.table_id, meta.column_id, meta.column_name, ColumnType::try_from(meta.column_type)),
                        Err(_) => info!("    解码 ColumnMeta protobuf 失败"),
                    }
                }
            }
        }
        info!("");
        info!("UUID v5 namespace: {}", LAOFLCHDB_NAMESPACE);
        info!("  'user' -> {}", generate_table_uuid("user"));
        info!("  'user_id' -> {}", generate_column_uuid("user_id"));
        info!("  'password' -> {}", generate_column_uuid("password"));
    }

    pub fn put_kv(&self, table_name: &str, key: &[u8], value: &[u8]) -> Result<(), String> {
        match self.db.cf_handle(table_name) {
            Some(cf) => {
                self.db.put_cf(&cf, key, value)
                    .map_err(|e| format!("put error: {}", e))
            }
            None => Err(format!("table '{}' not found", table_name)),
        }
    }

    pub fn get_kv(&self, table_name: &str, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        match self.db.cf_handle(table_name) {
            Some(cf) => {
                self.db.get_cf(&cf, key)
                    .map_err(|e| format!("get error: {}", e))
            }
            None => Err(format!("table '{}' not found", table_name)),
        }
    }

    pub fn delete_kv(&self, table_name: &str, key: &[u8]) -> Result<(), String> {
        match self.db.cf_handle(table_name) {
            Some(cf) => {
                self.db.delete_cf(&cf, key)
                    .map_err(|e| format!("delete error: {}", e))
            }
            None => Err(format!("table '{}' not found", table_name)),
        }
    }

    pub fn list_tables(&self) -> Vec<String> {
        self.db.cf_names()
            .into_iter()
            .map(|s| s.to_string())
            .filter(|s| s != "default")
            .collect()
    }

    pub fn raw_db(&self) -> &rocksdb::DB {
        &self.db
    }
}
