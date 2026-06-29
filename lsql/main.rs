use clap::Parser;
use tonic::transport::Channel;
use std::sync::atomic::{AtomicBool, Ordering};

use laoflchDB_rust::pb::rpc::laoflch_db_client::LaoflchDbClient;
use laoflchDB_rust::pb::rpc::{
    ListTablesRequest, ListSchemasRequest, SqlQueryRequest,
    GetVersionRequest, ListTableColsRequest, LoginRequest, LogoutRequest,
    ListIndicesRequest, GetIndexMetaRequest, GetIndexFieldsRequest,
    SearchIndexRequest, GetIndexStatsRequest,
};

#[derive(Parser, Debug)]
#[command(name = "lsql", version = env!("CARGO_PKG_VERSION"))]
#[command(about = "LaoflchDB 交互式 SQL 客户端 (类似 PostgreSQL 的 psql)")]
pub struct LsqlCli {
    #[arg(long, help = "数据库服务器地址，格式为 host:port")]
    pub host: String,

    #[arg(short, long, help = "默认 Schema 名称")]
    pub schema: Option<String>,

    #[arg(short = 'u', long, help = "用户名")]
    pub user: Option<String>,

    #[arg(short = 'W', long, help = "密码")]
    pub password: Option<String>,

    #[arg(short, long, help = "执行单次 SQL 命令后退出")]
    pub command: Option<String>,
}

// 全局变量存储当前 token
lazy_static::lazy_static! {
    static ref AUTH_TOKEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    static ref IS_AUTHENTICATED: AtomicBool = AtomicBool::new(false);
}

fn set_auth_token(token: String) {
    let mut guard = AUTH_TOKEN.lock().unwrap();
    *guard = Some(token);
    IS_AUTHENTICATED.store(true, Ordering::SeqCst);
}

fn get_auth_token() -> Option<String> {
    AUTH_TOKEN.lock().unwrap().clone()
}

