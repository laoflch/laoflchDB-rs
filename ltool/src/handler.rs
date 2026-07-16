//! 输入事件处理
//!
//! 处理 crossterm 键盘事件：命令模式、Tab 切换、各 Tab 的快捷键和输入框编辑。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::app::{
    App, FaceFocus, InputState, Tab, VectorFocus,
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

    // 各 Tab 特定处理（弹窗激活时由各 Tab 内部处理 Tab/Enter/Esc 等，不触发全局 Tab 切换）
    let handled = match app.current_tab {
        Tab::Image => handle_image_tab(app, event).await,
        Tab::Face => handle_face_tab(app, event).await,
        Tab::Vector => handle_vector_tab(app, event).await,
        Tab::Sql => handle_sql_tab(app, event).await,
    };
    if handled {
        return true;
    }

    // 无弹窗激活时的全局快捷键：Alt+1~4 切换主 Tab
    // Tab 键留给各 Tab 内部做字段/焦点切换，不在这里做全局切换
    match event.code {
        KeyCode::Char('1') if event.modifiers.contains(KeyModifiers::ALT) => {
            app.clear_image_tab_popups();
            app.current_tab = Tab::Image;
            return true;
        }
        KeyCode::Char('2') if event.modifiers.contains(KeyModifiers::ALT) => {
            app.clear_image_tab_popups();
            app.current_tab = Tab::Face;
            return true;
        }
        KeyCode::Char('3') if event.modifiers.contains(KeyModifiers::ALT) => {
            app.clear_image_tab_popups();
            app.current_tab = Tab::Vector;
            return true;
        }
        KeyCode::Char('4') if event.modifiers.contains(KeyModifiers::ALT) => {
            app.clear_image_tab_popups();
            app.current_tab = Tab::Sql;
            return true;
        }
        _ => {}
    }

    false
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

/// 执行命令模式命令（:login / :quit / :help / :bucket / :key）
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
        "bucket" => {
            // :bucket <name> 设置图片/人脸 Tab 的 bucket
            if parts.len() == 2 {
                match app.current_tab {
                    Tab::Image => {
                        app.image_tab.bucket.set_value(parts[1]);
                        app.set_status(format!("图片 Tab bucket 已设为: {}", parts[1]));
                    }
                    Tab::Face => {
                        app.face_tab.bucket.set_value(parts[1]);
                        app.set_status(format!("人脸 Tab bucket 已设为: {}", parts[1]));
                    }
                    _ => {
                        app.set_error("当前 Tab 不支持 bucket 设置");
                    }
                }
            } else {
                app.set_error("用法: :bucket <名称>");
            }
        }
        "key" => {
            // :key <value> 设置图片 Tab 的 key（留空则自动生成）
            if parts.len() == 2 {
                app.image_tab.key.set_value(parts[1]);
                app.set_status(format!("图片 key 已设为: {}", parts[1]));
            } else if parts.len() == 1 {
                app.image_tab.key.set_value("");
                app.set_status("图片 key 已清空（将自动生成）");
            } else {
                app.set_error("用法: :key [值]（无值则清空，自动生成）");
            }
        }
        "help" | "?" => {
            app.set_status(
                "命令: :login <user> <pass> | :bucket <名称> | :key [值] | :quit | :help | Alt+1~4 切换 Tab",
            );
        }
        _ => {
            app.set_error(format!("未知命令: {}（试 :help）", parts[0]));
        }
    }
}

/// 图片操作弹窗的选项
const IMAGE_ACTION_OPTIONS: &[&str] = &["查看元数据", "下载图片", "删除图片"];

