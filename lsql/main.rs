use clap::Parser;
use tonic::transport::Channel;

use laoflchDB_rust::pb::rpc::laoflch_db_client::LaoflchDbClient;
use laoflchDB_rust::pb::rpc::{
    ListTablesRequest, ListSchemasRequest, SqlQueryRequest,
    GetVersionRequest, ListTableColsRequest,
};

#[derive(Parser, Debug)]
#[command(name = "lsql", version = env!("CARGO_PKG_VERSION"))]
#[command(about = "LaoflchDB 交互式 SQL 客户端 (类似 PostgreSQL 的 psql)")]
pub struct LsqlCli {
    #[arg(long, help = "数据库服务器地址，格式为 host:port")]
    pub host: String,
    
    #[arg(short, long, default_value = "sys", help = "默认 Schema 名称")]
    pub schema: String,
    
    #[arg(short, long, help = "执行单次 SQL 命令后退出")]
    pub command: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = LsqlCli::parse();
    
    println!("欢迎使用 lsql - LaoflchDB SQL 客户端");
    println!("正在连接到 {}...", cli.host);
    
    // 连接到 gRPC 服务器
    let addr = format!("http://{}", cli.host);
    let mut client = LaoflchDbClient::connect(addr).await?;
    
    println!("连接成功！");
    
    // 获取所有可用的 schema
    let schemas = list_schemas_internal(&mut client).await?;
    
    // 使用用户指定的 schema 或默认的 sys schema
    let default_schema = cli.schema;
    if !schemas.contains(&default_schema) {
        eprintln!("错误: Schema '{}' 不存在！", default_schema);
        eprintln!("可用的 Schema: {}", schemas.join(", "));
        std::process::exit(1);
    }
    
    println!("默认 Schema: {}", default_schema);
    println!("");
    println!("输入 '\\help' 查看帮助，'\\q' 或 '\\quit' 退出，'\\dt' 查看所有表\n");
    
    if let Some(sql) = cli.command {
        // 单次命令模式
        execute_sql(&mut client, &default_schema, &sql).await?;
    } else {
        // 交互式模式
        run_interactive_mode(&mut client, default_schema).await?;
    }
    
    Ok(())
}

async fn run_interactive_mode(
    client: &mut LaoflchDbClient<Channel>,
    mut schema: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rl = rustyline::DefaultEditor::new()?;
    
    loop {
        let prompt = format!("lsql@{}> ", schema);
        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                
                if line.is_empty() {
                    continue;
                }
                
                rl.add_history_entry(line)?;
                
                if line.starts_with('\\') {
                    // 元命令
                    if let Some(new_schema) = handle_meta_command(client, &mut schema, line).await? {
                        schema = new_schema;
                    } else if line == "\\q" || line == "\\quit" {
                        break;
                    }
                } else {
                    // SQL 命令
                    if let Err(e) = execute_sql(client, &schema, line).await {
                        eprintln!("错误: {}", e);
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                println!("按 Ctrl+D 或输入 '\\q' 退出");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                println!("再见！");
                break;
            }
            Err(err) => {
                eprintln!("错误: {}", err);
                break;
            }
        }
    }
    
    Ok(())
}

async fn handle_meta_command(
    client: &mut LaoflchDbClient<Channel>,
    schema: &mut String,
    cmd: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    
    match parts[0] {
        "\\q" | "\\quit" => {
            println!("再见！");
            Ok(None)
        }
        "\\help" | "\\?" | "\\h" => {
            print_help();
            Ok(None)
        }
        "\\version" => {
            print_version(client).await?;
            Ok(None)
        }
        "\\dn" | "\\schemas" => {
            list_schemas(client).await?;
            Ok(None)
        }
        "\\dt" => {
            list_tables(client, schema).await?;
            Ok(None)
        }
        "\\c" | "\\connect" if parts.len() == 2 => {
            let new_schema = parts[1].to_string();
            // 验证 schema 是否存在
            let schemas = list_schemas_internal(client).await?;
            if schemas.contains(&new_schema) {
                println!("已切换到 Schema '{}'", new_schema);
                Ok(Some(new_schema))
            } else {
                println!("错误: Schema '{}' 不存在！", new_schema);
                println!("可用的 Schema: {}", schemas.join(", "));
                Ok(None)
            }
        }
        "\\d" if parts.len() == 2 => {
            describe_table(client, schema, parts[1]).await?;
            Ok(None)
        }
        _ => {
            println!("未知命令: '{}'. 输入 '\\help' 查看帮助。", cmd);
            Ok(None)
        }
    }
}

