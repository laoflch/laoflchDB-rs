//! App 全局状态管理
//!
//! 包含 4 个 Tab 的状态、gRPC 客户端、登录状态、命令模式输入等。

use crate::grpc_client::GrpcClients;

/// 当前激活的 Tab
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Image,
    Face,
    Vector,
    Sql,
}

impl Tab {
    /// 转为 Tab 栏显示用的中文名称
    pub fn title(self) -> &'static str {
        match self {
            Tab::Image => "1:图片",
            Tab::Face => "2:人脸",
            Tab::Vector => "3:向量",
            Tab::Sql => "4:SQL",
        }
    }

    /// Tab 在栏中的索引（0..4）
    pub fn index(self) -> usize {
        match self {
            Tab::Image => 0,
            Tab::Face => 1,
            Tab::Vector => 2,
            Tab::Sql => 3,
        }
    }

    /// 按数字键 1..=4 切换 Tab
    pub fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Tab::Image),
            1 => Some(Tab::Face),
            2 => Some(Tab::Vector),
            3 => Some(Tab::Sql),
            _ => None,
        }
    }
}

/// 单个输入框的状态
#[derive(Debug, Clone)]
pub struct InputState {
    pub value: String,
    pub cursor: usize,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    pub fn new() -> Self {
        Self {
            value: String::new(),
            cursor: 0,
        }
    }

    pub fn with_value(v: impl Into<String>) -> Self {
        let value = v.into();
        let cursor = value.chars().count();
        Self { value, cursor }
    }

    pub fn set_value(&mut self, v: impl Into<String>) {
        self.value = v.into();
        self.cursor = self.value.chars().count();
    }

    /// 清空输入框
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// 在光标位置插入字符（支持 UTF-8 中文）
    pub fn insert_char(&mut self, c: char) {
        let idx = self.char_to_byte_idx();
        self.value.insert(idx, c);
        self.cursor += 1;
    }

    /// 在光标位置插入字符串
    pub fn insert_str(&mut self, s: &str) {
        let idx = self.char_to_byte_idx();
        let n = s.chars().count();
        self.value.insert_str(idx, s);
        self.cursor += n;
    }

    /// 删除光标前一个字符（Backspace）
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let idx = self.char_to_byte_idx();
            // 找到前一个字符的字节边界
            let prev = self.value[..idx].char_indices().last().map(|(b, _)| b);
            if let Some(prev_idx) = prev {
                self.value.remove(prev_idx);
                self.cursor -= 1;
            }
        }
    }

    /// 删除光标位置的字符（Delete）
    pub fn delete(&mut self) {
        let idx = self.char_to_byte_idx();
        if idx < self.value.len() {
            // 找到当前字符的字节长度
            if let Some(c) = self.value[idx..].chars().next() {
                let len = c.len_utf8();
                let _ = len;
                self.value.remove(idx);
            }
        }
    }

    /// 光标左移
    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// 光标右移
    pub fn right(&mut self) {
        let total = self.value.chars().count();
        if self.cursor < total {
            self.cursor += 1;
        }
    }

    /// 光标到开头
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// 光标到结尾
    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }

    /// 光标位置（终端列数）
    pub fn cursor_pos(&self) -> u16 {
        self.cursor as u16
    }

    fn char_to_byte_idx(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }
}

/// 图片 Tab 状态
pub struct ImageTabState {
    /// 焦点：仅 FilePath（bucket/key 在状态栏中直接编辑，不再单独占框）
    pub focus: ImageFocus,
    pub bucket: InputState,
    pub file_path: InputState,
    pub key: InputState,
    pub images: Vec<laoflchdb_image_service_proto::proto::ImageMetadata>,
    pub upload_result: Option<String>,
    pub meta_detail: Option<String>,
    /// 当前选中行的索引，None 表示无选中
    pub selected_index: Option<usize>,
    pub list_scroll: usize,
    /// 本地路径输入框的补全下拉菜单
    pub path_popup: PathPopup,
    /// 本地文件操作弹窗：选择文件后弹出，提供上传和向量搜索两个 Tab
    pub local_file_action: Option<LocalFileAction>,
    /// 图片操作弹窗（选中列表中图片后按 Enter 弹出）
    pub action_popup_open: bool,
    /// 操作弹窗中当前选中的选项索引
    pub action_popup_selected: usize,
    /// 删除确认弹窗：存储待删除的 key
    pub delete_confirm: Option<String>,
    /// 下载确认弹窗：存储待下载的 key
    pub download_confirm: Option<String>,
    /// 下载保存路径输入
    pub download_path: InputState,
    /// 下载路径的滚动偏移（行数）
    pub download_path_scroll: usize,
    /// 向量搜索结果显示
    pub search_results: Vec<SearchResultItem>,
    /// 是否显示搜索弹窗
    pub show_search_results: bool,
    /// 搜索结果的滚动偏移
    pub search_results_scroll: usize,
}

/// 向量搜索结果项
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub id: u64,
    pub score: f32,
}

