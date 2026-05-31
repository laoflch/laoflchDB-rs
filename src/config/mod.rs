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
pub struct DatabaseConfig {
    pub db_path: String,
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
            addr: default_addr(),
            log_level: default_log_level(),
            access_protocols: vec![],
            permissions: None,
            default_policy: default_global_default_policy(),
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
