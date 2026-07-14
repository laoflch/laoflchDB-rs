//! 输入事件处理
//!
//! 处理 crossterm 键盘事件：命令模式、Tab 切换、各 Tab 的快捷键和输入框编辑。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{
    App, FaceFocus, ImageFocus, InputState, Tab, VectorFocus,
};

/// 处理一个键盘事件
///
/// 返回 true 表示需要重绘，false 表示无变化（实际上目前每次都重绘）。
pub async fn handle_event(app: &mut App, event: KeyEvent) -> bool {
    // 命令模式优先处理
    if app.command_mode.active {
        return handle_command_mode(app, event);
    }

    // 全局退出快捷键
    if event.code == KeyCode::Char('q')
        && (event.modifiers.contains(KeyModifiers::CONTROL))
    {
        app.should_quit = true;
        return true;
    }
    if event.code == KeyCode::Char('c')
        && event.modifiers.contains(KeyModifiers::CONTROL)
    {
        app.should_quit = true;
        return true;
    }

    // 进入命令模式
    if event.code == KeyCode::Char(':') && event.modifiers.is_empty() {
        app.enter_command();
        return true;
    }

    // Tab 切换（路径补全已改为下拉菜单式，不再占用 Tab 键）
    match event.code {
        KeyCode::Tab => {
            app.next_tab();
            return true;
        }
        KeyCode::BackTab => {
            app.prev_tab();
            return true;
        }
        KeyCode::Char('1') if event.modifiers.is_empty() => {
            app.current_tab = Tab::Image;
            return true;
        }
        KeyCode::Char('2') if event.modifiers.is_empty() => {
            app.current_tab = Tab::Face;
            return true;
        }
        KeyCode::Char('3') if event.modifiers.is_empty() => {
            app.current_tab = Tab::Vector;
            return true;
        }
        KeyCode::Char('4') if event.modifiers.is_empty() => {
            app.current_tab = Tab::Sql;
            return true;
        }
        _ => {}
    }

    // 各 Tab 特定处理
    match app.current_tab {
        Tab::Image => handle_image_tab(app, event).await,
        Tab::Face => handle_face_tab(app, event).await,
        Tab::Vector => handle_vector_tab(app, event).await,
        Tab::Sql => handle_sql_tab(app, event).await,
    }
}

/// 处理命令模式输入
fn handle_command_mode(app: &mut App, event: KeyEvent) -> bool {
    let input = &mut app.command_mode.input;
    match event.code {
        KeyCode::Esc => {
            app.exit_command();
        }
        KeyCode::Enter => {
            let cmd = input.value.trim().to_string();
            app.exit_command();
            execute_command(app, &cmd);
        }
        KeyCode::Backspace => {
            input.backspace();
        }
        KeyCode::Delete => {
            input.delete();
        }
        KeyCode::Left => {
            input.left();
        }
        KeyCode::Right => {
            input.right();
        }
        KeyCode::Home => {
            input.home();
        }
        KeyCode::End => {
            input.end();
        }
        KeyCode::Char(c) => {
            input.insert_char(c);
        }
        _ => {}
    }
    true
}

/// 执行命令模式命令（:login / :quit / :help）
fn execute_command(app: &mut App, cmd: &str) {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return;
    }
    match parts[0] {
        "login" => {
            if parts.len() == 3 {
                app.username = parts[1].to_string();
                app.password = parts[2].to_string();
                app.set_status(format!("尝试登录用户 {}...", parts[1]));
            } else {
                app.set_error("用法: :login 用户名 密码");
            }
        }
        "quit" | "q" | "exit" => {
            app.should_quit = true;
        }
        "help" | "?" => {
            app.set_status(
                "命令: :login <user> <pass> | :quit | :help | 1/2/3/4 切换 Tab | F1-F5 操作 | Ctrl+Q 退出",
            );
        }
        _ => {
            app.set_error(format!("未知命令: {}（试 :help）", parts[0]));
        }
    }
}