/// 处理图片 Tab 的事件
async fn handle_image_tab(app: &mut App, event: KeyEvent) -> bool {
    // 向量搜索结果显示弹窗优先
    if app.image_tab.show_search_results {
        let n = app.image_tab.search_results.len();
        match event.code {
            KeyCode::Esc => {
                app.image_tab.show_search_results = false;
                app.image_tab.search_results.clear();
                app.image_tab.search_results_scroll = 0;
                app.image_tab.search_selected = None;
                return true;
            }
            KeyCode::Up => {
                if n > 0 {
                    let cur = app.image_tab.search_selected.unwrap_or(0);
                    app.image_tab.search_selected = Some(if cur == 0 { n - 1 } else { cur - 1 });
                }
                return true;
            }
            KeyCode::Down => {
                if n > 0 {
                    let cur = app.image_tab.search_selected.unwrap_or(0);
                    app.image_tab.search_selected = Some(if cur >= n - 1 { 0 } else { cur + 1 });
                }
                return true;
            }
            KeyCode::PageUp => {
                if n > 0 {
                    let cur = app.image_tab.search_selected.unwrap_or(0);
                    app.image_tab.search_selected = Some(cur.saturating_sub(10));
                }
                return true;
            }
            KeyCode::PageDown => {
                if n > 0 {
                    let cur = app.image_tab.search_selected.unwrap_or(0);
                    app.image_tab.search_selected = Some((cur + 10).min(n - 1));
                }
                return true;
            }
            KeyCode::Enter => {
                // 查看选中图片的元数据
                if let Some(idx) = app.image_tab.search_selected {
                    if idx < n {
                        let key = app.image_tab.search_results[idx].id.to_string();
                        app.image_tab.key.set_value(&key);
                        let _ = crate::tab_image::get_metadata(app).await;
                    }
                }
                return true;
            }
            _ => {}
        }
    }

    // 本地文件操作弹窗优先
    if app.image_tab.local_file_action.is_some() {
        let tab = app.image_tab.local_file_action.as_ref().map(|a| a.tab).unwrap_or(0);
        if tab == 1 {
            // ── 向量搜索 Tab：支持字段编辑 ────────────────
            match event.code {
                KeyCode::Tab => {
                    // 在 Dim → TopK → MaxDistance → Tab0 → Tab1 间循环
                    use crate::app::VectorSearchFocus;
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    match action.search_focus {
                        VectorSearchFocus::Dim => action.search_focus = VectorSearchFocus::TopK,
                        VectorSearchFocus::TopK => action.search_focus = VectorSearchFocus::MaxDistance,
                        VectorSearchFocus::MaxDistance => {
                            // 切换到上传 Tab
                            action.tab = 0;
                            action.search_focus = VectorSearchFocus::Dim;
                        }
                    }
                    return true;
                }
                KeyCode::BackTab => {
                    use crate::app::VectorSearchFocus;
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    match action.search_focus {
                        VectorSearchFocus::MaxDistance => action.search_focus = VectorSearchFocus::TopK,
                        VectorSearchFocus::TopK => action.search_focus = VectorSearchFocus::Dim,
                        VectorSearchFocus::Dim => {
                            action.tab = 0;
                            action.search_focus = VectorSearchFocus::Dim;
                        }
                    }
                    return true;
                }
                KeyCode::Up => {
                    // 切换模型
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    if !action.models.is_empty() {
                        action.model_index = if action.model_index == 0 {
                            action.models.len() - 1
                        } else {
                            action.model_index - 1
                        };
                        action.model_name.set_value(&action.models[action.model_index]);
                    }
                    return true;
                }
                KeyCode::Down => {
                    // 切换模型
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    if !action.models.is_empty() {
                        action.model_index = (action.model_index + 1) % action.models.len();
                        action.model_name.set_value(&action.models[action.model_index]);
                    }
                    return true;
                }
                KeyCode::Enter => {
                    let file_path = app.image_tab.local_file_action.as_ref().map(|a| a.file_path.clone()).unwrap_or_default();
                    app.image_tab.file_path.set_value(&file_path);
                    let model_name = app.image_tab.local_file_action.as_ref().map(|a| a.model_name.value.clone()).unwrap_or_default();
                    let index_name = app.image_tab.local_file_action.as_ref().map(|a| a.index_name.value.clone()).unwrap_or_default();
                    let dim: i32 = app.image_tab.local_file_action.as_ref()
                        .and_then(|a| a.dim.value.parse().ok())
                        .unwrap_or(0);
                    let top_k: i32 = app.image_tab.local_file_action.as_ref()
                        .and_then(|a| a.top_k.value.parse().ok())
                        .unwrap_or(10);
                    let max_distance: f32 = app.image_tab.local_file_action.as_ref()
                        .and_then(|a| a.max_distance.value.parse().ok())
                        .unwrap_or(0.1);
                    app.image_tab.local_file_action = None;
                    let _ = crate::tab_image::search_similar_image(app, &model_name, &index_name, dim, top_k, max_distance).await;
                    return true;
                }
                KeyCode::Esc => {
                    app.image_tab.local_file_action = None;
                    app.set_status("已取消操作");
                    return true;
                }
                _ => {
                    // 编辑当前聚焦的字段
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    let input = match action.search_focus {
                        crate::app::VectorSearchFocus::Dim => &mut action.dim,
                        crate::app::VectorSearchFocus::TopK => &mut action.top_k,
                        crate::app::VectorSearchFocus::MaxDistance => &mut action.max_distance,
                    };
                    let changed = handle_input_event(input, event);
                    // 编辑时自动更新模型维度（dim 未设置时使用默认值）
                    return changed;
                }
            }
        } else {
            // ── 上传 Tab ────────────────────────────────
            match event.code {
                KeyCode::Tab | KeyCode::Right => {
                    let action = app.image_tab.local_file_action.as_mut().unwrap();
                    action.tab = 1;
                    action.search_focus = crate::app::VectorSearchFocus::Dim;
                    return true;
                }
                KeyCode::Left => {
                    // 已经是 Tab 0，不切换
                    return true;
                }
                KeyCode::Up | KeyCode::Down => {
                    return true;
                }
                KeyCode::Enter => {
                    let file_path = app.image_tab.local_file_action.as_ref().map(|a| a.file_path.clone()).unwrap_or_default();
                    app.image_tab.file_path.set_value(&file_path);
                    app.image_tab.local_file_action = None;
                    let _ = crate::tab_image::upload_image(app).await;
                    return true;
                }
                KeyCode::Esc => {
                    app.image_tab.local_file_action = None;
                    app.set_status("已取消操作");
                    return true;
                }
                _ => {}
            }
        }
    }

    // 删除确认弹窗
    if app.image_tab.delete_confirm.is_some() {
        match event.code {
            KeyCode::Enter => {
                let _ = crate::tab_image::delete_image(app).await;
                app.image_tab.delete_confirm = None;
                return true;
            }
            KeyCode::Esc => {
                app.image_tab.delete_confirm = None;
                app.set_status("已取消删除");
                return true;
            }
            _ => {}
        }
    }

    // 下载确认弹窗
    if app.image_tab.download_confirm.is_some() {
        match event.code {
            KeyCode::Enter => {
                let _ = crate::tab_image::download_image(app).await;
                app.image_tab.download_confirm = None;
                app.image_tab.download_path.clear();
                return true;
            }
            KeyCode::Esc => {
                app.image_tab.download_confirm = None;
                app.image_tab.download_path.clear();
                app.set_status("已取消下载");
                return true;
            }
            _ => {
                // 下载路径输入框编辑
                let changed = handle_input_event(&mut app.image_tab.download_path, event);
                return changed;
            }
        }
    }

    // 图片操作弹窗
    if app.image_tab.action_popup_open {
        match event.code {
            KeyCode::Up => {
                if app.image_tab.action_popup_selected > 0 {
                    app.image_tab.action_popup_selected -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = IMAGE_ACTION_OPTIONS.len().saturating_sub(1);
                if app.image_tab.action_popup_selected < max {
                    app.image_tab.action_popup_selected += 1;
                }
                return true;
            }
            KeyCode::Enter => {
                let idx = app.image_tab.action_popup_selected;
                let selected = app.image_tab.selected_index;
                app.image_tab.action_popup_open = false;
                match idx {
                    0 => {
                        // 查看元数据
                        if let Some(si) = selected {
                            if si < app.image_tab.images.len() {
                                let key = &app.image_tab.images[si].key;
                                app.image_tab.key.set_value(key);
                                let _ = crate::tab_image::get_metadata(app).await;
                            }
                        }
                    }
                    1 => {
                        // 下载图片
                        if let Some(si) = selected {
                            if si < app.image_tab.images.len() {
                                let meta = &app.image_tab.images[si];
                                let key = meta.key.clone();
                                app.image_tab.key.set_value(&key);
                                // 默认保存到 ~/Pictures/{key}.{ext}
                                let ext = match meta.content_type.as_str() {
                                    "image/jpeg" => "jpg",
                                    "image/png" => "png",
                                    "image/gif" => "gif",
                                    "image/webp" => "webp",
                                    "image/bmp" => "bmp",
                                    _ => "jpg",
                                };
                                let home = std::env::var("HOME").unwrap_or_default();
                                let default_path = format!("{home}/Pictures/{key}.{ext}");
                                app.image_tab.download_path.set_value(&default_path);
                                app.image_tab.download_confirm = Some(key);
                            }
                        }
                    }
                    2 => {
                        // 删除图片
                        if let Some(si) = selected {
                            if si < app.image_tab.images.len() {
                                let key = &app.image_tab.images[si].key;
                                app.image_tab.key.set_value(key);
                                app.image_tab.delete_confirm = Some(key.clone());
                            }
                        }
                    }
                    _ => {}
                }
                return true;
            }
            KeyCode::Esc => {
                app.image_tab.action_popup_open = false;
                return true;
            }
            _ => {}
        }
    }

    // 快捷键：F1=上传, F2=列出；Ctrl+M 元数据, Ctrl+D 删除
    // bucket/key 通过命令模式 :bucket / :key 设置，状态栏只读显示
    match event.code {
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

    // 弹窗激活时，Up/Down/Enter/Esc 优先交给弹窗
    if app.image_tab.path_popup.is_active() {
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
                        // 选中文件后弹出本地文件操作对话框（上传/向量索引）
                        let models = vec![
                            "jina-clip-v2".to_string(),
                            "siglip2".to_string(),
                            "bge-small-zh-v1.5".to_string(),
                        ];
                        app.image_tab.local_file_action = Some(crate::app::LocalFileAction {
                            file_path: full,
                            tab: 0,
                            model_name: crate::app::InputState::with_value("jina-clip-v2"),
                            index_name: crate::app::InputState::with_value("image"),
                            dim: crate::app::InputState::with_value("512"),
                            top_k: crate::app::InputState::with_value("10"),
                            max_distance: crate::app::InputState::with_value("0.1"),
                            models,
                            model_index: 0,
                            search_focus: crate::app::VectorSearchFocus::Dim,
                        });
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

    // 文件路径非空时按 Enter 重新打开操作对话框
    if event.code == KeyCode::Enter
        && !app.image_tab.file_path.value.is_empty()
        && app.image_tab.local_file_action.is_none()
        && !app.image_tab.path_popup.active
    {
        let models = vec![
            "jina-clip-v2".to_string(),
            "siglip2".to_string(),
            "bge-small-zh-v1.5".to_string(),
        ];
        let full = app.image_tab.file_path.value.clone();
        app.image_tab.local_file_action = Some(crate::app::LocalFileAction {
            file_path: full,
            tab: 0,
            model_name: crate::app::InputState::with_value("jina-clip-v2"),
            index_name: crate::app::InputState::with_value("image"),
            dim: crate::app::InputState::with_value("512"),
            top_k: crate::app::InputState::with_value("10"),
            max_distance: crate::app::InputState::with_value("0.1"),
            models,
            model_index: 0,
            search_focus: crate::app::VectorSearchFocus::Dim,
        });
        return true;
    }

    match event.code {
        KeyCode::Up => {
            let cur = app.image_tab.selected_index.unwrap_or(0);
            if cur > 0 {
                app.image_tab.selected_index = Some(cur - 1);
                auto_scroll_image(&mut app.image_tab);
            }
            return true;
        }
        KeyCode::Down => {
            let max = app.image_tab.images.len().saturating_sub(1);
            let cur = app.image_tab.selected_index.unwrap_or(0);
            if cur < max {
                app.image_tab.selected_index = Some(cur + 1);
                auto_scroll_image(&mut app.image_tab);
            }
            return true;
        }
        KeyCode::PageUp => {
            let cur = app.image_tab.selected_index.unwrap_or(0);
            let new = cur.saturating_sub(10);
            app.image_tab.selected_index = Some(new);
            auto_scroll_image(&mut app.image_tab);
            return true;
        }
        KeyCode::PageDown => {
            let max = app.image_tab.images.len().saturating_sub(1);
            let cur = app.image_tab.selected_index.unwrap_or(0);
            app.image_tab.selected_index = Some((cur + 10).min(max));
            auto_scroll_image(&mut app.image_tab);
            return true;
        }
        // 选中行按 Enter 弹出操作窗口
        KeyCode::Enter => {
            if app.image_tab.selected_index.is_some() && !app.image_tab.images.is_empty() {
                app.image_tab.action_popup_open = true;
                app.image_tab.action_popup_selected = 0;
            }
            return true;
        }
        // Esc 取消选中
        KeyCode::Esc => {
            app.image_tab.selected_index = None;
            app.image_tab.action_popup_open = false;
            return true;
        }
        _ => {}
    }

    // 焦点恒为 FilePath，直接处理输入
    // 无弹窗时 Tab/BackTab 用于切换主 Tab（返回 false 让外层未匹配，外层也不处理，所以这里显式切换）
    if app.image_tab.local_file_action.is_none()
        && !app.image_tab.action_popup_open
        && !app.image_tab.delete_confirm.is_some()
        && !app.image_tab.download_confirm.is_some()
        && !app.image_tab.show_search_results
        && !app.image_tab.path_popup.is_active()
    {
        match event.code {
            KeyCode::Tab => {
                app.next_tab();
                return true;
            }
            KeyCode::BackTab => {
                app.prev_tab();
                return true;
            }
            _ => {}
        }
    }

    let changed = handle_input_event(&mut app.image_tab.file_path, event);
    if changed {
        // 用户开始输入路径时，清除列表选中状态
        app.image_tab.selected_index = None;
        app.image_tab.action_popup_open = false;
        let cs = crate::path_complete::list_candidates(&app.image_tab.file_path.value);
        app.image_tab.path_popup.refresh(cs);
    }
    changed
}

/// 确保 selected_index 在可见范围内
fn auto_scroll_image(tab: &mut crate::app::ImageTabState) {
    let Some(idx) = tab.selected_index else { return };
    let visible: usize = 50;
    if idx < tab.list_scroll {
        tab.list_scroll = idx;
    } else if idx >= tab.list_scroll + visible {
        tab.list_scroll = idx - visible + 1;
    }
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
        KeyCode::Char(c) if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT => input.insert_char(c),
        _ => return false,
    }
    true
}

/// 处理鼠标事件
pub async fn handle_mouse_event(app: &mut App, event: MouseEvent) {
    match event.kind {
        MouseEventKind::Down(_) => {
            if app.current_tab == Tab::Image {
                // 布局：Tab栏(3行) + 路径输入框(3行) + 表格边框(1行) + 表头(1行) → 数据行起始 y
                let data_start_y = 3 + 3 + 2; // tab_bar(3) + path_area(3) + border(1) + header(1) = 8
                if event.row >= data_start_y {
                    // 点击在表格区域 → 选中该行
                    let row = (event.row - data_start_y) as usize + app.image_tab.list_scroll;
                    if row < app.image_tab.images.len() {
                        app.image_tab.selected_index = Some(row);
                        let visible: usize = 50;
                        if let Some(idx) = app.image_tab.selected_index {
                            if idx < app.image_tab.list_scroll {
                                app.image_tab.list_scroll = idx;
                            } else if idx >= app.image_tab.list_scroll + visible {
                                app.image_tab.list_scroll = idx - visible + 1;
                            }
                        }
                    }
                } else if event.row >= 3 {
                    // 点击在路径输入框区域（y=3..8）→ 清除选中
                    app.image_tab.selected_index = None;
                    app.image_tab.action_popup_open = false;
                }
            }
        }
        _ => {}
    }
}
