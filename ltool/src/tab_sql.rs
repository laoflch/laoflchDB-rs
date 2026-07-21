//! SQL Tab 业务逻辑
//!
//! 执行 SQL 查询并把结果转成字符串表格存入 `SqlTabState`，便于 UI 渲染。
//! 同时提供元数据查询功能：Schema 列表、表列表、表结构描述、版本信息。

use anyhow::{anyhow, Result};

use laoflchdb_client::pb::rpc::sql_field::Value as SqlFieldValue;
use laoflchdb_client::pb::rpc::{
    ListSchemasRequest, ListTablesRequest, ListTableColsRequest, GetVersionRequest,
    SqlQueryRequest,
};

use crate::app::{App, TableColumnInfo};

/// 执行 SQL 查询
pub async fn execute_sql(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let schema = app.sql_tab.schema.value.clone();
    let sql = app.sql_tab.sql.value.clone();

    if sql.trim().is_empty() {
        app.set_error("SQL 不能为空");
        return Ok(());
    }

    let req = SqlQueryRequest { schema, sql };
    app.set_status("正在执行 SQL...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .sql_query(auth_req)
            .await
            .map_err(|e| anyhow!("SQL 执行失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("SQL 错误: {}", resp.message));
        return Ok(());
    }

    // 把列名和行数据转换成字符串
    let columns = resp.columns.clone();
    let rows: Vec<Vec<String>> = resp
        .rows
        .iter()
        .map(|row| {
            row.values
                .iter()
                .map(|f| format_sql_field(&f.value))
                .collect()
        })
        .collect();

    let n = rows.len();
    app.sql_tab.columns = columns;
    app.sql_tab.rows = rows;
    app.sql_tab.list_scroll = 0;
    app.set_status(format!("查询成功，返回 {} 行", n));
    Ok(())
}

/// 清空当前 SQL 和结果
pub fn clear_sql(app: &mut App) {
    app.sql_tab.sql.set_value("");
    app.sql_tab.columns.clear();
    app.sql_tab.rows.clear();
    app.sql_tab.list_scroll = 0;
    app.set_status("已清空");
}

/// 获取所有 Schema 列表
pub async fn list_schemas(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let req = ListSchemasRequest {};
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .list_schemas(auth_req)
            .await
            .map_err(|e| anyhow!("获取 Schema 列表失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取 Schema 列表失败: {}", resp.message));
        return Ok(());
    }

    app.sql_tab.schemas = resp.schemas;
    app.sql_tab.show_schema_list = true;
    app.sql_tab.schema_list_scroll = 0;
    let n = app.sql_tab.schemas.len();
    app.set_status(format!("共 {} 个 Schema", n));
    Ok(())
}

/// 获取当前 Schema 的表列表
pub async fn list_tables(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let schema = app.sql_tab.schema.value.clone();
    let req = ListTablesRequest { schema: schema.clone() };

    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .list_tables(auth_req)
            .await
            .map_err(|e| anyhow!("获取表列表失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取表列表失败: {}", resp.message));
        return Ok(());
    }

    app.sql_tab.tables = resp.tables;
    app.sql_tab.show_table_list = true;
    app.sql_tab.table_list_scroll = 0;
    let n = app.sql_tab.tables.len();
    app.set_status(format!("Schema '{}' 共 {} 张表", schema, n));
    Ok(())
}

/// 获取表结构描述
pub async fn describe_table(app: &mut App, table_name: &str) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let schema = app.sql_tab.schema.value.clone();
    let req = ListTableColsRequest {
        schema: schema.clone(),
        table_name: table_name.to_string(),
    };

    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .list_table_cols(auth_req)
            .await
            .map_err(|e| anyhow!("获取表结构失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取表结构失败: {}", resp.message));
        return Ok(());
    }

    let columns = resp
        .columns
        .iter()
        .map(|c| TableColumnInfo {
            column_id: c.column_id,
            column_name: c.column_name.clone(),
            column_type: column_type_to_string(c.column_type),
            comment: c.comment.clone(),
        })
        .collect();

    app.sql_tab.table_columns = columns;
    app.sql_tab.show_table_desc = true;
    app.sql_tab.desc_scroll = 0;
    let n = app.sql_tab.table_columns.len();
    app.set_status(format!("表 '{}' 共 {} 列", table_name, n));
    Ok(())
}

/// 获取服务器版本信息
pub async fn get_version(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let req = GetVersionRequest {};
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .laoflchdb
            .get_version(auth_req)
            .await
            .map_err(|e| anyhow!("获取版本信息失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("获取版本信息失败: {}", resp.message));
        return Ok(());
    }

    app.sql_tab.server_version = resp.version;
    app.sql_tab.server_build_info = resp.build_info;
    app.sql_tab.show_version = true;
    app.set_status("已获取服务器版本信息");
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

/// 把 SqlField 的 oneof value 转成字符串
fn format_sql_field(v: &Option<SqlFieldValue>) -> String {
    match v {
        Some(SqlFieldValue::StringValue(s)) => s.clone(),
        Some(SqlFieldValue::Int64Value(n)) => n.to_string(),
        Some(SqlFieldValue::FloatValue(f)) => format_float(*f),
        Some(SqlFieldValue::BytesValue(b)) => format!("<bytes:{}>", b.len()),
        Some(SqlFieldValue::BoolValue(b)) => b.to_string(),
        None => "NULL".to_string(),
    }
}

/// 智能格式化浮点数
fn format_float(v: f64) -> String {
    if v == 0.0 {
        "0".to_string()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let s = format!("{:.6}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}