/// 本地文件操作弹窗：选择文件后弹出，提供上传和向量搜索两个 Tab
pub struct LocalFileAction {
    /// 文件路径
    pub file_path: String,
    /// 当前选中的 Tab：0=上传，1=向量搜索
    pub tab: usize,
    /// 向量索引模型名称
    pub model_name: InputState,
    /// 向量索引名称
    pub index_name: InputState,
    /// 向量维度（0 表示使用模型默认维度）
    pub dim: InputState,
    /// 可选模型列表（用于在向量搜索 Tab 中 ↑/↓ 切换）
    pub models: Vec<String>,
    /// 当前模型在 models 中的索引
    pub model_index: usize,
    /// 搜索返回 top_k
    pub top_k: InputState,
    /// 距离最大值（过滤掉距离大于此值的结果，默认 0.1）
    pub max_distance: InputState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFocus {
    FilePath,
}

impl Default for ImageTabState {
    fn default() -> Self {
        Self {
            focus: ImageFocus::FilePath,
            bucket: InputState::with_value("images"),
            file_path: InputState::new(),
            key: InputState::new(),
            images: Vec::new(),
            upload_result: None,
            meta_detail: None,
            selected_index: None,
            list_scroll: 0,
            path_popup: PathPopup::default(),
            local_file_action: None,
            action_popup_open: false,
            action_popup_selected: 0,
            delete_confirm: None,
            download_confirm: None,
            download_path: InputState::new(),
            download_path_scroll: 0,
            search_results: Vec::new(),
            show_search_results: false,
            search_results_scroll: 0,
        }
    }
}

/// 人脸 Tab 状态
pub struct FaceTabState {
    pub focus: FaceFocus,
    pub file_path: InputState,
    pub det_threshold: InputState,
    pub max_faces: InputState,
    pub save_aligned_images: bool,
    pub index_embedding: bool,
    pub bucket: InputState,
    /// 检测到的人脸列表（编号 / score / bbox / saved_key / vector_id）
    pub faces: Vec<(usize, f32, Vec<f32>, String, u64)>,
    /// 当前选中的人脸索引，用于显示 embedding 预览
    pub selected_face: usize,
    /// 当前选中人脸的 embedding（前 10 个值预览用）
    pub embedding_preview: Vec<f32>,
    pub list_scroll: usize,
    /// 本地路径输入框的补全下拉菜单
    pub path_popup: PathPopup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceFocus {
    FilePath,
    DetThreshold,
    MaxFaces,
    Bucket,
}

impl Default for FaceTabState {
    fn default() -> Self {
        Self {
            focus: FaceFocus::FilePath,
            file_path: InputState::new(),
            det_threshold: InputState::with_value("0.5"),
            max_faces: InputState::with_value("0"),
            save_aligned_images: true,
            index_embedding: true,
            bucket: InputState::with_value("faces"),
            faces: Vec::new(),
            selected_face: 0,
            embedding_preview: Vec::new(),
            list_scroll: 0,
            path_popup: PathPopup::default(),
        }
    }
}

/// 向量 Tab 状态
#[derive(Debug, Clone)]
pub struct VectorTabState {
    pub focus: VectorFocus,
    pub index_name: InputState,
    pub query_vec: InputState,
    pub top_k: InputState,
    /// 索引信息（num_elements, dim, distance_metric, max_layers）
    pub index_info: Option<(u64, u32, String, u32)>,
    /// 搜索结果列表 (id, distance)
    pub search_results: Vec<(u64, f32)>,
    pub delete_id: InputState,
    pub list_scroll: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorFocus {
    IndexName,
    QueryVec,
    TopK,
    DeleteId,
}

impl Default for VectorTabState {
    fn default() -> Self {
        Self {
            focus: VectorFocus::IndexName,
            index_name: InputState::with_value("face"),
            query_vec: InputState::new(),
            top_k: InputState::with_value("5"),
            index_info: None,
            search_results: Vec::new(),
            delete_id: InputState::new(),
            list_scroll: 0,
        }
    }
}

/// SQL Tab 状态
#[derive(Debug, Clone)]
pub struct SqlTabState {
    pub sql: InputState,
    pub schema: InputState,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub list_scroll: usize,
    pub focus_sql: bool,
}

/// 路径补全下拉菜单状态
///
/// 当路径输入框获得焦点且有候选时显示。Up/Down 导航，Enter 选中，Esc 关闭。
#[derive(Default)]
pub struct PathPopup {
    /// 是否显示
    pub active: bool,
    /// 候选列表
    pub candidates: Vec<crate::path_complete::Candidate>,
    /// 当前选中的索引
    pub selected: usize,
    /// 顶部滚动偏移（用于候选过多时）
    pub scroll: usize,
    /// 当前可见行数（由渲染时动态更新）
    pub visible: usize,
}

impl PathPopup {
    pub fn is_active(&self) -> bool {
        self.active && !self.candidates.is_empty()
    }

    /// 选中前一个候选
    pub fn prev(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.candidates.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.adjust_scroll();
    }

