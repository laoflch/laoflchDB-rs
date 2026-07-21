//! 索引 Tab 业务逻辑
//!
//! 提供全文索引（Tantivy）的访问和管理功能：
//! - F1: 列出所有索引
//! - F2: 查看索引详情（元数据 + 字段）
//! - F3: 查看索引统计
//! - F4: 搜索索引

use std::collections::HashMap;

use anyhow::{anyhow, Result};

use laoflchdb_client::pb::rpc::{
    ListIndicesRequest, GetIndexMetaRequest, GetIndexFieldsRequest,
    GetIndexStatsRequest, SearchIndexRequest,
};

use crate::app::{App, FullTextIndexMeta, FullTextFieldInfo, FullTextIndexStats, FullTextSearchResult};

/// 列出所有索引
pub async fn list_indices(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let req = ListIndicesRequest {};
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .list_indices(auth_req)
            .await
            .map_err(|e| anyhow!("获取索引列表失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取索引列表失败: {}", resp.message));
        return Ok(());
    }

    app.index_tab.all_indices = resp.index_names.clone();
    app.index_tab.show_index_list = true;
    app.index_tab.list_scroll = 0;
    let n = app.index_tab.all_indices.len();
    app.set_status(format!("共 {} 个全文索引", n));
    Ok(())
}

/// 获取索引详情（元数据 + 字段）
pub async fn get_index_detail(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let index_name = app.index_tab.index_name.value.trim().to_string();
    if index_name.is_empty() {
        app.set_error("请先输入索引名称");
        return Ok(());
    }

    app.set_status(format!("正在获取索引 '{}' 的详情...", index_name));

    // 获取元数据
    let meta_req = GetIndexMetaRequest {
        index_name: index_name.clone(),
    };
    let meta_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(meta_req);
        clients
            .laoflchdb
            .get_index_meta(auth_req)
            .await
            .map_err(|e| anyhow!("获取索引元数据失败: {}", e))?
            .into_inner()
    };

    if !meta_resp.success {
        app.set_error(format!("获取索引元数据失败: {}", meta_resp.message));
        return Ok(());
    }

    app.index_tab.index_meta = Some(FullTextIndexMeta {
        index_id: meta_resp.index_id,
        index_name: meta_resp.index_name.clone(),
        column_count: meta_resp.column_count as u64,
        comment: meta_resp.comment,
    });

    // 获取字段列表
    let fields_req = GetIndexFieldsRequest {
        index_name: index_name.clone(),
    };
    let fields_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(fields_req);
        clients
            .laoflchdb
            .get_index_fields(auth_req)
            .await
            .map_err(|e| anyhow!("获取索引字段失败: {}", e))?
            .into_inner()
    };

    if !fields_resp.success {
        app.set_error(format!("获取索引字段失败: {}", fields_resp.message));
        return Ok(());
    }

    app.index_tab.index_fields = fields_resp
        .fields
        .iter()
        .map(|c| FullTextFieldInfo {
            column_id: c.column_id,
            column_name: c.column_name.clone(),
            column_type: column_type_to_string(c.column_type),
            comment: c.comment.clone(),
        })
        .collect();

    app.index_tab.show_index_detail = true;
    app.index_tab.detail_scroll = 0;
    let n = app.index_tab.index_fields.len();
    app.set_status(format!("索引 '{}' 共 {} 个字段", index_name, n));
    Ok(())
}

/// 获取索引统计信息
pub async fn get_index_stats(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let req = GetIndexStatsRequest {};
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .get_index_stats(auth_req)
            .await
            .map_err(|e| anyhow!("获取索引统计失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取索引统计失败: {}", resp.message));
        return Ok(());
    }

    app.index_tab.index_stats = Some(FullTextIndexStats {
        total_indices: resp.total_indices as u64,
        index_names: resp.index_names,
    });

    app.index_tab.show_index_stats = true;
    app.set_status("已获取索引统计信息");
    Ok(())
}

/// 搜索索引
pub async fn search_index(app: &mut App, query: &str) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let index_name = app.index_tab.index_name.value.trim().to_string();
    if index_name.is_empty() {
        app.set_error("请先输入索引名称");
        return Ok(());
    }

    if query.trim().is_empty() {
        app.set_error("搜索查询不能为空");
        return Ok(());
    }

    let limit: u32 = app.index_tab.search_limit.value.parse().unwrap_or(10);

    let req = SearchIndexRequest {
        index_name: index_name.clone(),
        query: query.to_string(),
        limit: Some(limit),
        field_queries: HashMap::new(),
    };

    app.set_status("正在搜索...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .search_index(auth_req)
            .await
            .map_err(|e| anyhow!("搜索索引失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("搜索失败: {}", resp.message));
        return Ok(());
    }

    app.index_tab.search_results = resp
        .results
        .iter()
        .map(|r| FullTextSearchResult {
            doc_id: r.doc_id.clone(),
            score: r.score as f64,
        })
        .collect();

    app.index_tab.show_search_results = true;
    app.index_tab.search_scroll = 0;
    app.index_tab.search_selected = None;
    let n = app.index_tab.search_results.len();
    app.set_status(format!("搜索 '{}' 返回 {} 条结果", query, n));
    Ok(())
}

/// 列类型转字符串
fn column_type_to_string(col_type: i32) -> String {
    const COLUMN_TYPE_STRING: i32 = 0;
    const COLUMN_TYPE_INT64: i32 = 1;
    const COLUMN_TYPE_BYTES: i32 = 2;
    const COLUMN_TYPE_FLOAT: i32 = 3;
    const COLUMN_TYPE_LIST: i32 = 4;
    const COLUMN_TYPE_IMAGE: i32 = 5;

    match col_type {
        COLUMN_TYPE_STRING => "STRING".to_string(),
        COLUMN_TYPE_INT64 => "INT64".to_string(),
        COLUMN_TYPE_BYTES => "BYTES".to_string(),
        COLUMN_TYPE_FLOAT => "FLOAT".to_string(),
        COLUMN_TYPE_LIST => "LIST".to_string(),
        COLUMN_TYPE_IMAGE => "IMAGE".to_string(),
        _ => format!("UNKNOWN({})", col_type),
    }
}