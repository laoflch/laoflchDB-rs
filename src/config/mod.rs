use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct AccessProtocolConfig {
    pub protocol: String,
    pub enabled: bool,
    #[serde(default)]
    pub addr: Option<String>,
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
}

fn default_addr() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
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
        }
    }
}
