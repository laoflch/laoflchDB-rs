//! SQL Tab 业务逻辑
//!
//! 执行 SQL 查询并把结果转成字符串表格存入 `SqlTabState`，便于 UI 渲染。

use anyhow::{anyhow, Result};

use laoflchdb_client::pb::rpc::sql_field::Value as SqlFieldValue;
use laoflchdb_client::pb::rpc::SqlQueryRequest;

use crate::app::App;

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
