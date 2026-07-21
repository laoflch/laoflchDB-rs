//! TUI 渲染
//!
//! 使用 ratatui 渲染主界面：顶部 Tab 栏、中间内容区、底部状态栏和命令行。

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, Wrap};
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

    // 各 Tab 渲染
    let image_path_anchor: Option<Rect>;
    let face_path_anchor: Option<Rect>;
    let vector_input_anchor: Option<Rect>;

    match app.current_tab {
        Tab::Image => {
            image_path_anchor = draw_image_tab(f, app, chunks[1]);
            face_path_anchor = None;
            vector_input_anchor = None;
        }
        Tab::Face => {
            face_path_anchor = draw_face_tab(f, app, chunks[1]);
            image_path_anchor = None;
            vector_input_anchor = None;
        }
        Tab::Vector => {
            vector_input_anchor = Some(draw_vector_tab(f, app, chunks[1]));
            image_path_anchor = None;
            face_path_anchor = None;
        }
        Tab::Sql => {
            draw_sql_tab(f, app, chunks[1]);
            image_path_anchor = None;
            face_path_anchor = None;
            vector_input_anchor = None;
        }
    };

    draw_status_or_command(f, app, chunks[2]);

    // 路径补全弹窗最后渲染，浮在所有内容之上
    // 弹窗从路径输入框下方延伸到内容区底部（chunks[1] 的底部）
    let content_bottom = chunks[1].y + chunks[1].height;

    if let Some(anchor) = image_path_anchor {
        let max_visible = (content_bottom.saturating_sub(anchor.y + anchor.height)) as usize;
        app.image_tab.path_popup.visible = max_visible;
        let popup = &app.image_tab.path_popup;
        if popup.is_active() {
            draw_path_popup(f, popup, anchor, content_bottom);
        }
    }

    if let Some(anchor) = face_path_anchor {
        let max_visible = (content_bottom.saturating_sub(anchor.y + anchor.height)) as usize;
        app.face_tab.path_popup.visible = max_visible;
        let popup = &app.face_tab.path_popup;
        if popup.is_active() {
            draw_path_popup(f, popup, anchor, content_bottom);
        }
    }

    // 本地文件操作弹窗（浮在最顶层）
    if app.image_tab.local_file_action.is_some() {
        draw_local_file_action_dialog(f, app);
    }

    // 向量搜索结果弹窗（浮在最顶层）
    if app.image_tab.show_search_results {
        draw_search_results_popup(f, app);
    }

    // 图片操作弹窗（浮在最顶层）
    if app.image_tab.action_popup_open {
        draw_image_action_popup(f, app);
    }

    // 删除确认弹窗
    if let Some(ref key) = app.image_tab.delete_confirm {
        draw_delete_confirm_dialog(f, key);
    }

    // 下载确认弹窗
    if app.image_tab.download_confirm.is_some() {
        draw_download_confirm_dialog(f, app);
    }

    // ── 人脸 Tab 已保存人脸列表弹窗 ──
    if app.current_tab == Tab::Face && app.face_tab.show_saved {
        draw_face_saved_list(f, app);
    }
    // 人脸 Tab 操作弹窗
    if app.face_tab.saved_action_open {
        draw_face_saved_action_popup(f, app);
    }
    // 人脸 Tab 删除确认
    if let Some(ref key) = app.face_tab.saved_delete_key {
        draw_face_delete_confirm_dialog(f, key);
    }

    // 向量索引导航下拉菜单（浮在最顶层）
    if app.vector_tab.show_dropdown && !app.vector_tab.all_indices.is_empty() {
        if let Some(anchor) = vector_input_anchor {
            draw_index_dropdown_popup(f, app, anchor, content_bottom);
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
        Tab::Face => "F1提取 F3列表 F4保存 F5索引 F6导出 ↑↓选路径 Enter确认 Esc取消  ",
        Tab::Vector => "F1刷新列表 F2/Enter查看详情 ↓/Tab展开菜单 ↑↓导航 Esc关闭 | ",
        Tab::Sql => "F5执行 Ctrl+L清空 | ",
    };
    let help_span = Span::styled(help_text, Style::default().fg(Color::Gray));

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

    // 分割为表格区域和滚动条区域
    let result_chunks = Layout::horizontal([
        Constraint::Min(1),
        Constraint::Length(1),
    ]).split(result_area);
    let table_area = result_chunks[0];
    let scrollbar_area = result_chunks[1];

    let rows: Vec<Row> = app
        .image_tab
        .images
        .iter()
        .enumerate()
        .skip(app.image_tab.list_scroll)
        .take(50)
        .map(|(i, m)| {
            let cells = vec![
                Cell::from(truncate_str(&m.key, 18)),
                Cell::from(m.content_type.clone()),
                Cell::from(m.content_length.to_string()),
                Cell::from(format!("{}x{}", m.width, m.height)),
                Cell::from(format_timestamp(&m.last_modified)),
                Cell::from(if m.name.is_empty() { m.key.clone() } else { m.name.clone() }),
            ];
            // 选中行高亮
            if Some(i) == app.image_tab.selected_index {
                Row::new(cells).style(Style::default().bg(Color::Green).fg(Color::Black))
            } else {
                Row::new(cells)
            }
        })
        .collect();

    let header = Row::new(vec![
        Cell::from("key"),
        Cell::from("content_type"),
        Cell::from("size"),
        Cell::from("WxH"),
        Cell::from("last_modified"),
        Cell::from("name"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    // 如果有上传结果，附加到顶部
    let title = if let Some(ref r) = app.image_tab.upload_result {
        format!("图片列表  | 上传结果: {}", truncate_str(r, 60))
    } else {
        "图片列表".to_string()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(18),
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(21),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, table_area);

    // 滚动条
    let total = app.image_tab.images.len();
    let visible = 50; // 与 take(50) 一致
    if total > visible {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_symbol("█")
            .track_symbol(Some("░"))
            .style(Style::default().fg(Color::DarkGray));
        let mut state = ScrollbarState::new(total).position(app.image_tab.list_scroll);
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut state);
    }

    Some(path_area)
}

// ── 人脸 Tab ──────────────────────────────────────

fn draw_face_tab(f: &mut Frame, app: &mut App, area: Rect) -> Option<Rect> {
    use crate::app::FaceFocus;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    // 第一行：本地图片路径（独占一行，全宽）
    let path_area = Rect { x: chunks[0].x, y: chunks[0].y, width: chunks[0].width, height: 3 };
    draw_input_box(f, path_area, "本地图片路径", &app.face_tab.file_path, app.face_tab.focus == FaceFocus::FilePath);

    // 第二行：参数（det_threshold, max_faces, bucket, 复选框）共用一行
    let row2 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(20),
            Constraint::Length(20),
            Constraint::Length(20),
        ])
        .split(Rect {
            x: chunks[0].x,
            y: chunks[0].y + 3,
            width: chunks[0].width,
            height: 3,
        });

    draw_input_box(f, row2[0], "det_threshold", &app.face_tab.det_threshold, app.face_tab.focus == FaceFocus::DetThreshold);
    draw_input_box(f, row2[1], "max_faces", &app.face_tab.max_faces, app.face_tab.focus == FaceFocus::MaxFaces);
    draw_input_box(f, row2[2], "bucket", &app.face_tab.bucket, app.face_tab.focus == FaceFocus::Bucket);

    let save_str = if app.face_tab.save_aligned_images { "[x]" } else { "[ ]" };
    let p1 = Paragraph::new(format!("{} save_aligned (F4)", save_str))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p1, row2[3]);

    let idx_str = if app.face_tab.index_embedding { "[x]" } else { "[ ]" };
    let p2 = Paragraph::new(format!("{} index_embedding (F5)", idx_str))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(p2, row2[4]);

    // 第三行：导出路径（独占一行，全宽，和本地路径一致）
    let row3 = Rect { x: chunks[0].x, y: chunks[0].y + 6, width: chunks[0].width, height: 3 };
    draw_input_box(f, row3, "导出路径", &app.face_tab.export_path, app.face_tab.focus == FaceFocus::ExportPath);

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

    let table = Table::new(rows, [Constraint::Length(5), Constraint::Length(12), Constraint::Length(30), Constraint::Length(25), Constraint::Length(20)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, chunks[1]);

    Some(path_area)
}