/// 处理图片 Tab 的事件
async fn handle_image_tab(app: &mut App, event: KeyEvent) -> bool {
    // 快捷键：仅当焦点在"操作区"（非输入框）时触发。
    // 但本 Tab 所有焦点都是输入框，所以用 Ctrl 修饰键触发快捷键，避免与字符输入冲突。
    // 常用操作：Ctrl+U 上传, Ctrl+L 列出, Ctrl+M 元数据, Ctrl+D 删除
    // 另外支持 F1-F4 作为无修饰快捷键（不会与输入冲突）
    match event.code {
        KeyCode::Char('u') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = crate::tab_image::upload_image(app).await;
            return true;
        }
        KeyCode::Char('l') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = crate::tab_image::list_images(app).await;
            return true;
        }
        KeyCode::Char('m') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = crate::tab_image::get_metadata(app).await;
            return true;
        }
        KeyCode::Char('d') if event.modifiers.contains(KeyModifiers::CONTROL) => {
            let _ = crate::tab_image::delete_image(app).await;
            return true;
        }
        KeyCode::F(1) => {
            let _ = crate::tab_image::upload_image(app).await;
            return true;
        }
        KeyCode::F(2) => {
            let _ = crate::tab_image::list_images(app).await;
            return true;
        }
        _ => {}
    }

    // 当焦点在路径输入框且弹窗激活时，Up/Down/Enter/Esc 优先交给弹窗
    if app.image_tab.focus == ImageFocus::FilePath && app.image_tab.path_popup.is_active() {
        match event.code {
            KeyCode::Up => {
                app.image_tab.path_popup.prev();
                return true;
            }
            KeyCode::Down => {
                app.image_tab.path_popup.next();
                return true;
            }
            KeyCode::Enter => {
                if let Some(c) = app.image_tab.path_popup.current() {
                    let full = c.full_path.clone();
                    let is_dir = c.is_dir;
                    app.image_tab.file_path.set_value(&full);
                    app.image_tab.path_popup.close();
                    if is_dir {
                        // 进入目录后自动刷新候选
                        let cs = crate::path_complete::list_candidates(&full);
                        app.image_tab.path_popup.open(cs);
                    } else {
                        app.set_status(format!("已选择: {}", full));
                    }
                } else {
                    app.image_tab.path_popup.close();
                }
                return true;
            }
            KeyCode::Esc => {
                app.image_tab.path_popup.close();
                app.set_status("已取消路径补全");
                return true;
            }
            _ => {}
        }
    }

    match event.code {
        KeyCode::Tab => {
            // 切换焦点，并关闭弹窗
            app.image_tab.path_popup.close();
            app.image_tab.focus = match app.image_tab.focus {
                ImageFocus::Bucket => ImageFocus::FilePath,
                ImageFocus::FilePath => ImageFocus::Key,
                ImageFocus::Key => ImageFocus::Bucket,
            };
            // 若新焦点是 FilePath，立即刷新候选
            if app.image_tab.focus == ImageFocus::FilePath {
                let cs = crate::path_complete::list_candidates(&app.image_tab.file_path.value);
                app.image_tab.path_popup.open(cs);
            }
            return true;
        }
        KeyCode::Up if app.image_tab.focus != ImageFocus::FilePath || !app.image_tab.path_popup.is_active() => {
            app.image_tab.path_popup.close();
            app.image_tab.focus = match app.image_tab.focus {
                ImageFocus::Key => ImageFocus::FilePath,
                ImageFocus::FilePath => ImageFocus::Bucket,
                ImageFocus::Bucket => ImageFocus::Key,
            };
            return true;
        }
        KeyCode::Down if app.image_tab.focus != ImageFocus::FilePath || !app.image_tab.path_popup.is_active() => {
            app.image_tab.path_popup.close();
            app.image_tab.focus = match app.image_tab.focus {
                ImageFocus::Bucket => ImageFocus::FilePath,
                ImageFocus::FilePath => ImageFocus::Key,
                ImageFocus::Key => ImageFocus::Bucket,
            };
            return true;
        }
        KeyCode::PageUp => {
            if app.image_tab.list_scroll > 0 {
                app.image_tab.list_scroll = app.image_tab.list_scroll.saturating_sub(10);
            }
            return true;
        }
        KeyCode::PageDown => {
            let max = app.image_tab.images.len().saturating_sub(10);
            app.image_tab.list_scroll = (app.image_tab.list_scroll + 10).min(max);
            return true;
        }
        _ => {}
    }

    let input = match app.image_tab.focus {
        ImageFocus::Bucket => &mut app.image_tab.bucket,
        ImageFocus::FilePath => &mut app.image_tab.file_path,
        ImageFocus::Key => &mut app.image_tab.key,
    };
    let changed = handle_input_event(input, event);
    // 若路径输入框内容变化，刷新候选下拉
    if changed && app.image_tab.focus == ImageFocus::FilePath {
        let cs = crate::path_complete::list_candidates(&app.image_tab.file_path.value);
        app.image_tab.path_popup.refresh(cs);
    }
    changed
}

