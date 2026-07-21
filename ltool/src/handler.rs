//! 输入事件处理
//!
//! 处理 crossterm 键盘事件：命令模式、Tab 切换、各 Tab 的快捷键和输入框编辑。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::app::{
    App, FaceFocus, InputState, Tab,
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
        Tab::Index => handle_index_tab(app, event).await,
    };
    if handled {
        return true;
    }

    // 无弹窗激活时的全局快捷键：Alt+1~5 切换主 Tab
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
        KeyCode::Char('5') if event.modifiers.contains(KeyModifiers::ALT) => {
            app.clear_image_tab_popups();
            app.current_tab = Tab::Index;
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
                        // 获取 image 索引的 dim 作为默认值
                        let dim_default = crate::tab_image::get_image_index_dim(app).await;
                        app.image_tab.local_file_action = Some(crate::app::LocalFileAction {
                            file_path: full,
                            tab: 0,
                            model_name: crate::app::InputState::with_value("jina-clip-v2"),
                            index_name: crate::app::InputState::with_value("image"),
                            dim: crate::app::InputState::with_value(&dim_default),
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

    // 文件路径非空时按 Enter 重新打开操作对话框（无选中行时）
    if event.code == KeyCode::Enter
        && !app.image_tab.file_path.value.is_empty()
        && app.image_tab.local_file_action.is_none()
        && !app.image_tab.path_popup.active
        && app.image_tab.selected_index.is_none()
    {
        let models = vec![
            "jina-clip-v2".to_string(),
            "siglip2".to_string(),
            "bge-small-zh-v1.5".to_string(),
        ];
        let full = app.image_tab.file_path.value.clone();
        // 获取 image 索引的 dim 作为默认值
        let dim_default = crate::tab_image::get_image_index_dim(app).await;
        app.image_tab.local_file_action = Some(crate::app::LocalFileAction {
            file_path: full,
            tab: 0,
            model_name: crate::app::InputState::with_value("jina-clip-v2"),
            index_name: crate::app::InputState::with_value("image"),
            dim: crate::app::InputState::with_value(&dim_default),
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

fn auto_scroll_face_saved(tab: &mut crate::app::FaceTabState) {
    let Some(idx) = tab.saved_selected else { return };
    let visible: usize = 50;
    if idx < tab.saved_scroll {
        tab.saved_scroll = idx;
    } else if idx >= tab.saved_scroll + visible {
        tab.saved_scroll = idx - visible + 1;
    }
}

/// 处理人脸 Tab 的事件
async fn handle_face_tab(app: &mut App, event: KeyEvent) -> bool {
    // ── 检测结果操作弹窗 ──
    if app.face_tab.detection_action_open {
        const DETECTION_ACTION_OPTIONS: &[&str] = &["保存人脸并索引"];
        match event.code {
            KeyCode::Up => {
                if app.face_tab.detection_action_selected > 0 {
                    app.face_tab.detection_action_selected -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = DETECTION_ACTION_OPTIONS.len().saturating_sub(1);
                if app.face_tab.detection_action_selected < max {
                    app.face_tab.detection_action_selected += 1;
                }
                return true;
            }
            KeyCode::Enter => {
                let idx = app.face_tab.detection_action_selected;
                app.face_tab.detection_action_open = false;
                if idx == 0 {
                    // 保存人脸并索引
                    let _ = crate::tab_face::save_and_index_face(app).await;
                }
                return true;
            }
            KeyCode::Esc => {
                app.face_tab.detection_action_open = false;
                return true;
            }
            _ => {}
        }
    }

    // ── 已保存人脸操作弹窗 ──
    if app.face_tab.saved_action_open {
        const FACE_ACTION_OPTIONS: &[&str] = &["导出人脸", "删除人脸"];
        match event.code {
            KeyCode::Up => {
                if app.face_tab.saved_action_selected > 0 {
                    app.face_tab.saved_action_selected -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = FACE_ACTION_OPTIONS.len().saturating_sub(1);
                if app.face_tab.saved_action_selected < max {
                    app.face_tab.saved_action_selected += 1;
                }
                return true;
            }
            KeyCode::Enter => {
                let idx = app.face_tab.saved_action_selected;
                app.face_tab.saved_action_open = false;
                if idx == 0 {
                    // 导出人脸
                    if let Some(si) = app.face_tab.saved_selected {
                        if si < app.face_tab.saved_faces.len() {
                            let key = app.face_tab.saved_faces[si].key.clone();
                            let _ = crate::tab_face::export_saved_face(app, &key).await;
                        }
                    }
                } else if idx == 1 {
                    // 删除人脸
                    if let Some(si) = app.face_tab.saved_selected {
                        if si < app.face_tab.saved_faces.len() {
                            let key = app.face_tab.saved_faces[si].key.clone();
                            app.face_tab.saved_delete_key = Some(key);
                        }
                    }
                }
                return true;
            }
            KeyCode::Esc => {
                app.face_tab.saved_action_open = false;
                return true;
            }
            _ => {}
        }
    }

    // ── 已保存人脸删除确认弹窗 ──
    if app.face_tab.saved_delete_key.is_some() {
        match event.code {
            KeyCode::Enter => {
                let _ = crate::tab_face::delete_saved_face(app).await;
                return true;
            }
            KeyCode::Esc => {
                app.face_tab.saved_delete_key = None;
                app.set_status("已取消删除");
                return true;
            }
            _ => {}
        }
    }

    // 快捷键
    match event.code {
        KeyCode::F(1) => {
            // F1: 检测人脸（仅检测，不保存/索引）
            let _ = crate::tab_face::extract_features(app).await;
            return true;
        }
        KeyCode::F(3) => {
            // F3: 列出已保存人脸
            let _ = crate::tab_face::list_saved_faces(app).await;
            return true;
        }
        KeyCode::F(6) => {
            // F6: 导出所有检测到的人脸
            let export_path = app.face_tab.export_path.value.clone();
            let _ = crate::tab_face::export_faces(app, &export_path).await;
            return true;
        }
        _ => {}
    }

    // ── 检测结果列表导航 ──
    // 仅在未显示已保存人脸列表时启用，避免与 F3 弹窗导航冲突
    if !app.face_tab.show_saved && !app.face_tab.faces.is_empty() {
        match event.code {
            KeyCode::Up => {
                let cur = app.face_tab.selected_face_num.unwrap_or(0);
                if cur > 0 {
                    app.face_tab.selected_face_num = Some(cur - 1);
                    if cur - 1 < app.face_tab.list_scroll {
                        app.face_tab.list_scroll = cur - 1;
                    }
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.face_tab.faces.len().saturating_sub(1);
                let cur = app.face_tab.selected_face_num.unwrap_or(0);
                if cur < max {
                    app.face_tab.selected_face_num = Some(cur + 1);
                    if cur + 1 >= app.face_tab.list_scroll + 10 {
                        app.face_tab.list_scroll = cur + 1 - 9;
                    }
                }
                return true;
            }
            KeyCode::Enter => {
                if app.face_tab.selected_face_num.is_some() && !app.face_tab.faces.is_empty() {
                    app.face_tab.detection_action_open = true;
                    app.face_tab.detection_action_selected = 0;
                }
                return true;
            }
            _ => {}
        }
    }

    // ── 已保存人脸列表导航 ──
    if app.face_tab.show_saved {
        match event.code {
            KeyCode::Up => {
                let cur = app.face_tab.saved_selected.unwrap_or(0);
                if cur > 0 {
                    app.face_tab.saved_selected = Some(cur - 1);
                    auto_scroll_face_saved(&mut app.face_tab);
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.face_tab.saved_faces.len().saturating_sub(1);
                let cur = app.face_tab.saved_selected.unwrap_or(0);
                if cur < max {
                    app.face_tab.saved_selected = Some(cur + 1);
                    auto_scroll_face_saved(&mut app.face_tab);
                }
                return true;
            }
            KeyCode::PageUp => {
                let cur = app.face_tab.saved_selected.unwrap_or(0);
                app.face_tab.saved_selected = Some(cur.saturating_sub(10));
                auto_scroll_face_saved(&mut app.face_tab);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.face_tab.saved_faces.len().saturating_sub(1);
                let cur = app.face_tab.saved_selected.unwrap_or(0);
                app.face_tab.saved_selected = Some((cur + 10).min(max));
                auto_scroll_face_saved(&mut app.face_tab);
                return true;
            }
            KeyCode::Enter => {
                if app.face_tab.saved_selected.is_some() && !app.face_tab.saved_faces.is_empty() {
                    app.face_tab.saved_action_open = true;
                    app.face_tab.saved_action_selected = 0;
                }
                return true;
            }
            KeyCode::Esc => {
                if !app.face_tab.saved_action_open && app.face_tab.saved_delete_key.is_none() {
                    app.face_tab.show_saved = false;
                    app.set_status("已关闭已保存人脸列表");
                }
                return true;
            }
            _ => {}
        }
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
                FaceFocus::Bucket => FaceFocus::ExportPath,
                FaceFocus::ExportPath => FaceFocus::FilePath,
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
                FaceFocus::FilePath => FaceFocus::ExportPath,
                FaceFocus::DetThreshold => FaceFocus::FilePath,
                FaceFocus::MaxFaces => FaceFocus::DetThreshold,
                FaceFocus::Bucket => FaceFocus::MaxFaces,
                FaceFocus::ExportPath => FaceFocus::Bucket,
            };
            return true;
        }
        KeyCode::Down if app.face_tab.focus != FaceFocus::FilePath || !app.face_tab.path_popup.is_active() => {
            app.face_tab.path_popup.close();
            app.face_tab.focus = match app.face_tab.focus {
                FaceFocus::FilePath => FaceFocus::DetThreshold,
                FaceFocus::DetThreshold => FaceFocus::MaxFaces,
                FaceFocus::MaxFaces => FaceFocus::Bucket,
                FaceFocus::Bucket => FaceFocus::ExportPath,
                FaceFocus::ExportPath => FaceFocus::FilePath,
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
        FaceFocus::ExportPath => &mut app.face_tab.export_path,
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
    // 首次进入时自动刷新索引列表
    if !app.vector_tab.auto_refreshed {
        app.vector_tab.auto_refreshed = true;
        let _ = crate::tab_vector::get_all_indices(app).await;
    }

    // ── 向量详情弹窗优先 ──
    if app.vector_tab.show_vector_detail {
        match event.code {
            KeyCode::Up => {
                if app.vector_tab.vector_detail_scroll > 0 {
                    app.vector_tab.vector_detail_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                app.vector_tab.vector_detail_scroll += 1;
                return true;
            }
            KeyCode::PageUp => {
                app.vector_tab.vector_detail_scroll = app.vector_tab.vector_detail_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                app.vector_tab.vector_detail_scroll += 10;
                return true;
            }
            KeyCode::Esc => {
                app.vector_tab.show_vector_detail = false;
                return true;
            }
            _ => {}
        }
    }

    // ── 条目操作弹窗 ──
    if app.vector_tab.entries_action_open {
        const VEC_ACTION_OPTIONS: &[&str] = &["查看向量", "删除向量"];
        match event.code {
            KeyCode::Up => {
                if app.vector_tab.entries_action_selected > 0 {
                    app.vector_tab.entries_action_selected -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = VEC_ACTION_OPTIONS.len().saturating_sub(1);
                if app.vector_tab.entries_action_selected < max {
                    app.vector_tab.entries_action_selected += 1;
                }
                return true;
            }
            KeyCode::Enter => {
                let idx = app.vector_tab.entries_action_selected;
                let sel = app.vector_tab.entries_selected;
                app.vector_tab.entries_action_open = false;
                if let Some(entry_idx) = sel {
                    if let Some((id, emb)) = app.vector_tab.entries.get(entry_idx) {
                        if idx == 0 {
                            // 查看向量
                            app.vector_tab.vector_detail_embedding = emb.clone();
                            app.vector_tab.vector_detail_scroll = 0;
                            app.vector_tab.show_vector_detail = true;
                        } else if idx == 1 {
                            // 删除向量
                            let del_id = *id;
                            let _ = crate::tab_vector::delete_single_embedding(app, del_id).await;
                        }
                    }
                }
                return true;
            }
            KeyCode::Esc => {
                app.vector_tab.entries_action_open = false;
                return true;
            }
            _ => {}
        }
    }

    // F1: 刷新所有索引信息
    // F2: 获取当前索引名的详细信息
    // F3: 列出当前索引的所有向量条目
    // F4: 清空当前索引的所有向量
    // F5: 一致性分析
    // F6: 从 RocksDB 重建索引
    match event.code {
        KeyCode::F(1) => {
            let _ = crate::tab_vector::get_all_indices(app).await;
            return true;
        }
        KeyCode::F(2) => {
            let _ = crate::tab_vector::get_index_info(app).await;
            return true;
        }
        KeyCode::F(3) => {
            let _ = crate::tab_vector::list_embeddings(app).await;
            return true;
        }
        KeyCode::F(4) => {
            let _ = crate::tab_vector::clear_embeddings(app).await;
            return true;
        }
        KeyCode::F(5) => {
            let _ = crate::tab_vector::analyze_consistency(app).await;
            return true;
        }
        KeyCode::F(6) => {
            let _ = crate::tab_vector::rebuild_index(app).await;
            return true;
        }
        // Enter: 下拉菜单选中 / 条目操作弹窗 / 查询索引信息
        KeyCode::Enter => {
            if app.vector_tab.show_dropdown {
                // 从下拉菜单选中索引
                if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                    app.vector_tab.index_name.set_value(info.name.clone());
                }
                app.vector_tab.show_dropdown = false;
                let _ = crate::tab_vector::get_index_info(app).await;
            } else if app.vector_tab.entries_selected.is_some() && !app.vector_tab.entries.is_empty() {
                // 条目操作弹窗
                app.vector_tab.entries_action_open = true;
                app.vector_tab.entries_action_selected = 0;
            } else if !app.vector_tab.index_name.value.is_empty() {
                let _ = crate::tab_vector::get_index_info(app).await;
            }
            return true;
        }
        // Tab: 切换下拉菜单展开/收起
        KeyCode::Tab => {
            if app.vector_tab.show_dropdown {
                // 下拉菜单内部导航
                let max = app.vector_tab.all_indices.len().saturating_sub(1);
                if app.vector_tab.selected_dropdown < max {
                    app.vector_tab.selected_dropdown += 1;
                    // 实时更新输入框值，下方表格同步显示
                    if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                        app.vector_tab.index_name.set_value(info.name.clone());
                    }
                }
            } else if !app.vector_tab.all_indices.is_empty() {
                // 第一次展开下拉菜单
                app.vector_tab.show_dropdown = true;
                // 默认选中第一个或当前匹配的
                if !app.vector_tab.index_name.value.is_empty() {
                    if let Some(pos) = app.vector_tab.all_indices.iter().position(|i| i.name == app.vector_tab.index_name.value) {
                        app.vector_tab.selected_dropdown = pos;
                    }
                }
            }
            return true;
        }
        // Down: 下拉菜单打开时导航菜单，否则导航条目列表
        KeyCode::Down => {
            if app.vector_tab.show_dropdown {
                let max = app.vector_tab.all_indices.len().saturating_sub(1);
                if app.vector_tab.selected_dropdown < max {
                    app.vector_tab.selected_dropdown += 1;
                    if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                        app.vector_tab.index_name.set_value(info.name.clone());
                    }
                }
            } else if !app.vector_tab.entries.is_empty() {
                // 条目列表导航
                let cur = app.vector_tab.entries_selected.unwrap_or(0);
                let max = app.vector_tab.entries.len().saturating_sub(1);
                if cur < max {
                    app.vector_tab.entries_selected = Some(cur + 1);
                    if cur + 1 >= app.vector_tab.entries_scroll + 10 {
                        app.vector_tab.entries_scroll = cur + 1 - 9;
                    }
                } else {
                    app.vector_tab.entries_selected = Some(0);
                }
            }
            return true;
        }
        KeyCode::Up => {
            if app.vector_tab.show_dropdown {
                if app.vector_tab.selected_dropdown > 0 {
                    app.vector_tab.selected_dropdown -= 1;
                    // 实时更新输入框值，下方表格同步显示
                    if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                        app.vector_tab.index_name.set_value(info.name.clone());
                    }
                }
            } else if !app.vector_tab.entries.is_empty() {
                // 条目列表导航
                let cur = app.vector_tab.entries_selected.unwrap_or(0);
                if cur > 0 {
                    app.vector_tab.entries_selected = Some(cur - 1);
                    if cur - 1 < app.vector_tab.entries_scroll {
                        app.vector_tab.entries_scroll = cur - 1;
                    }
                } else {
                    app.vector_tab.entries_selected = Some(0);
                }
            }
            return true;
        }
        KeyCode::PageUp => {
            if app.vector_tab.show_dropdown {
                app.vector_tab.selected_dropdown = app.vector_tab.selected_dropdown.saturating_sub(10);
                if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                    app.vector_tab.index_name.set_value(info.name.clone());
                }
            } else if !app.vector_tab.entries.is_empty() {
                let cur = app.vector_tab.entries_selected.unwrap_or(0);
                let new_scroll = app.vector_tab.entries_scroll.saturating_sub(10);
                app.vector_tab.entries_scroll = new_scroll;
                app.vector_tab.entries_selected = Some(cur.saturating_sub(10));
            }
            return true;
        }
        KeyCode::PageDown => {
            if app.vector_tab.show_dropdown {
                let max = app.vector_tab.all_indices.len().saturating_sub(1);
                app.vector_tab.selected_dropdown = (app.vector_tab.selected_dropdown + 10).min(max);
                if let Some(info) = app.vector_tab.all_indices.get(app.vector_tab.selected_dropdown) {
                    app.vector_tab.index_name.set_value(info.name.clone());
                }
            } else if !app.vector_tab.entries.is_empty() {
                let cur = app.vector_tab.entries_selected.unwrap_or(0);
                let max = app.vector_tab.entries.len().saturating_sub(1);
                let new_sel = (cur + 10).min(max);
                app.vector_tab.entries_selected = Some(new_sel);
                app.vector_tab.entries_scroll = app.vector_tab.entries_scroll.saturating_add(10);
            }
            return true;
        }
        // Esc 关闭下拉菜单
        KeyCode::Esc => {
            if app.vector_tab.show_dropdown {
                app.vector_tab.show_dropdown = false;
                return true;
            }
        }
        // F3: 列出当前索引的所有向量条目
        KeyCode::F(3) => {
            let _ = crate::tab_vector::list_embeddings(app).await;
            return true;
        }
        // F4: 清空当前索引的所有向量
        KeyCode::F(4) => {
            let _ = crate::tab_vector::clear_embeddings(app).await;
            return true;
        }
        // Enter on entries list → open action popup
        KeyCode::Char('\n') | KeyCode::Char('\r') => {
            if !app.vector_tab.show_dropdown
                && !app.vector_tab.entries.is_empty()
                && app.vector_tab.entries_selected.is_some()
            {
                app.vector_tab.entries_action_open = true;
                app.vector_tab.entries_action_selected = 0;
                return true;
            }
        }
        _ => {}
    }

    // 字符输入进入 index_name 输入框
    let input = &mut app.vector_tab.index_name;
    let handled = handle_input_event(input, event);
    if handled {
        // 用户输入时关闭下拉菜单
        app.vector_tab.show_dropdown = false;
    }
    handled
}

/// 处理 SQL Tab 的事件
async fn handle_sql_tab(app: &mut App, event: KeyEvent) -> bool {
    // ── 弹窗优先处理 ──

    // Schema 列表弹窗
    if app.sql_tab.show_schema_list {
        match event.code {
            KeyCode::Esc => {
                app.sql_tab.show_schema_list = false;
                return true;
            }
            KeyCode::Enter => {
                // 选中 schema 后自动填入 schema 输入框
                if let Some(schema) = app.sql_tab.schemas.get(app.sql_tab.schema_list_scroll) {
                    app.sql_tab.schema.set_value(schema);
                }
                app.sql_tab.show_schema_list = false;
                app.set_status(format!("已切换到 Schema '{}'", app.sql_tab.schema.value));
                return true;
            }
            KeyCode::Up => {
                if app.sql_tab.schema_list_scroll > 0 {
                    app.sql_tab.schema_list_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.sql_tab.schemas.len().saturating_sub(1);
                if app.sql_tab.schema_list_scroll < max {
                    app.sql_tab.schema_list_scroll += 1;
                }
                return true;
            }
            KeyCode::PageUp => {
                app.sql_tab.schema_list_scroll = app.sql_tab.schema_list_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.sql_tab.schemas.len().saturating_sub(1);
                app.sql_tab.schema_list_scroll = (app.sql_tab.schema_list_scroll + 10).min(max);
                return true;
            }
            _ => {}
        }
    }

    // 表列表弹窗
    if app.sql_tab.show_table_list {
        match event.code {
            KeyCode::Esc => {
                app.sql_tab.show_table_list = false;
                return true;
            }
            KeyCode::Enter => {
                if let Some(table) = app.sql_tab.tables.get(app.sql_tab.table_list_scroll) {
                    let table_name = table.clone();
                    // 关闭表列表并打开表描述
                    app.sql_tab.show_table_list = false;
                    app.sql_tab.desc_input_active = true;
                    app.sql_tab.desc_input.set_value(&table_name);
                    let _ = crate::tab_sql::describe_table(app, &table_name).await;
                }
                return true;
            }
            KeyCode::Up => {
                if app.sql_tab.table_list_scroll > 0 {
                    app.sql_tab.table_list_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.sql_tab.tables.len().saturating_sub(1);
                if app.sql_tab.table_list_scroll < max {
                    app.sql_tab.table_list_scroll += 1;
                }
                return true;
            }
            KeyCode::PageUp => {
                app.sql_tab.table_list_scroll = app.sql_tab.table_list_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.sql_tab.tables.len().saturating_sub(1);
                app.sql_tab.table_list_scroll = (app.sql_tab.table_list_scroll + 10).min(max);
                return true;
            }
            _ => {}
        }
    }

    // 表结构描述弹窗
    if app.sql_tab.show_table_desc {
        match event.code {
            KeyCode::Esc => {
                app.sql_tab.show_table_desc = false;
                return true;
            }
            KeyCode::Up => {
                if app.sql_tab.desc_scroll > 0 {
                    app.sql_tab.desc_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.sql_tab.table_columns.len().saturating_sub(1);
                if app.sql_tab.desc_scroll < max {
                    app.sql_tab.desc_scroll += 1;
                }
                return true;
            }
            KeyCode::PageUp => {
                app.sql_tab.desc_scroll = app.sql_tab.desc_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.sql_tab.table_columns.len().saturating_sub(1);
                app.sql_tab.desc_scroll = (app.sql_tab.desc_scroll + 10).min(max);
                return true;
            }
            _ => {}
        }
    }

    // 版本信息弹窗
    if app.sql_tab.show_version {
        match event.code {
            KeyCode::Esc => {
                app.sql_tab.show_version = false;
                return true;
            }
            _ => {}
        }
    }

    // 描述表名输入弹窗
    if app.sql_tab.desc_input_active {
        match event.code {
            KeyCode::Esc => {
                app.sql_tab.desc_input_active = false;
                return true;
            }
            KeyCode::Enter => {
                let table_name = app.sql_tab.desc_input.value.trim().to_string();
                app.sql_tab.desc_input_active = false;
                if !table_name.is_empty() {
                    let _ = crate::tab_sql::describe_table(app, &table_name).await;
                }
                return true;
            }
            _ => {
                return handle_input_event(&mut app.sql_tab.desc_input, event);
            }
        }
    }

    // ── 普通快捷键 ──
    // Ctrl+L 清空
    if event.code == KeyCode::Char('l') && event.modifiers.contains(KeyModifiers::CONTROL) {
        crate::tab_sql::clear_sql(app);
        return true;
    }
    // F1: 列出 Schema
    if event.code == KeyCode::F(1) {
        let _ = crate::tab_sql::list_schemas(app).await;
        return true;
    }
    // F2: 列出表
    if event.code == KeyCode::F(2) {
        let _ = crate::tab_sql::list_tables(app).await;
        return true;
    }
    // F3: 描述表结构（打开输入弹窗）
    if event.code == KeyCode::F(3) {
        app.sql_tab.desc_input_active = true;
        app.sql_tab.desc_input = crate::app::InputState::new();
        app.set_status("请输入表名后按 Enter");
        return true;
    }
    // F4: 版本信息
    if event.code == KeyCode::F(4) {
        let _ = crate::tab_sql::get_version(app).await;
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

/// 处理索引 Tab 的事件
async fn handle_index_tab(app: &mut App, event: KeyEvent) -> bool {
    // ── 弹窗优先处理 ──

    // 索引列表弹窗
    if app.index_tab.show_index_list {
        match event.code {
            KeyCode::Esc => {
                app.index_tab.show_index_list = false;
                return true;
            }
            KeyCode::Enter => {
                if let Some(idx_name) = app.index_tab.all_indices.get(app.index_tab.list_scroll) {
                    let name = idx_name.clone();
                    app.index_tab.show_index_list = false;
                    app.index_tab.index_name.set_value(&name);
                    let _ = crate::tab_index::get_index_detail(app).await;
                }
                return true;
            }
            KeyCode::Up => {
                if app.index_tab.list_scroll > 0 {
                    app.index_tab.list_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.index_tab.all_indices.len().saturating_sub(1);
                if app.index_tab.list_scroll < max {
                    app.index_tab.list_scroll += 1;
                }
                return true;
            }
            KeyCode::PageUp => {
                app.index_tab.list_scroll = app.index_tab.list_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.index_tab.all_indices.len().saturating_sub(1);
                app.index_tab.list_scroll = (app.index_tab.list_scroll + 10).min(max);
                return true;
            }
            _ => {}
        }
    }

    // 索引详情弹窗
    if app.index_tab.show_index_detail {
        match event.code {
            KeyCode::Esc => {
                app.index_tab.show_index_detail = false;
                return true;
            }
            KeyCode::Up => {
                if app.index_tab.detail_scroll > 0 {
                    app.index_tab.detail_scroll -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = app.index_tab.index_fields.len().saturating_sub(1);
                if app.index_tab.detail_scroll < max {
                    app.index_tab.detail_scroll += 1;
                }
                return true;
            }
            KeyCode::PageUp => {
                app.index_tab.detail_scroll = app.index_tab.detail_scroll.saturating_sub(10);
                return true;
            }
            KeyCode::PageDown => {
                let max = app.index_tab.index_fields.len().saturating_sub(1);
                app.index_tab.detail_scroll = (app.index_tab.detail_scroll + 10).min(max);
                return true;
            }
            _ => {}
        }
    }

    // 索引统计弹窗
    if app.index_tab.show_index_stats {
        match event.code {
            KeyCode::Esc => {
                app.index_tab.show_index_stats = false;
                return true;
            }
            _ => {}
        }
    }

    // 搜索结果弹窗
    if app.index_tab.show_search_results {
        let n = app.index_tab.search_results.len();
        match event.code {
            KeyCode::Esc => {
                app.index_tab.show_search_results = false;
                app.index_tab.search_results.clear();
                app.index_tab.search_scroll = 0;
                app.index_tab.search_selected = None;
                return true;
            }
            KeyCode::Up => {
                if n > 0 {
                    let cur = app.index_tab.search_selected.unwrap_or(0);
                    app.index_tab.search_selected = Some(if cur == 0 { n - 1 } else { cur - 1 });
                }
                return true;
            }
            KeyCode::Down => {
                if n > 0 {
                    let cur = app.index_tab.search_selected.unwrap_or(0);
                    app.index_tab.search_selected = Some(if cur >= n - 1 { 0 } else { cur + 1 });
                }
                return true;
            }
            KeyCode::PageUp => {
                if n > 0 {
                    let cur = app.index_tab.search_selected.unwrap_or(0);
                    app.index_tab.search_selected = Some(cur.saturating_sub(10));
                }
                return true;
            }
            KeyCode::PageDown => {
                if n > 0 {
                    let cur = app.index_tab.search_selected.unwrap_or(0);
                    app.index_tab.search_selected = Some((cur + 10).min(n - 1));
                }
                return true;
            }
            _ => {}
        }
    }

    // 搜索查询输入弹窗
    if app.index_tab.search_input_active {
        match event.code {
            KeyCode::Esc => {
                app.index_tab.search_input_active = false;
                return true;
            }
            KeyCode::Enter => {
                let query = app.index_tab.search_input.value.trim().to_string();
                app.index_tab.search_input_active = false;
                if !query.is_empty() {
                    let _ = crate::tab_index::search_index(app, &query).await;
                }
                return true;
            }
            _ => {
                return handle_input_event(&mut app.index_tab.search_input, event);
            }
        }
    }

    // ── 普通快捷键 ──
    // F1: 列出所有索引
    if event.code == KeyCode::F(1) {
        let _ = crate::tab_index::list_indices(app).await;
        return true;
    }
    // F2: 查看索引详情
    if event.code == KeyCode::F(2) {
        let _ = crate::tab_index::get_index_detail(app).await;
        return true;
    }
    // F3: 查看索引统计
    if event.code == KeyCode::F(3) {
        let _ = crate::tab_index::get_index_stats(app).await;
        return true;
    }
    // F4: 搜索索引
    if event.code == KeyCode::F(4) {
        app.index_tab.search_input_active = true;
        app.index_tab.search_input = crate::app::InputState::new();
        app.set_status("请输入搜索查询后按 Enter");
        return true;
    }
    // Enter 在输入框非空时获取索引详情
    if event.code == KeyCode::Enter && !app.index_tab.index_name.value.is_empty() {
        let _ = crate::tab_index::get_index_detail(app).await;
        return true;
    }

    // 字符输入进入 index_name 输入框
    let input = &mut app.index_tab.index_name;
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
            } else if app.current_tab == Tab::Vector && !app.vector_tab.entries.is_empty() {
                // 布局：Tab栏(3) + 输入框(3) + 索引信息(18) + 条目表格边框(1) + 表头(1) → 数据行起始 y
                let data_start_y = 3 + 3 + 18 + 2; // = 26
                if event.row >= data_start_y {
                    let row = (event.row - data_start_y) as usize + app.vector_tab.entries_scroll;
                    if row < app.vector_tab.entries.len() {
                        app.vector_tab.entries_selected = Some(row);
                        // 调整滚动使选中行可见
                        let visible = 10usize;
                        if row < app.vector_tab.entries_scroll {
                            app.vector_tab.entries_scroll = row;
                        } else if row >= app.vector_tab.entries_scroll + visible {
                            app.vector_tab.entries_scroll = row - visible + 1;
                        }
                    }
                }
            }
        }
        _ => {}
    }
}
