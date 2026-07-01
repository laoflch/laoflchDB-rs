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
