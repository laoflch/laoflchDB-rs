//! TUI 渲染
//!
//! 使用 ratatui 渲染主界面：顶部 Tab 栏、中间内容区、底部状态栏和命令行。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::{App, ImageFocus, PathPopup, Tab};

/// 主渲染入口
pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Tab 栏
            Constraint::Min(10),   // 内容区
            Constraint::Length(3), // 状态栏 / 命令行
        ])
        .split(f.size());

    draw_tabs(f, app, chunks[0]);

    // 各 Tab 渲染返回路径输入框的 area（用于补全弹窗锚定）
    let path_anchor: Option<Rect> = match app.current_tab {
        Tab::Image => draw_image_tab(f, app, chunks[1]),
        Tab::Face => draw_face_tab(f, app, chunks[1]),
        Tab::Vector => {
            draw_vector_tab(f, app, chunks[1]);
            None
        }
        Tab::Sql => {
            draw_sql_tab(f, app, chunks[1]);
            None
        }
    };

    draw_status_or_command(f, app, chunks[2]);

    // 路径补全弹窗最后渲染，浮在所有内容之上
    if let Some(anchor) = path_anchor {
        let popup = match app.current_tab {
            Tab::Image => &app.image_tab.path_popup,
            Tab::Face => &app.face_tab.path_popup,
            _ => unreachable!(),
        };
        if popup.is_active() {
            draw_path_popup(f, popup, anchor);
        }
    }
}

