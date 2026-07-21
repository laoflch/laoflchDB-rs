//! 向量 Tab 业务逻辑
//!
//! 查看所有向量索引的元数据，通过下拉菜单和手动输入选择具体索引查看详情。

use anyhow::{anyhow, Result};

use laoflchdb_embedding_service_proto::proto::GetIndexInfoRequest;

use crate::app::{App, IndexInfo};

/// 获取所有索引信息（index_name 为空时返回全部）
pub async fn get_all_indices(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let req = GetIndexInfoRequest { index_name: String::new() };
    app.set_status("正在获取所有索引信息...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .embedding
            .get_index_info(auth_req)
            .await
    };

    let mut indices: Vec<IndexInfo> = Vec::new();

    let resp = match resp {
        Ok(r) => r.into_inner(),
        Err(_e) => {
            // 旧服务端不支持空 index_name，使用已知名称列表
            app.set_status("注意：服务端需要重启以支持多索引列表");
            for name in ["default", "image", "face", "memory"] {
                indices.push(IndexInfo {
                    name: name.to_string(),
                    num_elements: 0,
                    dim: 0,
                    distance_metric: String::new(),
                    max_layers: 0,
                    search_count: 0,
                    insert_count: 0,
                    delete_count: 0,
                    snapshot_path: String::new(),
                });
            }
            app.vector_tab.all_indices = indices;
            app.vector_tab.selected_dropdown = 0;
            app.set_status(format!("共有 {} 个索引（旧服务端，数据待刷新）", app.vector_tab.all_indices.len()));
            return Ok(());
        }
    };

    // 使用 all_stats（新协议）
    for s in &resp.all_stats {
        indices.push(IndexInfo {
            name: s.name.clone(),
            num_elements: s.num_elements,
            dim: s.dim,
            distance_metric: s.distance_metric.clone(),
            max_layers: s.max_layers,
            search_count: s.search_count,
            insert_count: s.insert_count,
            delete_count: s.delete_count,
            snapshot_path: s.snapshot_path.clone(),
        });
    }

    // 如果 all_stats 为空，尝试从单个 stats 获取（兼容旧协议）
    if indices.is_empty() {
        if let Some(s) = &resp.stats {
            indices.push(IndexInfo {
                name: if s.name.is_empty() { "default".to_string() } else { s.name.clone() },
                num_elements: s.num_elements,
                dim: s.dim,
                distance_metric: s.distance_metric.clone(),
                max_layers: s.max_layers,
                search_count: s.search_count,
                insert_count: s.insert_count,
                delete_count: s.delete_count,
                snapshot_path: s.snapshot_path.clone(),
            });
            app.set_status("注意：服务端需要重启以支持多索引列表");
        }
    }

    app.vector_tab.all_indices = indices;
    app.vector_tab.selected_dropdown = 0;
    let count = app.vector_tab.all_indices.len();
    app.set_status(format!("共有 {} 个索引", count));
    Ok(())
}

/// 获取单个索引的详细信息（通过 index_name）
pub async fn get_index_info(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    if index_name.is_empty() {
        app.set_error("请输入索引名称");
        return Ok(());
    }

    let req = GetIndexInfoRequest { index_name: index_name.clone() };
    app.set_status(format!("正在获取索引 {} 信息...", index_name));
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

    if let Some(s) = resp.stats {
        let info = IndexInfo {
            name: index_name.clone(),
            num_elements: s.num_elements,
            dim: s.dim,
            distance_metric: s.distance_metric.clone(),
            max_layers: s.max_layers,
            search_count: s.search_count,
            insert_count: s.insert_count,
            delete_count: s.delete_count,
            snapshot_path: s.snapshot_path.clone(),
        };

        // 更新或追加到 all_indices
        let found = app.vector_tab.all_indices.iter_mut().find(|i| i.name == index_name);
        if let Some(existing) = found {
            *existing = info;
        } else {
            app.vector_tab.all_indices.push(info);
        }

        app.set_status(format!(
            "索引 {}: elements={}, dim={}, metric={}",
            index_name, s.num_elements, s.dim, s.distance_metric
        ));
    }
    Ok(())
}

/// 列出当前索引中的所有向量条目
pub async fn list_embeddings(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    if index_name.is_empty() {
        app.set_error("请输入索引名称");
        return Ok(());
    }

    use laoflchdb_embedding_service_proto::proto::ListEmbeddingsRequest;
    let req = ListEmbeddingsRequest {
        index_name: index_name.clone(),
        limit: 0,  // 全部
        offset: 0,
    };

    app.set_status(format!("正在获取索引 {} 的向量条目...", index_name));
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .embedding
            .list_embeddings(auth_req)
            .await
            .map_err(|e| anyhow!("获取条目失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("获取条目失败: {}", resp.message));
        return Ok(());
    }

    app.vector_tab.entries = resp
        .entries
        .into_iter()
        .map(|e| (e.id, e.embedding))
        .collect();
    app.vector_tab.entries_scroll = 0;

    app.set_status(format!(
        "索引 {}: 共 {} 条向量",
        index_name, app.vector_tab.entries.len()
    ));
    Ok(())
}

/// 删除当前索引中的单个向量
pub async fn delete_single_embedding(app: &mut App, id: u64) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    use laoflchdb_embedding_service_proto::proto::DeleteEmbeddingRequest;
    let req = DeleteEmbeddingRequest {
        id,
        index_name: index_name.clone(),
    };
    let clients = app.clients.as_mut().unwrap();
    let auth_req = clients.auth_request(req);
    let resp = clients
        .embedding
        .delete_embedding(auth_req)
        .await
        .map_err(|e| anyhow!("删除向量失败: {}", e))?
        .into_inner();
    if !resp.success {
        app.set_error(format!("删除向量失败: {}", resp.message));
        return Ok(());
    }
    // 刷新列表
    let _ = list_embeddings(app).await;
    app.set_status(format!("已删除向量 id={}", id));
    Ok(())
}

/// 清空当前索引的所有向量
pub async fn clear_embeddings(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let index_name = app.vector_tab.index_name.value.clone();
    if index_name.is_empty() {
        app.set_error("请输入索引名称");
        return Ok(());
    }

    // 先获取所有条目
    use laoflchdb_embedding_service_proto::proto::ListEmbeddingsRequest;
    let list_req = ListEmbeddingsRequest {
        index_name: index_name.clone(),
        limit: 0,
        offset: 0,
    };
    let list_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(list_req);
        clients
            .embedding
            .list_embeddings(auth_req)
            .await
            .map_err(|e| anyhow!("获取条目失败: {}", e))?
            .into_inner()
    };
    if !list_resp.success {
        app.set_error(format!("获取条目失败: {}", list_resp.message));
        return Ok(());
    }

    let total = list_resp.entries.len();
    app.set_status(format!("正在清空索引 {} 的 {} 条向量...", index_name, total));

    use laoflchdb_embedding_service_proto::proto::DeleteEmbeddingRequest;
    for entry in &list_resp.entries {
        let del_req = DeleteEmbeddingRequest {
            id: entry.id,
            index_name: index_name.clone(),
        };
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(del_req);
        if let Err(e) = clients.embedding.delete_embedding(auth_req).await {
            log::warn!("删除向量 id={} 失败: {}", entry.id, e);
        }
    }

    // 刷新
    let _ = list_embeddings(app).await;
    app.set_status(format!("已清空索引 {} 的全部 {} 条向量", index_name, total));
    Ok(())
}