    /// 选中下一个候选
    pub fn next(&mut self) {
        if self.candidates.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.candidates.len();
        self.adjust_scroll();
    }

    /// 根据可见行数调整滚动，确保 selected 始终在可见范围内
    fn adjust_scroll(&mut self) {
        let page = self.visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + page {
            self.scroll = self.selected - page + 1;
        }
    }

    /// 当前选中的候选
    pub fn current(&self) -> Option<&crate::path_complete::Candidate> {
        self.candidates.get(self.selected)
    }

    /// 关闭弹窗
    pub fn close(&mut self) {
        self.active = false;
        self.candidates.clear();
        self.selected = 0;
        self.scroll = 0;
    }

    /// 用新候选刷新弹窗。候选非空时自动激活；selected 夹取到合法范围。
    pub fn refresh(&mut self, candidates: Vec<crate::path_complete::Candidate>) {
        self.candidates = candidates;
        if self.candidates.is_empty() {
            self.active = false;
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.active = true;
            if self.selected >= self.candidates.len() {
                self.selected = 0;
                self.scroll = 0;
            }
            self.adjust_scroll();
        }
    }

    /// 打开弹窗并设置候选
    pub fn open(&mut self, candidates: Vec<crate::path_complete::Candidate>) {
        self.candidates = candidates;
        self.selected = 0;
        self.scroll = 0;
        self.active = !self.candidates.is_empty();
    }
}

impl Default for SqlTabState {
    fn default() -> Self {
        Self {
            sql: InputState::with_value("SELECT 1"),
            schema: InputState::with_value("sys"),
            columns: Vec::new(),
            rows: Vec::new(),
            list_scroll: 0,
            focus_sql: true,
        }
    }
}

/// 命令模式状态
#[derive(Debug, Clone)]
pub struct CommandMode {
    pub active: bool,
    pub input: InputState,
}

impl Default for CommandMode {
    fn default() -> Self {
        Self {
            active: false,
            input: InputState::new(),
        }
    }
}

/// 全局 App 状态
pub struct App {
    pub clients: Option<GrpcClients>,
    pub current_tab: Tab,
    pub host: String,
    pub username: String,
    pub password: String,
    pub logged_in: bool,
    pub status_message: String,
    pub status_is_error: bool,
    pub should_quit: bool,
    pub image_tab: ImageTabState,
    pub face_tab: FaceTabState,
    pub vector_tab: VectorTabState,
    pub sql_tab: SqlTabState,
    pub command_mode: CommandMode,
}

impl App {
    pub fn new(host: String, username: String, password: String) -> Self {
        Self {
            clients: None,
            current_tab: Tab::Image,
            host,
            username,
            password,
            logged_in: false,
            status_message: "ltool - LaoflchDB TUI 客户端，Alt+1~4 切换 Tab，Ctrl+Q 退出".to_string(),
            status_is_error: false,
            should_quit: false,
            image_tab: ImageTabState::default(),
            face_tab: FaceTabState::default(),
            vector_tab: VectorTabState::default(),
            sql_tab: SqlTabState::default(),
            command_mode: CommandMode::default(),
        }
    }

    /// 设置普通状态消息
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_is_error = false;
    }

    /// 设置错误状态消息
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
        self.status_is_error = true;
    }

    /// 切换到下一个 Tab（Tab 键）
    pub fn next_tab(&mut self) {
        self.clear_image_tab_popups();
        self.current_tab = match self.current_tab {
            Tab::Image => Tab::Face,
            Tab::Face => Tab::Vector,
            Tab::Vector => Tab::Sql,
            Tab::Sql => Tab::Image,
        };
    }

    /// 切换到上一个 Tab（Shift+Tab）
    pub fn prev_tab(&mut self) {
        self.clear_image_tab_popups();
        self.current_tab = match self.current_tab {
            Tab::Image => Tab::Sql,
            Tab::Face => Tab::Image,
            Tab::Vector => Tab::Face,
            Tab::Sql => Tab::Vector,
        };
    }

    /// 清除图片 Tab 的所有弹窗
    pub fn clear_image_tab_popups(&mut self) {
        self.image_tab.local_file_action = None;
        self.image_tab.action_popup_open = false;
        self.image_tab.delete_confirm = None;
        self.image_tab.download_confirm = None;
        self.image_tab.download_path.clear();
        self.image_tab.download_path_scroll = 0;
        self.image_tab.search_results.clear();
        self.image_tab.show_search_results = false;
        self.image_tab.search_results_scroll = 0;
        self.image_tab.path_popup.close();
    }

    /// 进入命令模式
    pub fn enter_command(&mut self) {
        self.command_mode.active = true;
        self.command_mode.input = InputState::new();
    }

    /// 退出命令模式
    pub fn exit_command(&mut self) {
        self.command_mode.active = false;
    }

    /// 检查是否已登录，未登录则设置状态提示并返回 false
    pub fn require_login(&mut self) -> bool {
        if self.logged_in {
            true
        } else {
            self.set_error("请先登录（输入 :login 用户名 密码）");
            false
        }
    }
}
