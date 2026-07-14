//! 向量 Tab 业务逻辑
//!
//! 获取索引信息、搜索最近邻、删除向量。

use anyhow::{anyhow, Result};

use laoflchdb_embedding_service_proto::proto::{
    DeleteEmbeddingRequest, GetIndexInfoRequest, SearchEmbeddingRequest,
};

use crate::app::App;

/// 获取索引信息
pub async fn get_index_info(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();

    let req = GetIndexInfoRequest { index_name };
    app.set_status("正在获取索引信息...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .embedding
            .get_index_info(auth_req)
            .await
            .map_err(|e| anyhow!("获取失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("获取失败: {}", resp.message));
        return Ok(());
    }
    if let Some(stats) = resp.stats {
        // 注意：distance_metric 是 String，需要先 clone 再组成元组
        let info = (
            stats.num_elements,
            stats.dim,
            stats.distance_metric.clone(),
            stats.max_layers,
        );
        app.vector_tab.index_info = Some(info);
        app.set_status(format!(
            "索引: elements={}, dim={}, metric={}, layers={}",
            stats.num_elements, stats.dim, stats.distance_metric, stats.max_layers
        ));
    }
    Ok(())
}

/// 搜索最近邻
///
/// `query_vec` 中输入用逗号分隔的浮点数（如 "0.1,0.2,..."）。
pub async fn search(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    let top_k: i32 = app.vector_tab.top_k.value.parse().unwrap_or(5);

    let query: Vec<f32> = app
        .vector_tab
        .query_vec
        .value
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if query.is_empty() {
        app.set_error("请输入查询向量（逗号分隔的浮点数）");
        return Ok(());
    }

    let req = SearchEmbeddingRequest {
        query_embedding: query,
        top_k,
        index_name,
    };
    app.set_status("正在搜索...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .embedding
            .search_embedding(auth_req)
            .await
            .map_err(|e| anyhow!("搜索失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("搜索失败: {}", resp.message));
        return Ok(());
    }

    let results: Vec<(u64, f32)> = resp.results.iter().map(|r| (r.id, r.distance)).collect();
    let n = results.len();
    app.vector_tab.search_results = results;
    app.vector_tab.list_scroll = 0;
    app.set_status(format!("找到 {} 个结果", n));
    Ok(())
}

/// 删除向量
pub async fn delete_embedding(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    let id: u64 = match app.vector_tab.delete_id.value.parse() {
        Ok(v) => v,
        Err(_) => {
            app.set_error("请输入要删除的向量 ID（正整数）");
            return Ok(());
        }
    };

    let req = DeleteEmbeddingRequest { id, index_name };
    app.set_status("正在删除向量...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .embedding
            .delete_embedding(auth_req)
            .await
            .map_err(|e| anyhow!("删除失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("删除失败: {}", resp.message));
        return Ok(());
    }
    app.set_status(format!("已删除向量 id={}", id));
    Ok(())
}