/// 绘制已保存人脸列表（F3 弹窗）
fn draw_face_saved_list(f: &mut Frame, app: &mut App) {
    let area = f.size();
    let width = area.width.saturating_sub(4).min(80);
    let height = (area.height.saturating_sub(4)).min(30);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let count = app.face_tab.saved_faces.len();
    let title = format!("已保存人脸列表 (共 {} 张)  ↑↓导航 Enter菜单 Esc关闭", count);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(Color::Black).fg(Color::White));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    let header = Row::new(vec![
        Cell::from("key"),
        Cell::from("size"),
        Cell::from("WxH"),
        Cell::from("format"),
        Cell::from("上传时间"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    let rows: Vec<Row> = app
        .face_tab
        .saved_faces
        .iter()
        .skip(app.face_tab.saved_scroll)
        .take(50)
        .enumerate()
        .map(|(i, meta)| {
            let abs_idx = i + app.face_tab.saved_scroll;
            let selected = app.face_tab.saved_selected == Some(abs_idx);
            let cells = vec![
                Cell::from(meta.key.clone()),
                Cell::from(meta.content_length.to_string()),
                Cell::from(format!("{}x{}", meta.width, meta.height)),
                Cell::from(meta.format.clone()),
                Cell::from(format_timestamp(&meta.last_modified)),
            ];
            if selected {
                Row::new(cells).style(Style::default().bg(Color::Green).fg(Color::Black))
            } else {
                Row::new(cells)
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(30),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(20),
        ],
    )
    .header(header);

    f.render_widget(table, inner);
}

/// 已保存人脸操作弹窗
fn draw_face_saved_action_popup(f: &mut Frame, app: &mut App) {
    const FACE_ACTION_OPTIONS: &[&str] = &["导出人脸", "删除人脸"];
    let area = f.size();
    let width = 30;
    let height = FACE_ACTION_OPTIONS.len() as u16 + 3;
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("人脸操作")
        .style(Style::default().bg(Color::Black).fg(Color::White));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    for (i, opt) in FACE_ACTION_OPTIONS.iter().enumerate() {
        let selected = i == app.face_tab.saved_action_selected;
        let style = if selected {
            Style::default().bg(Color::Green).fg(Color::Black)
        } else {
            Style::default().fg(Color::White)
        };
        let prefix = if selected { "▶ " } else { "  " };
        let line = Paragraph::new(Line::from(Span::styled(
            format!("{}{}", prefix, opt),
            style,
        )));
        f.render_widget(line, Rect {
            x: inner.x,
            y: inner.y + i as u16,
            width: inner.width,
            height: 1,
        });
    }
}

/// 已保存人脸删除确认弹窗
fn draw_face_delete_confirm_dialog(f: &mut Frame, key: &str) {
    let area = f.size();
    let width = 50.min(area.width.saturating_sub(4));
    let height = 5;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("确认删除")
        .style(Style::default().bg(Color::Black).fg(Color::Red));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    let msg = Paragraph::new(format!("确认删除人脸 {} ?\nEnter 确认  Esc 取消", key))
        .wrap(Wrap { trim: false });
    f.render_widget(msg, inner);
}

// ── 向量 Tab ──────────────────────────────────────

fn draw_vector_tab(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);

    // ── 顶部：索引名称输入框 ─────────────────────
    let input_area = chunks[0];
    draw_input_box(f, input_area, "索引名称（F1 刷新列表，F2/Enter 查看详情）", &app.vector_tab.index_name, true);

    // ── 下部：索引信息详情 ─────────────────────
    let info = if !app.vector_tab.index_name.value.is_empty() {
        app.vector_tab.all_indices.iter().find(|i| i.name == app.vector_tab.index_name.value)
    } else {
        app.vector_tab.all_indices.first()
    };

    let header = Row::new(vec![
        Cell::from("属性"),
        Cell::from("值"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan));

    let mut rows = Vec::new();

    // 头部信息
    if let Some(info) = info {
        rows.push(Row::new(vec![
            Cell::from("索引名称"),
            Cell::from(info.name.clone()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("向量数"),
            Cell::from(info.num_elements.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("维度"),
            Cell::from(info.dim.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("距离度量"),
            Cell::from(info.distance_metric.clone()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("最大层数"),
            Cell::from(info.max_layers.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("搜索次数"),
            Cell::from(info.search_count.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("插入次数"),
            Cell::from(info.insert_count.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("删除次数"),
            Cell::from(info.delete_count.to_string()),
        ]));
        rows.push(Row::new(vec![
            Cell::from("快照路径"),
            Cell::from(info.snapshot_path.clone()),
        ]));
    } else {
        let hint = if app.vector_tab.all_indices.is_empty() {
            "按 F1 获取所有索引列表，或在输入框输入索引名称后按 Enter/F2 查看详情"
        } else {
            "按 ↓/Tab 展开下拉菜单选择索引，或输入索引名称后按 Enter/F2 查看详情"
        };
        rows.push(Row::new(vec![
            Cell::from(hint),
            Cell::from(""),
        ]));
    }

    let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(20)])
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("索引信息"));

    f.render_widget(table, chunks[1]);

    input_area
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

/// 绘制路径补全下拉菜单（浮在顶层，延伸至内容区底部）
///
/// 锚定在路径输入框的正下方，高度延伸到内容区底部（content_bottom），
/// 最多显示所有候选；右侧显示滚动条。
/// 必须在所有其他渲染之后调用，确保浮在最顶层。
fn draw_path_popup(f: &mut Frame, popup: &PathPopup, anchor: Rect, content_bottom: u16) {
    let total = popup.candidates.len();
    if total == 0 {
        return;
    }

    // 弹窗宽度 = 输入框宽度，最小 40
    let width = anchor.width.max(40).min(80);
    let x = anchor.x;

    // 底部可用的最大行数（内容区底部 - 输入框下方）
    let max_rows = content_bottom.saturating_sub(anchor.y + anchor.height);
    // 实际显示行数 = min(候选数, 可用行数)
    let visible = total.min(max_rows as usize);
    if visible == 0 {
        return;
    }

    let height = visible as u16 + 2; // 每项 1 行 + 边框
    let y = anchor.y + anchor.height; // 始终在输入框下方
    let area = Rect { x, y, width, height };

    // 清除背景
    f.render_widget(Clear, area);

    // 外框（黑色背景 + 黄色边框）
    let title = format!(" 路径补全 {}/{} ", popup.selected + 1, total);
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

    // 右侧留 1 列给滚动条
    let list_width = inner.width.saturating_sub(1);
    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: list_width,
        height: inner.height,
    };
    let scrollbar_area = Rect {
        x: inner.x + list_width,
        y: inner.y,
        width: 1,
        height: inner.height,
    };

    // 渲染每一行
    for i in 0..visible {
        let idx = popup.scroll + i;
        let c = &popup.candidates[idx];
        let is_selected = idx == popup.selected;

        let row_area = Rect {
            x: list_area.x,
            y: list_area.y + i as u16,
            width: list_area.width,
            height: 1,
        };

        // 选中行高亮背景
        if is_selected {
            let bg = Block::default().style(Style::default().bg(Color::Cyan));
            f.render_widget(bg, row_area);
        }

        // 文本
        let (fg, bold) = if is_selected {
            (Color::Black, true)
        } else if c.is_dir {
            (Color::Blue, true)
        } else {
            (Color::White, false)
        };

        let num_str = format!("{}", idx + 1);
        let name_display = truncate_str(&c.display, row_area.width as usize - 4);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(num_str, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(
                    name_display,
                    Style::default()
                        .fg(fg)
                        .add_modifier(if bold { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ])),
            row_area,
        );
    }

    // 滚动条（仅当候选数超过可见行数时显示）
    if total > visible {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_symbol("█")
            .track_symbol(Some("░"))
            .style(Style::default().fg(Color::DarkGray));
        let mut state = ScrollbarState::new(total).position(popup.scroll);
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut state);
    }
}

/// 绘制向量索引选择下拉菜单（浮在顶层，延伸至内容区底部）
///
/// 锚定在索引名称输入框的正下方，高度延伸到内容区底部（content_bottom）。
/// 必须在所有其他渲染之后调用，确保浮在最顶层。
fn draw_index_dropdown_popup(f: &mut Frame, app: &App, anchor: Rect, content_bottom: u16) {
    let total = app.vector_tab.all_indices.len();
    if total == 0 {
        return;
    }

    // 弹窗宽度 = 输入框宽度，最小 40
    let width = anchor.width.max(40).min(60);
    let x = anchor.x;

    // 底部可用的最大行数
    let max_rows = content_bottom.saturating_sub(anchor.y + anchor.height);
    let visible = total.min(max_rows as usize).min(10); // 最多显示10个
    if visible == 0 {
        return;
    }

    let height = visible as u16 + 2;
    let y = anchor.y + anchor.height;
    let area = Rect { x, y, width, height };

    // 清除背景
    f.render_widget(Clear, area);

    // 外框（黑色背景 + 黄色边框，与路径补全一致）
    let title = format!(" 选择索引 {}/{} ", app.vector_tab.selected_dropdown + 1, total);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().bg(Color::Black).fg(Color::Yellow));
    f.render_widget(block, area);

    // 内部内容区
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    // 渲染每一行
    for i in 0..visible {
        let idx = i;
        let info = &app.vector_tab.all_indices[idx];
        let is_selected = idx == app.vector_tab.selected_dropdown;

        let row_area = Rect {
            x: inner.x,
            y: inner.y + i as u16,
            width: inner.width,
            height: 1,
        };

        // 选中行高亮背景（青色背景 + 黑色文字，与路径补全一致）
        if is_selected {
            let bg = Block::default().style(Style::default().bg(Color::Cyan));
            f.render_widget(bg, row_area);
        }

        let name_display = truncate_str(&info.name, row_area.width as usize - 2);
        let fg = if is_selected { Color::Black } else { Color::White };
        let bold = is_selected;

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    name_display,
                    Style::default()
                        .fg(fg)
                        .add_modifier(if bold { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ])),
            row_area,
        );
    }
}

/// 绘制本地文件操作弹窗
///
/// 包含两个 Tab：上传（Tab 0）和向量索引（Tab 1）。
/// 向量搜索 Tab 中 Dim/TopK/距离≤ 字段可编辑，Tab 切换焦点，↑/↓ 切换模型。
fn draw_local_file_action_dialog(f: &mut Frame, app: &mut App) {
    let action = match &app.image_tab.local_file_action {
        Some(a) => a,
        None => return,
    };

    let area = f.size();
    let width = 60.min(area.width.saturating_sub(4));
    // 向量搜索 Tab：上边框 + Tab栏 + 文件行 + 模型行 + 输入框(3行) + 提示行 + 下边框 = 10
    // 上传 Tab：上边框 + Tab栏 + 文件行 + 空行 + 提示行 + 下边框 = 7
    let height = if action.tab == 1 { 10 } else { 7 };
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    // 清除背景
    f.render_widget(Clear, dialog_area);

    // 外框
    let title = format!("本地文件操作  [{}]", action.file_path);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(truncate_str(&title, dialog_area.width as usize - 2))
        .style(Style::default().bg(Color::Black).fg(Color::White));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    // ── Tab 栏 ──────────────────────────────────
    let tab_titles = ["上传", "图片检索"];
    let tab_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    };
    let tab_spans: Vec<Span> = tab_titles.iter().enumerate().map(|(i, t)| {
        let selected = i == action.tab;
        let sep = if i == 0 { "" } else { "  " };
        if selected {
            Span::styled(
                format!("{}[ {} ]", sep, t),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(
                format!("{} {} ", sep, t),
                Style::default().fg(Color::DarkGray),
            )
        }
    }).collect();
    f.render_widget(Paragraph::new(Line::from(tab_spans)), tab_area);

    // ── Tab 内容区 ──────────────────────────────
    let content_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(3),
    };

    match action.tab {
        0 => {
            // 上传 Tab
            let path_display = truncate_str(&action.file_path, content_area.width as usize);
            let path_line = Paragraph::new(Line::from(vec![
                Span::styled("文件: ", Style::default().fg(Color::Cyan)),
                Span::raw(path_display),
            ]));
            f.render_widget(path_line, Rect {
                x: content_area.x,
                y: content_area.y,
                width: content_area.width,
                height: 1,
            });

            let hint = Paragraph::new(Line::from(vec![
                Span::styled("Enter ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("确认上传  "),
                Span::styled("Esc ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("取消  "),
                Span::styled("Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("切换到向量索引"),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(hint, Rect {
                x: content_area.x,
                y: content_area.y + 3,
                width: content_area.width,
                height: 1,
            });
        }
        1 => {
            // 向量搜索 Tab：显示可编辑的输入框
            let path_display = truncate_str(&action.file_path, content_area.width as usize);
            let path_line = Paragraph::new(Line::from(vec![
                Span::styled("文件: ", Style::default().fg(Color::Cyan)),
                Span::raw(path_display),
            ]));
            f.render_widget(path_line, Rect {
                x: content_area.x,
                y: content_area.y,
                width: content_area.width,
                height: 1,
            });

            // 模型名称（只读，由 ↑/↓ 切换）
            let model_line = Paragraph::new(Line::from(vec![
                Span::styled("模型: ", Style::default().fg(Color::Cyan)),
                Span::styled(&action.model_name.value, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  (↑/↓ 切换)"),
            ]));
            f.render_widget(model_line, Rect {
                x: content_area.x,
                y: content_area.y + 1,
                width: content_area.width,
                height: 1,
            });

            // 输入框：Dim, TopK, 距离≤（同一行水平排列，每个宽 1/3）
            use crate::app::VectorSearchFocus;
            let input_rows = [
                ("维数(dim)", &action.dim, VectorSearchFocus::Dim),
                ("TopK", &action.top_k, VectorSearchFocus::TopK),
                ("距离≤", &action.max_distance, VectorSearchFocus::MaxDistance),
            ];

            let inputs_y = content_area.y + 2;
            let col_width = content_area.width / 3;
            let input_h: u16 = 3;

            for (i, (label, input, focus)) in input_rows.iter().enumerate() {
                let focused = action.search_focus == *focus;
                let style = if focused {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(*label)
                    .style(style);
                let p = Paragraph::new(input.value.clone())
                    .block(block)
                    .alignment(Alignment::Left);
                let input_area = Rect {
                    x: content_area.x + i as u16 * col_width,
                    y: inputs_y,
                    width: col_width,
                    height: input_h,
                };
                f.render_widget(p, input_area);
                if focused {
                    let cursor_x = input_area.x + 1 + input.value.chars().take(input.cursor).map(|c| c.len_utf8()).count() as u16;
                    let cursor_y = input_area.y + 1;
                    if cursor_x < input_area.x + input_area.width {
                        f.set_cursor(cursor_x, cursor_y);
                    }
                }
            }

            // 提示
            let hint = Paragraph::new(Line::from(vec![
                Span::styled("Enter ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("搜索  "),
                Span::styled("Tab ", Style::default().fg(Color::Cyan)),
                Span::raw("切换字段  "),
                Span::styled("↑/↓ ", Style::default().fg(Color::Cyan)),
                Span::raw("切换模型  "),
                Span::styled("Esc ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("取消"),
            ]))
            .alignment(Alignment::Center);
            let hint_y = inputs_y + input_h + 1;
            f.render_widget(hint, Rect {
                x: content_area.x,
                y: hint_y,
                width: content_area.width,
                height: 1,
            });
        }
        _ => {}
    }
}

/// 绘制向量搜索结果弹窗
///
/// 固定显示 5 行结果，超出时支持 ↑/↓ 选择并自动滚动，PgUp/PgDn 翻页。
fn draw_search_results_popup(f: &mut Frame, app: &mut App) {
    let area = f.size();
    let results = &app.image_tab.search_results;
    let total = results.len();
    if total == 0 {
        return;
    }

    // 固定显示 5 行结果
    const VISIBLE_RESULTS: usize = 5;
    let result_rows = VISIBLE_RESULTS;
    let width = 50.min(area.width.saturating_sub(4));
    // 弹窗高度 = 上下边框(2) + 表头(1) + 分隔线(1) + 结果行 + 底部提示(1)
    let height = result_rows as u16 + 5;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let title = format!("向量搜索[{}]  ({}/{})", app.image_tab.search_index_name, total, total);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(truncate_str(&title, dialog_area.width as usize - 2))
        .style(Style::default().bg(Color::Black).fg(Color::White));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    // 将可视区域分为左右：内容区 + 滚动条区
    let content_width = inner.width.saturating_sub(1);
    let content_area = Rect {
        x: inner.x,
        y: inner.y,
        width: content_width,
        height: inner.height,
    };
    let scrollbar_area = Rect {
        x: inner.x + content_width,
        y: inner.y,
        width: 1,
        height: inner.height,
    };

    // 表头
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" ID", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled("距离", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ]));
    f.render_widget(header, Rect {
        x: content_area.x,
        y: content_area.y,
        width: content_area.width,
        height: 1,
    });

    // 分隔线
    let sep = Paragraph::new(Line::from(Span::raw("─".repeat(content_area.width as usize))))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(sep, Rect {
        x: content_area.x,
        y: content_area.y + 1,
        width: content_area.width,
        height: 1,
    });

    // 结果行区域：从 y+2 到 y+height-2（留 1 行给底部提示）
    let result_area_y = content_area.y + 2;
    let result_area_h = content_area.height.saturating_sub(3); // 表头+分隔+提示
    let visible = result_area_h as usize;

    // ── 自动滚动：确保选中行在可见区域内 ──
    if let Some(sel) = app.image_tab.search_selected {
        if sel < app.image_tab.search_results_scroll {
            app.image_tab.search_results_scroll = sel;
        } else if sel >= app.image_tab.search_results_scroll + visible {
            app.image_tab.search_results_scroll = sel - visible + 1;
        }
    }
    // 限制滚动范围
    if visible >= total {
        app.image_tab.search_results_scroll = 0;
    } else if app.image_tab.search_results_scroll + visible > total {
        app.image_tab.search_results_scroll = total.saturating_sub(visible);
    }

    let start = app.image_tab.search_results_scroll;
    let end = (start + visible).min(total);

    for (i, idx) in (start..end).enumerate() {
        let result = &results[idx];
        let row_y = result_area_y + i as u16;
        if row_y >= content_area.y + content_area.height - 1 {
            break;
        }
        let is_selected = app.image_tab.search_selected == Some(idx);
        let row_style = if is_selected {
            Style::default().bg(Color::Green).fg(Color::Black)
        } else {
            Style::default()
        };
        let row = Paragraph::new(Line::from(vec![
            Span::raw(format!(" {}", result.id)),
            Span::raw("  "),
            Span::styled(
                format!("{:.4}", result.score),
                if is_selected {
                    Style::default().fg(Color::Black)
                } else {
                    Style::default().fg(Color::Yellow)
                },
            ),
        ]))
        .style(row_style);
        f.render_widget(row, Rect {
            x: content_area.x,
            y: row_y,
            width: content_area.width,
            height: 1,
        });
    }

    // 滚动条（仅当结果数超过可见行数时显示）
    if total > visible {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_symbol("█")
            .track_symbol(Some("░"))
            .style(Style::default().fg(Color::DarkGray));
        let mut state = ScrollbarState::new(total - visible + 1).position(app.image_tab.search_results_scroll);
        f.render_stateful_widget(scrollbar, scrollbar_area, &mut state);
    }

    // 底部提示
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("↑/↓ ", Style::default().fg(Color::Cyan)),
        Span::raw("选择  "),
        Span::styled("PgUp/PgDn ", Style::default().fg(Color::Cyan)),
        Span::raw("翻页  "),
        Span::styled("Enter ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("元数据  "),
        Span::styled("Esc ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("关闭"),
    ]))
    .alignment(Alignment::Center);
    let hint_y = content_area.y + content_area.height - 1;
    f.render_widget(hint, Rect {
        x: content_area.x,
        y: hint_y,
        width: content_area.width,
        height: 1,
    });
}

/// 将 Unix 时间戳字符串（秒）格式化为 Asia/Shanghai 时区的时间字符串
fn format_timestamp(ts: &str) -> String {
    let secs: u64 = match ts.parse() {
        Ok(s) => s,
        Err(_) => return ts.to_string(),
    };
    let naive = chrono::DateTime::from_timestamp(secs as i64, 0)
        .unwrap_or_default();
    // Asia/Shanghai = UTC+8, 不使用 DST
    let shanghai = naive
        .checked_add_signed(chrono::TimeDelta::hours(8))
        .unwrap_or(naive);
    shanghai.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 绘制图片操作弹窗
const IMAGE_ACTION_OPTIONS: &[&str] = &["查看元数据", "下载图片", "删除图片"];

fn draw_image_action_popup(f: &mut Frame, app: &mut App) {
    let area = f.size();
    let width = 30;
    let height = IMAGE_ACTION_OPTIONS.len() as u16 + 3; // 标题 + 选项 + 边框
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("图片操作")
        .style(Style::default().bg(Color::Black).fg(Color::White));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    for (i, opt) in IMAGE_ACTION_OPTIONS.iter().enumerate() {
        let selected = i == app.image_tab.action_popup_selected;
        let style = if selected {
            Style::default().bg(Color::Green).fg(Color::Black)
        } else {
            Style::default().fg(Color::White)
        };
        let prefix = if selected { "▶ " } else { "  " };
        let line = Paragraph::new(Line::from(Span::styled(
            format!("{}{}", prefix, opt),
            style,
        )));
        f.render_widget(line, Rect {
            x: inner.x,
            y: inner.y + i as u16,
            width: inner.width,
            height: 1,
        });
    }
}

/// 绘制删除确认弹窗
fn draw_delete_confirm_dialog(f: &mut Frame, key: &str) {
    let area = f.size();
    let width = 50.min(area.width.saturating_sub(4));
    let height = 5;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("确认删除")
        .style(Style::default().bg(Color::Black).fg(Color::Red));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    let key_display = truncate_str(key, inner.width as usize);
    let msg = Paragraph::new(Line::from(vec![
        Span::styled("确认删除图片 ", Style::default().fg(Color::White)),
        Span::styled(key_display, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" ?"),
    ]));
    f.render_widget(msg, Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    });

    let hint = Paragraph::new(Line::from(vec![
        Span::styled("Enter ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("确认删除  "),
        Span::styled("Esc ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("取消"),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hint, Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: 1,
    });
}

/// 绘制下载确认弹窗
fn draw_download_confirm_dialog(f: &mut Frame, app: &mut App) {
    let area = f.size();
    let width = 60.min(area.width.saturating_sub(4));
    let height = 12;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let dialog_area = Rect { x, y, width, height };

    f.render_widget(Clear, dialog_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("下载图片")
        .style(Style::default().bg(Color::Black).fg(Color::Cyan));
    f.render_widget(block, dialog_area);

    let inner = Rect {
        x: dialog_area.x + 1,
        y: dialog_area.y + 1,
        width: dialog_area.width.saturating_sub(2),
        height: dialog_area.height.saturating_sub(2),
    };

    const VISIBLE_LINES: usize = 4;

    // 提示文字
    let label = Paragraph::new("保存路径（支持多行编辑，方向键移动光标）:");
    f.render_widget(label, Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1,
    });

    // 计算光标所在行，并更新滚动偏移
    let line_width = inner.width as usize;
    let cursor_line = if line_width > 0 {
        app.image_tab.download_path.cursor / line_width
    } else {
        0
    };

    // 自动滚动：确保光标在可见区域内
    if cursor_line < app.image_tab.download_path_scroll {
        app.image_tab.download_path_scroll = cursor_line;
    }
    if cursor_line >= app.image_tab.download_path_scroll + VISIBLE_LINES {
        app.image_tab.download_path_scroll = cursor_line - VISIBLE_LINES + 1;
    }

    // 路径输入框（4 行，可滚动，自动换行）
    let input_style = Style::default().bg(Color::DarkGray).fg(Color::White);
    let input = Paragraph::new(app.image_tab.download_path.value.as_str())
        .style(input_style)
        .scroll((app.image_tab.download_path_scroll as u16, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(input, Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: VISIBLE_LINES as u16,
    });

    // 光标
    if app.image_tab.download_confirm.is_some() {
        let visible_line = cursor_line - app.image_tab.download_path_scroll;
        let col = app.image_tab.download_path.cursor - cursor_line * line_width;
        let cursor_y = inner.y + 1 + visible_line as u16;
        let cursor_x = inner.x + col as u16;
        f.set_cursor(cursor_x, cursor_y);
    }

    // 操作提示
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("Enter ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("确认下载  "),
        Span::styled("Esc ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("取消  "),
        Span::styled("←/→ ", Style::default().fg(Color::Cyan)),
        Span::raw("移动光标"),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(hint, Rect {
        x: inner.x,
        y: inner.y + 6,
        width: inner.width,
        height: 1,
    });

    // 滚动指示
    let total_lines = if line_width > 0 {
        (app.image_tab.download_path.value.len() + line_width - 1) / line_width
    } else {
        0
    };
    if total_lines > VISIBLE_LINES {
        let max_scroll = total_lines - VISIBLE_LINES;
        let pct = if max_scroll > 0 {
            app.image_tab.download_path_scroll as f64 / max_scroll as f64
        } else {
            0.0
        };
        let scrollbar_pos = (pct * (VISIBLE_LINES as f64 - 1.0)).round() as u16;
        let scrollbar_x = inner.x + inner.width;
        for i in 0..VISIBLE_LINES as u16 {
            let ch = if i == scrollbar_pos { "█" } else { "░" };
            let s = Paragraph::new(ch).style(Style::default().fg(Color::DarkGray));
            f.render_widget(s, Rect {
                x: scrollbar_x,
                y: inner.y + 1 + i,
                width: 1,
                height: 1,
            });
        }
    }
}