/// 处理人脸 Tab 的事件
async fn handle_face_tab(app: &mut App, event: KeyEvent) -> bool {
    // 快捷键用 F1-F5，避免与输入框字符冲突
    // F1=提取特征, F2=比较特征, F3=清空结果(预留), F4=切换save_aligned, F5=切换index_embedding
    match event.code {
        KeyCode::F(1) => {
            let _ = crate::tab_face::extract_features(app).await;
            return true;
        }
        KeyCode::F(2) => {
            let _ = crate::tab_face::compare_features(app).await;
            return true;
        }
        KeyCode::F(4) => {
            app.face_tab.save_aligned_images = !app.face_tab.save_aligned_images;
            app.set_status(format!(
                "save_aligned 已{}",
                if app.face_tab.save_aligned_images { "开启" } else { "关闭" }
            ));
            return true;
        }
        KeyCode::F(5) => {
            app.face_tab.index_embedding = !app.face_tab.index_embedding;
            app.set_status(format!(
                "index_embedding 已{}",
                if app.face_tab.index_embedding { "开启" } else { "关闭" }
            ));
            return true;
        }
        _ => {}
    }

    // 当焦点在路径输入框且弹窗激活时，Up/Down/Enter/Esc 优先交给弹窗
    if app.face_tab.focus == FaceFocus::FilePath && app.face_tab.path_popup.is_active() {
        match event.code {
            KeyCode::Up => {
                app.face_tab.path_popup.prev();
                return true;
            }
            KeyCode::Down => {
                app.face_tab.path_popup.next();
                return true;
            }
            KeyCode::Enter => {
                if let Some(c) = app.face_tab.path_popup.current() {
                    let full = c.full_path.clone();
                    let is_dir = c.is_dir;
                    app.face_tab.file_path.set_value(&full);
                    app.face_tab.path_popup.close();
                    if is_dir {
                        let cs = crate::path_complete::list_candidates(&full);
                        app.face_tab.path_popup.open(cs);
                    } else {
                        app.set_status(format!("已选择: {}", full));
                    }
                } else {
                    app.face_tab.path_popup.close();
                }
                return true;
            }
            KeyCode::Esc => {
                app.face_tab.path_popup.close();
                app.set_status("已取消路径补全");
                return true;
            }
            _ => {}
        }
    }

    match event.code {
        KeyCode::Tab => {
            app.face_tab.path_popup.close();
            app.face_tab.focus = match app.face_tab.focus {
                FaceFocus::FilePath => FaceFocus::DetThreshold,
                FaceFocus::DetThreshold => FaceFocus::MaxFaces,
                FaceFocus::MaxFaces => FaceFocus::Bucket,
                FaceFocus::Bucket => FaceFocus::FilePath,
            };
            if app.face_tab.focus == FaceFocus::FilePath {
                let cs = crate::path_complete::list_candidates(&app.face_tab.file_path.value);
                app.face_tab.path_popup.open(cs);
            }
            return true;
        }
        KeyCode::Up if app.face_tab.focus != FaceFocus::FilePath || !app.face_tab.path_popup.is_active() => {
            app.face_tab.path_popup.close();
            app.face_tab.focus = match app.face_tab.focus {
                FaceFocus::FilePath => FaceFocus::Bucket,
                FaceFocus::DetThreshold => FaceFocus::FilePath,
                FaceFocus::MaxFaces => FaceFocus::DetThreshold,
                FaceFocus::Bucket => FaceFocus::MaxFaces,
            };
            return true;
        }
        KeyCode::Down if app.face_tab.focus != FaceFocus::FilePath || !app.face_tab.path_popup.is_active() => {
            app.face_tab.path_popup.close();
            app.face_tab.focus = match app.face_tab.focus {
                FaceFocus::FilePath => FaceFocus::DetThreshold,
                FaceFocus::DetThreshold => FaceFocus::MaxFaces,
                FaceFocus::MaxFaces => FaceFocus::Bucket,
                FaceFocus::Bucket => FaceFocus::FilePath,
            };
            return true;
        }
        KeyCode::PageUp => {
            if app.face_tab.list_scroll > 0 {
                app.face_tab.list_scroll = app.face_tab.list_scroll.saturating_sub(10);
            }
            return true;
        }
        KeyCode::PageDown => {
            let max = app.face_tab.faces.len().saturating_sub(10);
            app.face_tab.list_scroll = (app.face_tab.list_scroll + 10).min(max);
            return true;
        }
        _ => {}
    }

    let input = match app.face_tab.focus {
        FaceFocus::FilePath => &mut app.face_tab.file_path,
        FaceFocus::DetThreshold => &mut app.face_tab.det_threshold,
        FaceFocus::MaxFaces => &mut app.face_tab.max_faces,
        FaceFocus::Bucket => &mut app.face_tab.bucket,
    };
    let changed = handle_input_event(input, event);
    if changed && app.face_tab.focus == FaceFocus::FilePath {
        let cs = crate::path_complete::list_candidates(&app.face_tab.file_path.value);
        app.face_tab.path_popup.refresh(cs);
    }
    changed
}

