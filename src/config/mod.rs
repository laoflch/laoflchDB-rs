use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub enum PermissionAction {
    #[serde(rename = "get")]
    Get,
    #[serde(rename = "put")]
    Put,
    #[serde(rename = "delete")]
    Delete,
    #[serde(rename = "create_table")]
    CreateTable,
    #[serde(rename = "drop_table")]
    DropTable,
    #[serde(rename = "list_tables")]
    ListTables,
    #[serde(rename = "list_table_cols")]
    ListTableCols,
    #[serde(rename = "add_row")]
    AddRow,
    #[serde(rename = "get_row")]
    GetRow,
    #[serde(rename = "update_row")]
    UpdateRow,
    #[serde(rename = "delete_row")]
    DeleteRow,
    #[serde(rename = "get_all_meta")]
    GetAllMeta,
    #[serde(rename = "get_schema_info")]
    GetSchemaInfo,
    #[serde(rename = "get_table_meta")]
    GetTableMeta,
    #[serde(rename = "query")]
    Query,
    #[serde(rename = "*")]
    All,
}

impl std::fmt::Display for PermissionAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionAction::Get => write!(f, "get"),
            PermissionAction::Put => write!(f, "put"),
            PermissionAction::Delete => write!(f, "delete"),
            PermissionAction::CreateTable => write!(f, "create_table"),
            PermissionAction::DropTable => write!(f, "drop_table"),
            PermissionAction::ListTables => write!(f, "list_tables"),
            PermissionAction::ListTableCols => write!(f, "list_table_cols"),
            PermissionAction::AddRow => write!(f, "add_row"),
            PermissionAction::GetRow => write!(f, "get_row"),
            PermissionAction::UpdateRow => write!(f, "update_row"),
            PermissionAction::DeleteRow => write!(f, "delete_row"),
            PermissionAction::GetAllMeta => write!(f, "get_all_meta"),
            PermissionAction::GetSchemaInfo => write!(f, "get_schema_info"),
            PermissionAction::GetTableMeta => write!(f, "get_table_meta"),
            PermissionAction::Query => write!(f, "query"),
            PermissionAction::All => write!(f, "*"),
        }
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct TablePermissions {
    #[serde(default)]
    pub allowed_tables: Vec<String>,
    #[serde(default)]
    pub denied_tables: Vec<String>,
    #[serde(default)]
    pub allowed_schemas: Vec<String>,
    #[serde(default)]
    pub denied_schemas: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServicePermission {
    pub service_id: String,
    #[serde(default = "default_default_policy")]
    pub default_policy: String,
    #[serde(default)]
    pub allowed_actions: Vec<PermissionAction>,
    #[serde(default)]
    pub denied_actions: Vec<PermissionAction>,
    #[serde(default)]
    pub table_permissions: Option<TablePermissions>,
}

fn default_default_policy() -> String {
    "allow".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct AccessProtocolConfig {
    pub protocol: String,
    pub enabled: bool,
    #[serde(default)]
    pub addr: Option<String>,
    #[serde(default)]
    pub service_id: Option<String>,
    #[serde(default)]
    pub permissions: Option<ServicePermission>,
}

#[derive(Debug, Deserialize, Clone)]
pub enum RuntimeMode {
    #[serde(rename = "multi_thread")]
    MultiThread,
    #[serde(rename = "single_thread")]
    SingleThread,
}

impl Default for RuntimeMode {
    fn default() -> Self {
        RuntimeMode::MultiThread
    }
}

/// 向量化服务配置
#[derive(Debug, Deserialize, Clone)]
pub struct VectorServiceConfig {
    /// 是否启用向量化服务
    #[serde(default)]
    pub enabled: bool,
    /// 是否在启动时自动扫描加载模型
    #[serde(default = "default_vector_auto_load")]
    pub auto_load: bool,
    /// 指定启动时加载的模型名称列表（空列表表示加载所有有效模型）
    #[serde(default)]
    pub load_models: Vec<String>,
}

fn default_vector_auto_load() -> bool {
    true
}

/// 嵌入向量索引服务配置
#[derive(Debug, Deserialize, Clone)]
pub struct EmbeddingIndexConfig {
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
    /// 向量维度
    #[serde(default = "default_hnsw_dim")]
    pub dim: usize,
    /// HNSW max connections (M)
    #[serde(default = "default_hnsw_m")]
    pub m: u32,
    /// HNSW ef construction
    #[serde(default = "default_hnsw_ef_construction")]
    pub ef_construction: u32,
    /// HNSW ef search
    #[serde(default = "default_hnsw_ef_search")]
    pub ef_search: u32,
    /// 最大元素数
    #[serde(default = "default_hnsw_max_elements")]
    pub max_elements: usize,
    /// KV RocksDB 数据路径
    #[serde(default = "default_hnsw_kv_db_path")]
    pub kv_db_path: String,
    /// 图拓扑快照保存路径
    #[serde(default = "default_hnsw_snapshot_path")]
    pub snapshot_path: String,
}

fn default_hnsw_dim() -> usize { 512 }
fn default_hnsw_m() -> u32 { 32 }
fn default_hnsw_ef_construction() -> u32 { 200 }
fn default_hnsw_ef_search() -> u32 { 50 }
fn default_hnsw_max_elements() -> usize { 1_000_000 }
fn default_hnsw_kv_db_path() -> String { "./laoflch_hnsw_data".to_string() }
fn default_hnsw_snapshot_path() -> String { "./laoflch_hnsw_snapshots".to_string() }

impl Default for EmbeddingIndexConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dim: default_hnsw_dim(),
            m: default_hnsw_m(),
            ef_construction: default_hnsw_ef_construction(),
            ef_search: default_hnsw_ef_search(),
            max_elements: default_hnsw_max_elements(),
            kv_db_path: default_hnsw_kv_db_path(),
            snapshot_path: default_hnsw_snapshot_path(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    pub db_path: String,
    pub index_path: String,
    #[serde(default = "default_model_path")]
    pub model_path: String,
    #[serde(default = "default_addr")]
    pub addr: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub access_protocols: Vec<AccessProtocolConfig>,
    #[serde(default)]
    pub permissions: Option<Vec<ServicePermission>>,
    #[serde(default = "default_global_default_policy")]
    pub default_policy: String,
    #[serde(default)]
    pub runtime_mode: RuntimeMode,
    #[serde(default)]
    pub vector_service: Option<VectorServiceConfig>,
    #[serde(default)]
    pub embedding_index: Option<EmbeddingIndexConfig>,
    #[serde(default)]
    pub object_store: Option<ObjectStoreConfig>,
    #[serde(default)]
    pub image_service: Option<ImageServiceConfig>,
}

/// 对象存储服务配置（S3 兼容）
#[derive(Debug, Deserialize, Clone)]
pub struct ObjectStoreConfig {
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
    /// 对象存储数据路径
    #[serde(default = "default_object_store_db_path")]
    pub db_path: String,
}

/// 图片服务配置
#[derive(Debug, Deserialize, Clone)]
pub struct ImageServiceConfig {
    /// 是否启用（必须同时启用 object_store）
    #[serde(default)]
    pub enabled: bool,
    /// 默认 bucket 名称
    #[serde(default = "default_image_bucket")]
    pub default_bucket: String,
}

fn default_image_bucket() -> String {
    "images".to_string()
}

fn default_object_store_db_path() -> String {
    "./laoflch_object_store_data".to_string()
}

fn default_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_global_default_policy() -> String {
    "allow".to_string()
}

fn default_model_path() -> String {
    "./laoflch_db_model".to_string()
}

impl DatabaseConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("读取配置文件失败: {}", e))?;
        serde_yaml::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))
    }

    pub fn load_or_default() -> Self {
        let config_paths = [
            "./laoflchdb.yaml",
            "./config/laoflchdb.yaml",
            "/etc/laoflchdb.yaml",
        ];
        
        for path in config_paths {
            if Path::new(path).exists() {
                match Self::load_from_file(path) {
                    Ok(config) => {
                        log::info!("加载配置文件: {}", path);
                        return config;
                    }
                    Err(e) => {
                        log::warn!("配置文件解析失败 {}: {}", path, e);
                    }
                }
            }
        }
        
        Self::default()
    }

    pub fn default() -> Self {
        Self {
            db_path: "./laoflch_db_data".to_string(),
            index_path: "./laoflch_index_data".to_string(),
            model_path: "./laoflch_db_model".to_string(),
            addr: default_addr(),
            log_level: default_log_level(),
            access_protocols: vec![],
            permissions: None,
            default_policy: default_global_default_policy(),
            runtime_mode: RuntimeMode::default(),
            vector_service: None,
            embedding_index: None,
            object_store: None,
            image_service: None,
        }
    }

    pub fn get_service_permission(&self, service_id: &str) -> Option<ServicePermission> {
        if let Some(perms) = &self.permissions {
            for perm in perms {
                if perm.service_id == service_id {
                    return Some(perm.clone());
                }
            }
        }
        
        for protocol in &self.access_protocols {
            if let Some(ref permissions) = protocol.permissions {
                if permissions.service_id == service_id {
                    return Some(permissions.clone());
                }
            }
        }
        
        None
    }

    pub fn get_global_default_policy(&self) -> bool {
        self.default_policy.to_lowercase() == "allow"
    }

    pub fn get_service_ids(&self) -> Vec<String> {
        self.access_protocols.iter()
            .filter_map(|p| p.service_id.clone())
            .collect()
    }

    pub fn get_permission_service_ids(&self) -> Vec<String> {
        if let Some(ref perms) = self.permissions {
            perms.iter().map(|p| p.service_id.clone()).collect()
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = DatabaseConfig::default();
        assert_eq!(config.db_path, "./laoflch_db_data");
        assert_eq!(config.index_path, "./laoflch_index_data");
        assert_eq!(config.addr, "127.0.0.1:50051");
        assert_eq!(config.log_level, "info");
        assert_eq!(config.default_policy, "allow");
    }

    #[test]
    fn test_load_config_from_file() {
        let config_content = r#"
db_path: "./test_db"
addr: "127.0.0.1:12345"
log_level: "debug"
default_policy: "deny"
"#;
        
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "{}", config_content).unwrap();
        
        let config = DatabaseConfig::load_from_file(temp_file.path()).unwrap();
        assert_eq!(config.db_path, "./test_db");
        assert_eq!(config.index_path, "./test_index");
        assert_eq!(config.addr, "127.0.0.1:12345");
        assert_eq!(config.log_level, "debug");
        assert_eq!(config.default_policy, "deny");
    }

    #[test]
    fn test_get_global_default_policy() {
        let config_allow = DatabaseConfig {
            default_policy: "allow".to_string(),
            ..DatabaseConfig::default()
        };
        assert!(config_allow.get_global_default_policy());
        
        let config_deny = DatabaseConfig {
            default_policy: "deny".to_string(),
            ..DatabaseConfig::default()
        };
        assert!(!config_deny.get_global_default_policy());
    }

    #[test]
    fn test_get_service_ids() {
        let config = DatabaseConfig {
            access_protocols: vec![
                AccessProtocolConfig {
                    protocol: "grpc".to_string(),
                    enabled: true,
                    addr: Some("127.0.0.1:1234".to_string()),
                    service_id: Some("service1".to_string()),
                    permissions: None,
                },
                AccessProtocolConfig {
                    protocol: "rest".to_string(),
                    enabled: true,
                    addr: Some("127.0.0.1:5678".to_string()),
                    service_id: Some("service2".to_string()),
                    permissions: None,
                }
            ],
            ..DatabaseConfig::default()
        };
        let ids = config.get_service_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"service1".to_string()));
        assert!(ids.contains(&"service2".to_string()));
    }

    #[test]
    fn test_get_service_permission_from_permissions() {
        let config = DatabaseConfig {
            permissions: Some(vec![
                ServicePermission {
                    service_id: "test_service".to_string(),
                    default_policy: "allow".to_string(),
                    allowed_actions: vec![PermissionAction::Get],
                    denied_actions: vec![],
                    table_permissions: None,
                }
            ]),
            ..DatabaseConfig::default()
        };
        let perm = config.get_service_permission("test_service").unwrap();
        assert_eq!(perm.service_id, "test_service");
    }

    #[test]
    fn test_permission_action_display() {
        assert_eq!(PermissionAction::Get.to_string(), "get");
        assert_eq!(PermissionAction::Query.to_string(), "query");
        assert_eq!(PermissionAction::All.to_string(), "*");
    }
}
