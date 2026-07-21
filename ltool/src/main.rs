//! ltool - LaoflchDB TUI 客户端入口
//!
//! 基于 ratatui + crossterm，提供图片 / 人脸 / 向量 / SQL / 索引 五个 Tab。
//! 通过 gRPC 连接 laoflchdb 服务（默认 127.0.0.1:19777）。

mod app;
mod grpc_client;
mod handler;
mod path_complete;
mod tab_face;
mod tab_image;
mod tab_index;
mod tab_sql;
mod tab_vector;
mod ui;

use std::io::{stdout, Stdout};
use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::App;
use crate::grpc_client::GrpcClients;

/// 命令行参数
#[derive(Parser, Debug)]
#[command(
    name = "ltool",
    version,
    about = "LaoflchDB TUI 客户端 - 图片/人脸/向量/SQL 可视化管理工具"
)]
struct Cli {
    /// 服务器地址（host:port）
    #[arg(long, default_value = "127.0.0.1:19777")]
    host: String,

    /// 用户名
    #[arg(short = 'u', long)]
    user: Option<String>,

    /// 密码
    #[arg(short = 'p', long)]
    password: Option<String>,

    /// 连接检查模式：非交互式验证 gRPC 连接和登录，然后退出
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // --check 模式：非交互式连接验证
    if cli.check {
        return run_check(&cli).await;
    }

    // 初始化终端
    enable_raw_mode().map_err(|e| anyhow!("启用 raw mode 失败: {}", e))?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)
        .map_err(|e| anyhow!("进入备用屏幕失败: {}", e))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| anyhow!("创建终端失败: {}", e))?;

    // 创建 App
    let username = cli.user.clone().unwrap_or_default();
    let password = cli.password.clone().unwrap_or_default();
    let mut app = App::new(cli.host.clone(), username, password);

    // 初始化 gRPC clients
    match GrpcClients::connect(&cli.host).await {
        Ok(clients) => app.clients = Some(clients),
        Err(e) => {
            app.set_error(format!("连接 {} 失败: {}", cli.host, e));
        }
    }

    // 如果提供了用户名/密码且连接成功，自动登录
    if let (Some(_), Some(_)) = (&cli.user, &cli.password) {
        if let Some(ref mut clients) = app.clients {
            let u = app.username.clone();
            let p = app.password.clone();
            match clients.login(&u, &p).await {
                Ok(()) => {
                    app.logged_in = true;
                    app.set_status(format!("登录成功：{}", u));
                }
                Err(e) => {
                    app.set_error(format!("登录失败: {}", e));
                }
            }
        }
    }

    // 主循环
    let result = run_loop(&mut terminal, &mut app).await;

    // 恢复终端
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture).ok();
    terminal.show_cursor().ok();

    result
}

/// 主事件循环
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        if app.should_quit {
            return Ok(());
        }

        // 绘制
        terminal.draw(|f| ui::draw(f, app))?;

        // 等待事件（200ms 超时，便于周期性刷新）
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) => {
                handler::handle_event(app, key).await;
            }
            Event::Mouse(mouse) => {
                handler::handle_mouse_event(app, mouse).await;
            }
            _ => {}
        }
    }
}

/// 非交互式连接检查
///
/// 验证 gRPC 连接、登录、各服务可用性，然后退出。用于 CI 和快速验证。
async fn run_check(cli: &Cli) -> Result<()> {
    println!("ltool 连接检查");
    println!("==================");

    // 1. 连接
    print!("[1/5] 连接 {} ... ", cli.host);
    let mut clients = GrpcClients::connect(&cli.host).await?;
    println!("OK");

    // 2. 登录（如果提供了凭据）
    if let (Some(u), Some(p)) = (&cli.user, &cli.password) {
        print!("[2/5] 登录 {} ... ", u);
        clients.login(u, p).await?;
        println!("OK (token 已保存)");
    } else {
        println!("[2/5] 跳过登录（未提供凭据）");
    }

    // 3. 主服务 - ListSchemas
    print!("[3/5] LaoflchDb.ListSchemas ... ");
    use laoflchdb_client::pb::rpc::ListSchemasRequest;
    let resp = clients
        .laoflchdb
        .list_schemas(ListSchemasRequest {})
        .await?
        .into_inner();
    println!("OK (schemas: {:?})", resp.schemas);

    // 4. 图片服务 - ListImages
    if clients.is_logged_in() {
        print!("[4/5] ImageService.ListImages (bucket=images) ... ");
        use laoflchdb_image_service_proto::proto::ListImagesRequest;
        let req = clients.auth_request(ListImagesRequest {
            bucket: "images".to_string(),
            prefix: String::new(),
            max_keys: 5,
            marker: String::new(),
        });
        let resp = clients.image.list_images(req).await?.into_inner();
        println!("OK ({} 张图片)", resp.images.len());
    } else {
        println!("[4/5] 跳过 ImageService（未登录）");
    }

    // 5. 向量服务 - GetIndexInfo
    if clients.is_logged_in() {
        print!("[5/5] EmbeddingIndexService.GetIndexInfo (index=face) ... ");
        use laoflchdb_embedding_service_proto::proto::GetIndexInfoRequest;
        let req = clients.auth_request(GetIndexInfoRequest {
            index_name: "face".to_string(),
        });
        let resp = clients.embedding.get_index_info(req).await?.into_inner();
        if resp.success {
            if let Some(s) = resp.stats {
                println!("OK (num_elements={}, dim={})", s.num_elements, s.dim);
            } else {
                println!("OK (空索引)");
            }
        } else {
            println!("OK (索引不存在: {})", resp.message);
        }
    } else {
        println!("[5/5] 跳过 EmbeddingIndexService（未登录）");
    }

    println!();
    println!("✓ 连接检查通过");
    Ok(())
}