/// 绘制顶部 Tab 栏
fn draw_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let titles = vec!["1:图片", "2:人脸", "3:向量", "4:SQL"];
    let selected = app.current_tab.index();

    let spans: Vec<Span> = titles
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if i == selected {
                Span::styled(
                    format!(" {} ", t),
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {} ", t), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title("ltool");
    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(paragraph, area);
}

/// 绘制底部状态栏或命令输入行
fn draw_status_or_command(f: &mut Frame, app: &mut App, area: Rect) {
    if app.command_mode.active {
        // 命令行输入
        let input = &app.command_mode.input;
        let line = Line::from(vec![
            Span::styled(": ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(&input.value),
        ]);
        let block = Block::default().borders(Borders::ALL).title("命令");
        let p = Paragraph::new(line).block(block);
        f.render_widget(p, area);

        // 设置光标位置
        let cursor_x = area.x + 1 + 2 + input.value.chars().take(input.cursor).map(|c| c.len_utf8()).count() as u16;
        let cursor_y = area.y + 1;
        f.set_cursor(cursor_x, cursor_y);
        return;
    }

    // 状态栏：登录状态 | 连接状态 | 当前 Tab | 消息
    let login_span = if app.logged_in {
        Span::styled(
            format!("用户: {} ", app.username),
            Style::default().fg(Color::Green),
        )
    } else {
        Span::styled("未登录 ", Style::default().fg(Color::Red))
    };
    let conn_span = Span::styled(
        format!("{} ", app.host),
        Style::default().fg(Color::Cyan),
    );
    let tab_span = Span::styled(
        format!("[{}] ", app.current_tab.title()),
        Style::default().fg(Color::Yellow),
    );
    // 当前 Tab 的快捷键提示
    let help_text = match app.current_tab {
        Tab::Image => "F1上传 F2列出 :bucket/:key设置 ↑↓选路径 Enter确认 Esc取消 | ",
        Tab::Face => "F1提取 F2比较 F4保存 F5索引 ↑↓选路径 Enter确认 Esc取消 | ",
        Tab::Vector => "F1索引信息 F2搜索 F3删除 | ",
        Tab::Sql => "F5执行 Ctrl+L清空 | ",
    };
    let help_span = Span::styled(help_text, Style::default().fg(Color::DarkGray));

    // 图片 Tab 在状态栏显示 bucket/key（留空表示自动生成）
    let ctx_span = if app.current_tab == Tab::Image {
        Span::styled(
            format!(
                "bucket={} key={} | ",
                app.image_tab.bucket.value,
                if app.image_tab.key.value.is_empty() { "(自动)" } else { &app.image_tab.key.value }
            ),
            Style::default().fg(Color::Magenta),
        )
    } else {
        Span::raw("")
    };

    let msg_color = if app.status_is_error {
        Color::Red
    } else {
        Color::Green
    };
    let msg_span = Span::styled(&app.status_message, Style::default().fg(msg_color));

    let line = Line::from(vec![login_span, conn_span, tab_span, help_span, ctx_span, msg_span]);
    let block = Block::default().borders(Borders::ALL);
    let p = Paragraph::new(line).block(block);
    f.render_widget(p, area);
}

// ── 图片 Tab ──────────────────────────────────────

fn draw_image_tab(f: &mut Frame, app: &mut App, area: Rect) -> Option<Rect> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    // 文件路径输入框独占一整行
    let path_area = chunks[0];
    draw_input_box(
        f,
        path_area,
        "本地文件路径",
        &app.image_tab.file_path,
        app.image_tab.focus == ImageFocus::FilePath,
    );

    // 下半部：结果区
    let result_area = chunks[1];
    let rows: Vec<Row> = app
        .image_tab
        .images
        .iter()
        .skip(app.image_tab.list_scroll)
        .take(50)
        .map(|m| {
            Row::new(vec![
                Cell::from(m.key.clone()),
                Cell::from(m.content_type.clone()),
                Cell::from(m.content_length.to_string()),
                Cell::from(format!("{}x{}", m.width, m.height)),
                Cell::from(m.last_modified.clone()),
            ])
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("key"),
        Cell::from("content_type"),
        Cell::from("size"),
        Cell::from("WxH"),
        Cell::from("last_modified"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    // 如果有上传结果，附加到顶部
    let title = if let Some(ref r) = app.image_tab.upload_result {
        format!("图片列表  | 上传结果: {}", truncate_str(r, 60))
    } else {
        "图片列表".to_string()
    };

    let table = Table::new(rows, [Constraint::Length(20), Constraint::Length(20), Constraint::Length(10), Constraint::Length(12), Constraint::Min(10)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, result_area);

    Some(path_area)
}

// ── 人脸 Tab ──────────────────────────────────────

fn draw_face_tab(f: &mut Frame, app: &mut App, area: Rect) -> Option<Rect> {
    use crate::app::FaceFocus;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    // 输入区：分两行
    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(40), Constraint::Length(15), Constraint::Length(15)])
        .split(chunks[0]);

    let path_area = row1[0];
    draw_input_box(f, path_area, "本地图片路径", &app.face_tab.file_path, app.face_tab.focus == FaceFocus::FilePath);
    draw_input_box(f, row1[1], "det_threshold", &app.face_tab.det_threshold, app.face_tab.focus == FaceFocus::DetThreshold);
    draw_input_box(f, row1[2], "max_faces", &app.face_tab.max_faces, app.face_tab.focus == FaceFocus::MaxFaces);

    // 第二行：bucket + 复选框提示
    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Length(20), Constraint::Length(20)])
        .split({
            let r = Rect { x: chunks[0].x, y: chunks[0].y + 3, width: chunks[0].width, height: 3 };
            r
        });

    draw_input_box(f, row2[0], "bucket", &app.face_tab.bucket, app.face_tab.focus == FaceFocus::Bucket);

    let save_str = if app.face_tab.save_aligned_images { "[x]" } else { "[ ]" };
    let p1 = Paragraph::new(format!("{} save_aligned (F4 切换)", save_str))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p1, row2[1]);

    let idx_str = if app.face_tab.index_embedding { "[x]" } else { "[ ]" };
    let p2 = Paragraph::new(format!("{} index_embedding (F5 切换)", idx_str))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p2, row2[2]);

    // 结果区
    let rows: Vec<Row> = app
        .face_tab
        .faces
        .iter()
        .skip(app.face_tab.list_scroll)
        .take(50)
        .map(|(i, score, bbox, key, vid)| {
            let bbox_str = if bbox.len() >= 4 {
                format!("[{:.0},{:.0},{:.0},{:.0}]", bbox[0], bbox[1], bbox[2], bbox[3])
            } else {
                "-".to_string()
            };
            Row::new(vec![
                Cell::from(i.to_string()),
                Cell::from(format!("{:.4}", score)),
                Cell::from(bbox_str),
                Cell::from(key.clone()),
                Cell::from(if *vid == 0 { "-".to_string() } else { vid.to_string() }),
            ])
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("#"),
        Cell::from("score"),
        Cell::from("bbox"),
        Cell::from("saved_image_key"),
        Cell::from("vector_id"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    // 把 embedding 预览也显示在 title 里
    let preview: String = app
        .face_tab
        .embedding_preview
        .iter()
        .take(10)
        .map(|v| format!("{:.4}", v))
        .collect::<Vec<_>>()
        .join(", ");
    let title = format!("人脸列表  | embedding 前10: [{}]", preview);

    let table = Table::new(rows, [Constraint::Length(5), Constraint::Length(12), Constraint::Length(30), Constraint::Length(20), Constraint::Length(12)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, chunks[1]);

    Some(path_area)
}

// ── 向量 Tab ──────────────────────────────────────

fn draw_vector_tab(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::app::VectorFocus;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    let row1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20), Constraint::Length(10), Constraint::Length(60)])
        .split(chunks[0]);

    draw_input_box(f, row1[0], "index_name", &app.vector_tab.index_name, app.vector_tab.focus == VectorFocus::IndexName);
    draw_input_box(f, row1[1], "top_k", &app.vector_tab.top_k, app.vector_tab.focus == VectorFocus::TopK);
    draw_input_box(f, row1[2], "查询向量（逗号分隔）", &app.vector_tab.query_vec, app.vector_tab.focus == VectorFocus::QueryVec);

    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(20)])
        .split({
            Rect { x: chunks[0].x, y: chunks[0].y + 3, width: chunks[0].width, height: 3 }
        });
    draw_input_box(f, row2[0], "删除向量 ID", &app.vector_tab.delete_id, app.vector_tab.focus == VectorFocus::DeleteId);

    // 索引信息
    let info_str = if let Some((n, dim, metric, layers)) = &app.vector_tab.index_info {
        format!("num_elements={}, dim={}, metric={}, max_layers={}", n, dim, metric, layers)
    } else {
        "(尚未获取索引信息，按 i 获取)".to_string()
    };

    // 搜索结果
    let rows: Vec<Row> = app
        .vector_tab
        .search_results
        .iter()
        .skip(app.vector_tab.list_scroll)
        .take(50)
        .map(|(id, dist)| Row::new(vec![Cell::from(id.to_string()), Cell::from(format!("{:.4}", dist))]))
        .collect();
    let header = Row::new(vec![Cell::from("id"), Cell::from("distance")])
        .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    let table = Table::new(rows, [Constraint::Length(20), Constraint::Length(20)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(format!("搜索结果  | {}", info_str)));

    f.render_widget(table, chunks[1]);
}

