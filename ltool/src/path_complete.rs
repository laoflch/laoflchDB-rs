//! 本地文件路径自动补全（下拉菜单式）
//!
//! 在文件路径输入框中输入字符时自动列出候选下拉菜单，
//! Up/Down 选择，Enter 确认，Esc 取消。
//! 支持 ~ 展开（home 目录）。

use std::path::{Path, PathBuf};

/// 单个候选项
#[derive(Debug, Clone)]
pub struct Candidate {
    /// 显示名（目录带末尾 /）
    pub display: String,
    /// 选中后写入输入框的完整路径
    pub full_path: String,
    /// 是否目录
    pub is_dir: bool,
}

/// 列出输入路径对应的候选
pub fn list_candidates(input: &str) -> Vec<Candidate> {
    let (dir, prefix, had_tilde) = match resolve_input(input) {
        Some(r) => r,
        None => return Vec::new(),
    };
    let dir_path = if dir.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(&dir)
    };
    if !dir_path.exists() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(&dir_path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut matches: Vec<(String, bool)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&prefix) {
            continue;
        }
        let is_hidden = name.starts_with('.');
        if is_hidden && !prefix.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().ok();
        let is_dir = file_type.map(|t| t.is_dir()).unwrap_or(false);
        matches.push((name, is_dir));
    }
    // 排序：目录优先，然后按名称
    matches.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0),
    });

    matches
        .into_iter()
        .map(|(name, is_dir)| {
            let display = if is_dir {
                format!("{}/", name)
            } else {
                name.clone()
            };
            let full_expanded = join_path(&dir, &name);
            let full_expanded = if is_dir {
                format!("{}/", full_expanded)
            } else {
                full_expanded
            };
            let full_path = if had_tilde {
                reapply_tilde(&full_expanded, input)
            } else {
                full_expanded
            };
            Candidate {
                display,
                full_path,
                is_dir,
            }
        })
        .collect()
}

/// 解析输入，返回 (目录, 前缀, 是否有 ~)
fn resolve_input(input: &str) -> Option<(String, String, bool)> {
    if input.is_empty() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        return Some((home, String::new(), false));
    }
    let (expanded, had_tilde) = expand_tilde(input);
    let (dir, prefix) = split_dir_prefix(&expanded);
    Some((dir, prefix, had_tilde))
}

/// 展开 ~ 为 home 目录
fn expand_tilde(input: &str) -> (String, bool) {
    if input == "~" {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        (home, true)
    } else if let Some(rest) = input.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        (format!("{}/{}", home, rest), true)
    } else {
        (input.to_string(), false)
    }
}

/// 把补全后的路径重新用 ~ 表示
fn reapply_tilde(completed: &str, original: &str) -> String {
    if original == "~" {
        if let Some(name) = Path::new(completed).file_name() {
            return format!("~/{}", name.to_string_lossy());
        }
        return completed.to_string();
    }
    if let Some(_rest) = original.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        if let Some(rest) = completed.strip_prefix(&format!("{}/", home)) {
            return format!("~/{}", rest);
        }
    }
    completed.to_string()
}

/// 分离目录和前缀
fn split_dir_prefix(path: &str) -> (String, String) {
    if let Some(idx) = path.rfind('/') {
        let dir = &path[..idx];
        let prefix = &path[idx + 1..];
        let dir = if dir.is_empty() && path.starts_with('/') {
            "/".to_string()
        } else {
            dir.to_string()
        };
        (dir, prefix.to_string())
    } else {
        (String::new(), path.to_string())
    }
}

/// 拼接目录和文件名
fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_dir_prefix() {
        assert_eq!(split_dir_prefix("/tmp/tes"), ("/tmp".to_string(), "tes".to_string()));
        assert_eq!(split_dir_prefix("tes"), ("".to_string(), "tes".to_string()));
        assert_eq!(split_dir_prefix("/tmp/"), ("/tmp".to_string(), "".to_string()));
        assert_eq!(split_dir_prefix("/"), ("/".to_string(), "".to_string()));
        assert_eq!(split_dir_prefix("/tm"), ("/".to_string(), "tm".to_string()));
    }

    #[test]
    fn test_list_candidates_tmp_dir() {
        let cs = list_candidates("/tm");
        assert_eq!(cs.len(), 1, "/tm 应只匹配 /tmp");
        assert!(cs[0].is_dir, "/tmp 应是目录");
        assert_eq!(cs[0].full_path, "/tmp/");
    }

    #[test]
    fn test_list_candidates_nonexistent_dir() {
        let cs = list_candidates("/this/does/not/exist/prefix");
        assert!(cs.is_empty(), "不存在的目录应返回空");
    }

    #[test]
    fn test_list_candidates_no_match_in_dir() {
        let cs = list_candidates("/tmp/zzzz_nonexistent_prefix_xyz");
        assert!(cs.is_empty(), "无匹配前缀应返回空");
    }

    #[test]
    fn test_list_candidates_empty_input() {
        let cs = list_candidates("");
        assert!(!cs.is_empty(), "空输入应列出 home 目录");
    }

    #[test]
    fn test_list_candidates_tilde() {
        let cs = list_candidates("~");
        assert!(!cs.is_empty(), "~ 应展开为 home 目录");
        assert!(cs.iter().all(|c| c.full_path.starts_with('~')), "候选应以 ~/ 开头");
    }
}