fn print_help() {
    println!("lsql 帮助:");
    println!("  \\q, \\quit                退出 lsql");
    println!("  \\help, \\?, \\h           显示此帮助信息");
    println!("  \\version                显示版本信息");
    println!("  \\dn, \\schemas           列出所有可用的 Schema");
    println!("  \\c, \\connect <schema>    切换到指定的 Schema");
    println!("  \\dt                     列出当前 schema 中的所有表");
    println!("  \\d <table>              显示表结构");
    println!("  <sql>                   执行 SQL 查询");
    println!();
}

async fn print_version(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("lsql 客户端版本: {}", env!("CARGO_PKG_VERSION"));
    
    match client.get_version(GetVersionRequest {}).await {
        Ok(response) => {
            let response = response.into_inner();
            if response.success {
                println!("数据库服务版本: {}", response.version);
                if !response.build_info.is_empty() {
                    println!("服务构建信息: {}", response.build_info);
                }
            } else {
                println!("无法获取数据库服务版本: {}", response.message);
            }
        }
        Err(e) => {
            println!("警告: 无法连接到数据库服务获取版本信息: {}", e);
        }
    }
    
    Ok(())
}

async fn list_tables(
    client: &mut LaoflchDbClient<Channel>,
    schema: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = ListTablesRequest {
        schema: schema.to_string(),
    };
    
    let response = client.list_tables(request).await?;
    let response = response.into_inner();
    
    if response.success {
        if response.tables.is_empty() {
            println!("没有找到表");
        } else {
            println!("当前 Schema '{}' 中的表:", schema);
            for table in response.tables {
                println!("  - {}", table);
            }
        }
    } else {
        println!("错误: {}", response.message);
    }
    
    Ok(())
}

async fn list_schemas(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    let schemas = list_schemas_internal(client).await?;
    
    if schemas.is_empty() {
        println!("没有找到 Schema");
    } else {
        println!("所有 Schema:");
        for schema in schemas {
            println!("  - {}", schema);
        }
    }
    
    Ok(())
}

async fn list_schemas_internal(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let request = ListSchemasRequest {};
    
    let response = client.list_schemas(request).await?;
    let response = response.into_inner();
    
    if response.success {
        Ok(response.schemas)
    } else {
        Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, response.message)))
    }
}

async fn describe_table(
    client: &mut LaoflchDbClient<Channel>,
    schema: &str,
    table: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = ListTableColsRequest {
        schema: schema.to_string(),
        table_name: table.to_string(),
    };
    
    let response = client.list_table_cols(request).await?;
    let response = response.into_inner();
    
    if !response.success {
        println!("错误: {}", response.message);
        return Ok(());
    }
    
    if response.columns.is_empty() {
        println!("表 '{}' 没有列", table);
        return Ok(());
    }
    
    // 显示表名
    println!("表 \"{}.{}\"", schema, table);
    
    // 计算列宽
    let mut col_id_width = 8; // "列ID".len()
    let mut col_name_width = 8; // "列名".len()
    let mut col_type_width = 8; // "类型".len()
    
    for col in &response.columns {
        let id_str = col.column_id.to_string();
        let type_str = column_type_to_string(col.column_type);
        
        if id_str.len() > col_id_width {
            col_id_width = id_str.len();
        }
        if col.column_name.len() > col_name_width {
            col_name_width = col.column_name.len();
        }
        if type_str.len() > col_type_width {
            col_type_width = type_str.len();
        }
    }
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    println!("+");
    
    // 打印表头
    println!("| {:^width1$} | {:^width2$} | {:^width3$} |",
        "列ID", "列名", "类型",
        width1 = col_id_width,
        width2 = col_name_width,
        width3 = col_type_width
    );
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    println!("+");
    
    // 打印列信息
    for col in &response.columns {
        let type_str = column_type_to_string(col.column_type);
        println!("| {:>width1$} | {:<width2$} | {:<width3$} |",
            col.column_id,
            col.column_name,
            type_str,
            width1 = col_id_width,
            width2 = col_name_width,
            width3 = col_type_width
        );
    }
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    println!("+");
    
    println!("({} 列)", response.columns.len());
    
    Ok(())
}