// ── SQL Tab ──────────────────────────────────────

fn draw_sql_tab(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(8), Constraint::Min(5)])
        .split(area);

    // schema 输入框
    draw_input_box(f, chunks[0], "schema", &app.sql_tab.schema, !app.sql_tab.focus_sql);

    // SQL 输入区
    let sql_block = Block::default()
        .borders(Borders::ALL)
        .title("SQL（F5 或 Enter 执行，Ctrl+L 清空）")
        .style(if app.sql_tab.focus_sql {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        });
    let p = Paragraph::new(app.sql_tab.sql.value.clone()).block(sql_block);
    f.render_widget(p, chunks[1]);
    if app.sql_tab.focus_sql {
        let cursor = app.sql_tab.sql.cursor;
        // 计算光标在多行中的位置
        let mut x = chunks[1].x + 1;
        let mut y = chunks[1].y + 1;
        for (i, c) in app.sql_tab.sql.value.chars().enumerate() {
            if i >= cursor {
                break;
            }
            if c == '\n' {
                x = chunks[1].x + 1;
                y += 1;
            } else {
                x += 1;
            }
        }
        f.set_cursor(x, y);
    }

    // 结果表格
    let header = Row::new(app.sql_tab.columns.iter().map(|c| Cell::from(c.clone())).collect::<Vec<_>>())
        .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    let rows: Vec<Row> = app
        .sql_tab
        .rows
        .iter()
        .skip(app.sql_tab.list_scroll)
        .take(100)
        .map(|r| Row::new(r.iter().map(|c| Cell::from(truncate_str(c, 50))).collect::<Vec<_>>()))
        .collect();

    let constraints = if app.sql_tab.columns.is_empty() {
        vec![Constraint::Min(10)]
    } else {
        app.sql_tab.columns.iter().map(|_| Constraint::Length(20)).collect()
    };

    let table = Table::new(rows, constraints)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(format!("查询结果（{} 行）", app.sql_tab.rows.len())));

    f.render_widget(table, chunks[2]);
}

// ── 通用辅助 ──────────────────────────────────────

/// 绘制带边框的输入框
fn draw_input_box(
    f: &mut Frame,
    area: Rect,
    title: &str,
    input: &crate::app::InputState,
    focused: bool,
) {
    let style = if focused {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let block = Block::default().borders(Borders::ALL).title(title).style(style);
    let p = Paragraph::new(input.value.clone())
        .block(block)
        .alignment(Alignment::Left);
    f.render_widget(p, area);

    if focused {
        // 设置光标
        let cursor_x = area.x + 1 + input.value.chars().take(input.cursor).map(|c| c.len_utf8()).count() as u16;
        let cursor_y = area.y + 1;
        // 防止光标超出区域
        if cursor_x < area.x + area.width {
            f.set_cursor(cursor_x, cursor_y);
        }
    }
}

/// 截断字符串到最大字符数
fn truncate_str(s: &str, max_chars: usize) -> String {
    let n = s.chars().count();
    if n <= max_chars {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_chars - 3).collect();
        t.push_str("...");
        t
    }
}