/// 处理向量 Tab 的事件
async fn handle_vector_tab(app: &mut App, event: KeyEvent) -> bool {
    // 快捷键用 F1/F2/F3，避免与输入框字符冲突
    // F1=索引信息, F2=搜索, F3=删除
    match event.code {
        KeyCode::F(1) => {
            let _ = crate::tab_vector::get_index_info(app).await;
            return true;
        }
        KeyCode::F(2) => {
            let _ = crate::tab_vector::search(app).await;
            return true;
        }
        KeyCode::F(3) => {
            let _ = crate::tab_vector::delete_embedding(app).await;
            return true;
        }
        KeyCode::Tab => {
            app.vector_tab.focus = match app.vector_tab.focus {
                VectorFocus::IndexName => VectorFocus::QueryVec,
                VectorFocus::QueryVec => VectorFocus::TopK,
                VectorFocus::TopK => VectorFocus::DeleteId,
                VectorFocus::DeleteId => VectorFocus::IndexName,
            };
            return true;
        }
        KeyCode::Up => {
            app.vector_tab.focus = match app.vector_tab.focus {
                VectorFocus::IndexName => VectorFocus::DeleteId,
                VectorFocus::QueryVec => VectorFocus::IndexName,
                VectorFocus::TopK => VectorFocus::QueryVec,
                VectorFocus::DeleteId => VectorFocus::TopK,
            };
            return true;
        }
        KeyCode::Down => {
            app.vector_tab.focus = match app.vector_tab.focus {
                VectorFocus::IndexName => VectorFocus::QueryVec,
                VectorFocus::QueryVec => VectorFocus::TopK,
                VectorFocus::TopK => VectorFocus::DeleteId,
                VectorFocus::DeleteId => VectorFocus::IndexName,
            };
            return true;
        }
        KeyCode::PageUp => {
            if app.vector_tab.list_scroll > 0 {
                app.vector_tab.list_scroll = app.vector_tab.list_scroll.saturating_sub(10);
            }
            return true;
        }
        KeyCode::PageDown => {
            let max = app.vector_tab.search_results.len().saturating_sub(10);
            app.vector_tab.list_scroll = (app.vector_tab.list_scroll + 10).min(max);
            return true;
        }
        _ => {}
    }

    let input = match app.vector_tab.focus {
        VectorFocus::IndexName => &mut app.vector_tab.index_name,
        VectorFocus::QueryVec => &mut app.vector_tab.query_vec,
        VectorFocus::TopK => &mut app.vector_tab.top_k,
        VectorFocus::DeleteId => &mut app.vector_tab.delete_id,
    };
    handle_input_event(input, event)
}