fn column_type_to_string(col_type: i32) -> String {
    // 列类型常量（来自 protobuf 定义）
    const COLUMN_TYPE_INT64: i32 = 0;
    const COLUMN_TYPE_STRING: i32 = 1;
    const COLUMN_TYPE_BYTES: i32 = 2;
    const COLUMN_TYPE_FLOAT: i32 = 3;
    const COLUMN_TYPE_BOOL: i32 = 4;
    
    match col_type {
        COLUMN_TYPE_INT64 => "INT64".to_string(),
        COLUMN_TYPE_STRING => "STRING".to_string(),
        COLUMN_TYPE_BYTES => "BYTES".to_string(),
        COLUMN_TYPE_FLOAT => "FLOAT".to_string(),
        COLUMN_TYPE_BOOL => "BOOL".to_string(),
        _ => format!("UNKNOWN({})", col_type),
    }
}

async fn execute_sql(
    client: &mut LaoflchDbClient<Channel>,
    schema: &str,
    sql: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;
    
    let start = Instant::now();
    
    let request = SqlQueryRequest {
        schema: schema.to_string(),
        sql: sql.to_string(),
    };
    
    let response = client.sql_query(request).await?;
    let response = response.into_inner();
    
    let elapsed = start.elapsed();
    
    if response.success {
        print_query_result(&response);
        println!("\n耗时: {:?}", elapsed);
    } else {
        println!("错误: {}", response.message);
    }
    
    Ok(())
}

fn print_query_result(response: &laoflchDB_rust::pb::rpc::SqlQueryResponse) {
    use laoflchDB_rust::pb::rpc::sql_field;
    
    if response.rows.is_empty() {
        println!("查询成功，没有返回结果");
        return;
    }
    
    // 计算列宽（最小宽度为列名长度）
    let mut col_widths: Vec<usize> = response.columns.iter().map(|col| col.len()).collect();
    
    // 遍历所有行，计算每列最大宽度
    for row in &response.rows {
        for (i, field) in row.values.iter().enumerate() {
            let width = match &field.value {
                Some(sql_field::Value::StringValue(v)) => {
                    // 限制字符串显示长度，最多50个字符
                    if v.len() > 50 { 53 } else { v.len() } // 50 + "..."的长度
                }
                Some(sql_field::Value::Int64Value(v)) => v.to_string().len(),
                Some(sql_field::Value::FloatValue(v)) => {
                    // 格式化浮点数，最多显示6位小数
                    let formatted = format_float(*v);
                    formatted.len()
                }
                Some(sql_field::Value::BytesValue(v)) => format!("<bytes:{}>", v.len()).len(),
                Some(sql_field::Value::BoolValue(v)) => v.to_string().len(),
                None => 4, // "NULL"
            };
            if width > col_widths[i] {
                col_widths[i] = width;
            }
        }
    }
    
    // 打印分隔线
    print_separator(&col_widths);
    
    // 打印列名（左对齐）
    print!("|");
    for (i, col) in response.columns.iter().enumerate() {
        print!(" {:<width$} |", col, width = col_widths[i]);
    }
    println!();
    
    // 打印分隔线
    print_separator(&col_widths);
    
    // 打印数据行
    for row in &response.rows {
        print!("|");
        for (i, field) in row.values.iter().enumerate() {
            let (value_str, is_numeric) = match &field.value {
                Some(sql_field::Value::StringValue(v)) => {
                    // 截断超长字符串
                    if v.len() > 50 {
                        (format!("{}...", &v[..50]), false)
                    } else {
                        (v.clone(), false)
                    }
                }
                Some(sql_field::Value::Int64Value(v)) => (v.to_string(), true),
                Some(sql_field::Value::FloatValue(v)) => (format_float(*v), true),
                Some(sql_field::Value::BytesValue(v)) => (format!("<bytes:{}>", v.len()), false),
                Some(sql_field::Value::BoolValue(v)) => (v.to_string(), false),
                None => ("NULL".to_string(), false),
            };
            
            // 数字右对齐，其他左对齐
            if is_numeric {
                print!(" {:>width$} |", value_str, width = col_widths[i]);
            } else {
                print!(" {:<width$} |", value_str, width = col_widths[i]);
            }
        }
        println!();
    }
    
    // 打印分隔线
    print_separator(&col_widths);
    
    // 打印行数统计
    println!("({} 行)", response.rows.len());
}

fn format_float(v: f64) -> String {
    // 智能格式化浮点数
    if v == 0.0 {
        "0".to_string()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        // 整数
        format!("{}", v as i64)
    } else {
        // 最多6位小数，去除尾部的0
        let formatted = format!("{:.6}", v);
        let trimmed = formatted.trim_end_matches('0').trim_end_matches('.').to_string();
        trimmed
    }
}

fn print_separator(col_widths: &[usize]) {
    print!("+");
    for &width in col_widths {
        print!("-{:-<width$}-+", "", width = width);
    }
    println!();
}