/// 绘制路径补全下拉菜单（浮在顶层，深灰底，图片项显示缩略图）
///
/// 锚定在路径输入框的正下方，每项占 THUMB_H 行（为缩略图腾空间），最多显示 6 项。
/// 必须在所有其他渲染之后调用，确保浮在最顶层。
fn draw_path_popup(f: &mut Frame, popup: &PathPopup, anchor: Rect) {
    use crate::app::{THUMB_H, THUMB_W};
    use ratatui_image::Image;

    const MAX_VISIBLE: usize = 6;
    let total = popup.candidates.len();
    let visible = total.min(MAX_VISIBLE);
    if visible == 0 {
        return;
    }

    // 每项占 THUMB_H 行；弹窗高度 = visible * THUMB_H + 2（边框）
    let row_h = THUMB_H;
    let width = anchor.width.max(40).min(80);
    let height = (visible as u16) * row_h + 2;
    let x = anchor.x;
    let y_below = anchor.y + anchor.height;
    let y = if y_below + height <= f.size().bottom() {
        y_below
    } else if anchor.y >= height {
        anchor.y - height
    } else {
        y_below
    };
    let area = Rect { x, y, width, height };

    // 清除背景
    f.render_widget(Clear, area);

    // 外框
    let title = format!(
        " 路径补全 {}/{}  📁目录 🖼图片 ",
        popup.selected + 1,
        total
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(Color::Black).fg(Color::Yellow));
    f.render_widget(block, area);

    // 内部内容区（去掉边框）
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    // 为每个可见项分配一个行 Rect
    let row_areas: Vec<Rect> = (0..visible)
        .map(|i| Rect {
            x: inner.x,
            y: inner.y + (i as u16) * row_h,
            width: inner.width,
            height: row_h,
        })
        .collect();

    for (i, c) in popup
        .candidates
        .iter()
        .skip(popup.scroll)
        .take(MAX_VISIBLE)
        .enumerate()
    {
        let idx_in_all = popup.scroll + i;
        let is_selected = idx_in_all == popup.selected;
        let row_area = row_areas[i];

        // 选中行背景高亮
        if is_selected {
            f.render_widget(
                ratatui::widgets::Clear,
                row_area,
            );
            // 用 Block 不带边框填充背景色
            let bg = Block::default().style(Style::default().bg(Color::Cyan));
            f.render_widget(bg, row_area);
        }

        // 左侧缩略图区域（THUMB_W 列宽）
        let thumb_area = Rect {
            x: row_area.x,
            y: row_area.y,
            width: THUMB_W,
            height: row_h,
        };

        // 右侧文本区域
        let text_area = Rect {
            x: row_area.x + THUMB_W,
            y: row_area.y,
            width: row_area.width.saturating_sub(THUMB_W),
            height: row_h,
        };

        // 渲染缩略图或类型图标
        if c.is_image {
            if let Some(hb) = popup.get_thumbnail(&c.full_path) {
                let img_widget = Image::new(hb);
                f.render_widget(img_widget, thumb_area);
            } else {
                // 加载失败或未就绪：显示占位
                let placeholder = Paragraph::new("🖼")
                    .style(Style::default().fg(Color::DarkGray))
                    .alignment(Alignment::Center);
                f.render_widget(placeholder, thumb_area);
            }
        } else if c.is_dir {
            // 目录图标居中
            let icon = Paragraph::new("📁\n  ")
                .style(
                    Style::default()
                        .fg(if is_selected { Color::Black } else { Color::Blue })
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center);
            f.render_widget(icon, thumb_area);
        }

        // 渲染文本区域：序号 + 文件名（垂直居中）
        let (name_fg, name_bold) = if c.is_dir {
            (if is_selected { Color::Black } else { Color::Blue }, true)
        } else if c.is_image {
            (if is_selected { Color::Black } else { Color::White }, is_selected)
        } else {
            (if is_selected { Color::Black } else { Color::White }, false)
        };

        // 截断文件名以适应文本区宽度
        let avail = text_area.width as usize;
        let name_display = truncate_str(&c.display, avail);
        // 序号（小号暗色）
        let num_str = format!("{}", idx_in_all + 1);
        let num_span = Span::styled(
            num_str,
            Style::default()
                .fg(if is_selected { Color::Black } else { Color::DarkGray }),
        );
        let name_span = Span::styled(
            name_display,
            Style::default()
                .fg(name_fg)
                .add_modifier(if name_bold { Modifier::BOLD } else { Modifier::empty() }),
        );

        // 垂直居中：在 text_area 中间行显示
        let mid_y = text_area.y + text_area.height / 2;
        let line = Line::from(vec![num_span, Span::raw(" "), name_span]);
        // 在 text_area 中间行渲染：手动设置 buffer
        // 用 Paragraph 限制高度为 1，放在中间行
        let single_line_area = Rect {
            x: text_area.x,
            y: mid_y,
            width: text_area.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(line), single_line_area);
    }
}