fn clear_auth_token() {
    let mut guard = AUTH_TOKEN.lock().unwrap();
    *guard = None;
    IS_AUTHENTICATED.store(false, Ordering::SeqCst);
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

    // 如果提供了用户名和密码，进行登录
    if let (Some(username), Some(password)) = (&cli.user, &cli.password) {
        println!("正在验证用户 {}...", username);
        let request = LoginRequest {
            username: username.clone(),
            password: password.clone(),
        };

        match client.login(request).await {
            Ok(response) => {
                let response = response.into_inner();
                if response.success {
                    set_auth_token(response.token.clone());
                    println!("✅ 登录成功！");
                    println!("   用户: {}", response.username);
                } else {
                    eprintln!("❌ 登录失败: {}", response.message);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("❌ 登录请求失败: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        println!("⚠️  未提供用户名和密码，将以访客模式连接（部分功能可能受限）");
    }

    // 获取所有可用的 schema
    let schemas = list_schemas_internal(&mut client).await?;

    // 使用用户指定的 schema 或默认的 sys schema
    let default_schema = cli.schema.unwrap_or_else(|| "sys".to_string());
    if !schemas.contains(&default_schema) {
        eprintln!("警告: Schema '{}' 不存在！", default_schema);
    }

    println!("默认 Schema: {}", default_schema);
    println!("");
    println!("输入 '\\help' 查看帮助，'\\q' 或 '\\quit' 退出，'\\dt' 查看所有表\n");

    if let Some(cmd) = cli.command {
        // 单次命令模式
        let mut schema = default_schema.clone();
        if cmd.starts_with('\\') {
            // 元命令
            handle_meta_command(&mut client, &mut schema, &cmd).await?;
        } else {
            // SQL 命令
            execute_sql(&mut client, &default_schema, &cmd).await?;
        }
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
        let prompt = if IS_AUTHENTICATED.load(Ordering::SeqCst) {
            format!("lsql@{}> ", schema)
        } else {
            format!("lsql@{} (guest)> ", schema)
        };

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
                break;
            }
            Err(err) => {
                eprintln!("错误: {}", err);
                break;
            }
        }
    }

    // 退出时注销登录
    if IS_AUTHENTICATED.load(Ordering::SeqCst) {
        if let Some(token) = get_auth_token() {
            let request = LogoutRequest { token };
            if let Err(e) = client.logout(request).await {
                eprintln!("注销失败: {}", e);
            } else {
                println!("已注销登录");
            }
        }
        clear_auth_token();
    }

    println!("再见！");
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
        "\\version" | "\\v" => {
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
        "\\login" => {
            handle_login(client).await?;
            Ok(None)
        }
        "\\logout" => {
            handle_logout(client).await?;
            Ok(None)
        }
        "\\di" | "\\indices" => {
            list_indices(client).await?;
            Ok(None)
        }
        "\\dix" if parts.len() == 2 => {
            describe_index(client, parts[1]).await?;
            Ok(None)
        }
        "\\search" if parts.len() >= 3 => {
            let index_name = parts[1];
            let query = parts[2..].join(" ");
            search_index(client, index_name, &query).await?;
            Ok(None)
        }
        "\\dstats" => {
            get_index_stats(client).await?;
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
    println!("  \\version, \\v            显示版本信息");
    println!("  \\login                  登录数据库");
    println!("  \\logout                 退出登录");
    println!("  \\dn, \\schemas           列出所有可用的 Schema");
    println!("  \\c, \\connect <schema>    切换到指定的 Schema");
    println!("  \\dt                     列出当前 schema 中的所有表");
    println!("  \\d <table>              显示表结构");
    println!("  \\di, \\indices           列出所有全文索引");
    println!("  \\dix <index>            显示索引详细信息");
    println!("  \\search <index> <query> 搜索索引");
    println!("  \\dstats                 显示索引统计信息");
    println!("  <sql>                   执行 SQL 查询");
    println!();
}

async fn print_version(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("lsql 版本: {}", env!("CARGO_PKG_VERSION"));
    
    match client.get_version(GetVersionRequest {}).await {
        Ok(response) => {
            let response = response.into_inner();
            if response.success {
                println!("laoflchdb 版本: {}", response.version);
                println!("服务构建信息: {}", response.build_info);
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

async fn handle_login(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{self, Write};
    
    if IS_AUTHENTICATED.load(Ordering::SeqCst) {
        println!("⚠️  您已经登录，先使用 \\logout 退出当前登录");
        return Ok(());
    }

    print!("用户名: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim();

    print!("密码: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim();

    let request = LoginRequest {
        username: username.to_string(),
        password: password.to_string(),
    };

    match client.login(request).await {
        Ok(response) => {
            let response = response.into_inner();
            if response.success {
                set_auth_token(response.token.clone());
                println!("✅ 登录成功！");
                println!("   用户: {}", response.username);
            } else {
                println!("❌ 登录失败: {}", response.message);
            }
        }
        Err(e) => {
            println!("❌ 登录请求失败: {}", e);
        }
    }

    Ok(())
}

async fn handle_logout(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !IS_AUTHENTICATED.load(Ordering::SeqCst) {
        println!("⚠️  您当前未登录");
        return Ok(());
    }

    if let Some(token) = get_auth_token() {
        let request = LogoutRequest { token: token.clone() };
        match client.logout(request).await {
            Ok(response) => {
                let response = response.into_inner();
                if response.success {
                    clear_auth_token();
                    println!("✅ 已成功退出登录");
                } else {
                    println!("❌ 注销失败: {}", response.message);
                }
            }
            Err(e) => {
                println!("❌ 注销请求失败: {}", e);
                clear_auth_token();
                println!("已清除本地登录状态");
            }
        }
    } else {
        println!("⚠️  未找到登录凭证");
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
    let mut col_comment_width = 8; // "注释".len()
    
    for col in &response.columns {
        let id_str = col.column_id.to_string();
        let type_str = column_type_to_string(col.column_type);
        let comment_str = col.comment.clone();
        
        if id_str.len() > col_id_width {
            col_id_width = id_str.len();
        }
        if col.column_name.len() > col_name_width {
            col_name_width = col.column_name.len();
        }
        if type_str.len() > col_type_width {
            col_type_width = type_str.len();
        }
        if comment_str.len() > col_comment_width {
            col_comment_width = comment_str.len();
        }
    }
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_comment_width + 2 { print!("-"); }
    println!("+");
    
    // 打印表头
    println!("| {:^width1$} | {:^width2$} | {:^width3$} | {:^width4$} |",
        "列ID", "列名", "类型", "注释",
        width1 = col_id_width,
        width2 = col_name_width,
        width3 = col_type_width,
        width4 = col_comment_width
    );
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_comment_width + 2 { print!("-"); }
    println!("+");
    
    // 打印列信息
    for col in &response.columns {
        let type_str = column_type_to_string(col.column_type);
        let comment_str = col.comment.clone();
        println!("| {:>width1$} | {:<width2$} | {:<width3$} | {:<width4$} |",
            col.column_id,
            col.column_name,
            type_str,
            comment_str,
            width1 = col_id_width,
            width2 = col_name_width,
            width3 = col_type_width,
            width4 = col_comment_width
        );
    }
    
    // 打印分隔线
    print!("+");
    for _ in 0..col_id_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_name_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_type_width + 2 { print!("-"); }
    print!("+");
    for _ in 0..col_comment_width + 2 { print!("-"); }
    println!("+");
    
    println!("({} 列)", response.columns.len());
    
    Ok(())
}

fn column_type_to_string(col_type: i32) -> String {
    // 列类型常量（来自 protobuf 定义）
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

async fn execute_sql(
    client: &mut LaoflchDbClient<Channel>,
    schema: &str,
    sql: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::time::Instant;
    use tonic::Request;

    let start = Instant::now();

    let request = SqlQueryRequest {
        schema: schema.to_string(),
        sql: sql.to_string(),
    };

    // 添加 auth token 到请求中
    let mut req = Request::new(request);
    if let Some(token) = get_auth_token() {
        let metadata = req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }

    let response = client.sql_query(req).await?;
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
                    // 限制字符串显示长度，最多50个字符（使用字符数）
                    if v.chars().count() > 50 { 53 } else { v.chars().count() } // 50 + "..."的长度
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
                    // 截断超长字符串（使用字符边界安全处理）
                    if v.chars().count() > 50 {
                        (v.chars().take(50).collect::<String>() + "...", false)
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

async fn list_indices(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic::Request;
    
    let request = ListIndicesRequest {};
    
    let mut req = Request::new(request);
    if let Some(token) = get_auth_token() {
        let metadata = req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }
    
    match client.list_indices(req).await {
        Ok(response) => {
            let response = response.into_inner();
            if response.success {
                if response.index_names.is_empty() {
                    println!("没有找到索引");
                } else {
                    println!("所有全文索引:");
                    for index_name in response.index_names {
                        println!("  - {}", index_name);
                    }
                }
            } else {
                println!("错误: {}", response.message);
            }
        }
        Err(e) => {
            println!("❌ 获取索引列表失败: {}", e);
            println!("⚠️  全文索引服务可能未启用或需要登录");
        }
    }
    
    Ok(())
}

async fn describe_index(
    client: &mut LaoflchDbClient<Channel>,
    index_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic::Request;
    
    let meta_request = GetIndexMetaRequest {
        index_name: index_name.to_string(),
    };
    
    let mut meta_req = Request::new(meta_request);
    if let Some(token) = get_auth_token() {
        let metadata = meta_req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }
    
    match client.get_index_meta(meta_req).await {
        Ok(response) => {
            let response = response.into_inner();
            if !response.success {
                println!("错误: {}", response.message);
                return Ok(());
            }
            
            println!("索引 \"{}\"", index_name);
            println!("  索引ID: {}", response.index_id);
            println!("  列数: {}", response.column_count);
            if !response.comment.is_empty() {
                println!("  注释: {}", response.comment);
            }
        }
        Err(e) => {
            println!("❌ 获取索引元数据失败: {}", e);
            println!("⚠️  可能需要登录或索引不存在");
            return Ok(());
        }
    }
    
    let fields_request = GetIndexFieldsRequest {
        index_name: index_name.to_string(),
    };
    
    let mut fields_req = Request::new(fields_request);
    if let Some(token) = get_auth_token() {
        let metadata = fields_req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }
    
    match client.get_index_fields(fields_req).await {
        Ok(response) => {
            let response = response.into_inner();
            if !response.success {
                println!("错误: {}", response.message);
                return Ok(());
            }
            
            if response.fields.is_empty() {
                println!("  字段: (无)");
                return Ok(());
            }
            
            let mut col_name_width = 8;
            let mut col_type_width = 8;
            let mut col_comment_width = 8;
            
            for field in &response.fields {
                if field.column_name.len() > col_name_width {
                    col_name_width = field.column_name.len();
                }
                let type_str = column_type_to_string(field.column_type);
                if type_str.len() > col_type_width {
                    col_type_width = type_str.len();
                }
                if field.comment.len() > col_comment_width {
                    col_comment_width = field.comment.len();
                }
            }
            
            println!();
            println!("  字段:");
            
            print!("  +");
            for _ in 0..col_name_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_type_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_comment_width + 2 { print!("-"); }
            println!("+");
            
            println!("  | {:^width1$} | {:^width2$} | {:^width3$} |",
                "字段名", "类型", "注释",
                width1 = col_name_width,
                width2 = col_type_width,
                width3 = col_comment_width
            );
            
            print!("  +");
            for _ in 0..col_name_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_type_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_comment_width + 2 { print!("-"); }
            println!("+");
            
            for field in &response.fields {
                let type_str = column_type_to_string(field.column_type);
                println!("  | {:<width1$} | {:<width2$} | {:<width3$} |",
                    field.column_name,
                    type_str,
                    field.comment,
                    width1 = col_name_width,
                    width2 = col_type_width,
                    width3 = col_comment_width
                );
            }
            
            print!("  +");
            for _ in 0..col_name_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_type_width + 2 { print!("-"); }
            print!("+");
            for _ in 0..col_comment_width + 2 { print!("-"); }
            println!("+");
        }
        Err(e) => {
            println!("❌ 获取索引字段失败: {}", e);
        }
    }
    
    Ok(())
}

async fn search_index(
    client: &mut LaoflchDbClient<Channel>,
    index_name: &str,
    query: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic::Request;
    
    let request = SearchIndexRequest {
        index_name: index_name.to_string(),
        query: query.to_string(),
        limit: Some(10),
        field_queries: Default::default(),
    };
    
    let mut req = Request::new(request);
    if let Some(token) = get_auth_token() {
        let metadata = req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }
    
    match client.search_index(req).await {
        Ok(response) => {
            let response = response.into_inner();
            if !response.success {
                println!("错误: {}", response.message);
                return Ok(());
            }
            
            if response.results.is_empty() {
                println!("没有找到匹配的文档");
                return Ok(());
            }
            
            println!("在索引 \"{}\" 中搜索 \"{}\":", index_name, query);
            println!("找到 {} 个结果:", response.results.len());
            println!();
            
            for (i, result) in response.results.iter().enumerate() {
                println!("结果 {}:", i + 1);
                println!("  文档ID: {}", result.doc_id);
                println!("  评分: {:.4}", result.score);
                println!("  字段:");
                for (field_name, field_value) in &result.fields {
                    let display_value = if field_value.chars().count() > 100 {
                        field_value.chars().take(100).collect::<String>() + "..."
                    } else {
                        field_value.clone()
                    };
                    println!("    {}: {}", field_name, display_value);
                }
                println!();
            }
        }
        Err(e) => {
            println!("❌ 搜索索引失败: {}", e);
            println!("⚠️  全文索引服务可能未启用或索引不存在");
        }
    }
    
    Ok(())
}

async fn get_index_stats(
    client: &mut LaoflchDbClient<Channel>,
) -> Result<(), Box<dyn std::error::Error>> {
    use tonic::Request;
    
    let request = GetIndexStatsRequest {};
    
    let mut req = Request::new(request);
    if let Some(token) = get_auth_token() {
        let metadata = req.metadata_mut();
        metadata.insert("authorization", token.parse().unwrap());
    }
    
    match client.get_index_stats(req).await {
        Ok(response) => {
            let response = response.into_inner();
            if !response.success {
                println!("错误: {}", response.message);
                return Ok(());
            }
            
            println!("索引统计信息:");
            println!("  索引总数: {}", response.total_indices);
            if !response.index_names.is_empty() {
                println!("  索引列表:");
                for index_name in response.index_names {
                    println!("    - {}", index_name);
                }
            }
        }
        Err(e) => {
            println!("❌ 获取索引统计信息失败: {}", e);
            println!("⚠️  全文索引服务可能未启用");
        }
    }
    
    Ok(())
}
