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