/// 处理 SQL Tab 的事件
async fn handle_sql_tab(app: &mut App, event: KeyEvent) -> bool {
    // Ctrl+L 清空
    if event.code == KeyCode::Char('l') && event.modifiers.contains(KeyModifiers::CONTROL) {
        crate::tab_sql::clear_sql(app);
        return true;
    }
    // F5 或 Enter 执行
    if event.code == KeyCode::F(5) {
        let _ = crate::tab_sql::execute_sql(app).await;
        return true;
    }
    if event.code == KeyCode::Enter && app.sql_tab.focus_sql {
        let _ = crate::tab_sql::execute_sql(app).await;
        return true;
    }

    // 在 SQL 输入框与 Schema 输入框之间切换
    if event.code == KeyCode::Up && app.sql_tab.focus_sql {
        app.sql_tab.focus_sql = false;
        return true;
    }
    if event.code == KeyCode::Down && !app.sql_tab.focus_sql {
        app.sql_tab.focus_sql = true;
        return true;
    }

    // PageUp/PageDown 滚动结果
    if event.code == KeyCode::PageUp {
        if app.sql_tab.list_scroll > 0 {
            app.sql_tab.list_scroll = app.sql_tab.list_scroll.saturating_sub(10);
        }
        return true;
    }
    if event.code == KeyCode::PageDown {
        let max = app.sql_tab.rows.len().saturating_sub(10);
        app.sql_tab.list_scroll = (app.sql_tab.list_scroll + 10).min(max);
        return true;
    }

    let input = if app.sql_tab.focus_sql {
        &mut app.sql_tab.sql
    } else {
        &mut app.sql_tab.schema
    };
    // SQL 模式下 Enter 换行（而不是执行）：用 Shift+Enter 触发换行的体验较复杂，这里用 Ctrl+Enter 不可靠；
    // 改为：当焦点在 SQL 框时，Enter 执行 SQL，Ctrl+J 插入换行。
    if event.code == KeyCode::Char('j') && event.modifiers.contains(KeyModifiers::CONTROL) {
        input.insert_char('\n');
        return true;
    }
    handle_input_event(input, event)
}

/// 通用的输入框事件处理（不处理 Enter / Tab / 快捷键，由调用方处理）
fn handle_input_event(input: &mut InputState, event: KeyEvent) -> bool {
    match event.code {
        KeyCode::Backspace => input.backspace(),
        KeyCode::Delete => input.delete(),
        KeyCode::Left => input.left(),
        KeyCode::Right => input.right(),
        KeyCode::Home => input.home(),
        KeyCode::End => input.end(),
        KeyCode::Char(c) if event.modifiers.is_empty() => input.insert_char(c),
        _ => return false,
    }
    true
}
