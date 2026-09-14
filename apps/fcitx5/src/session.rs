//! 每个 fcitx5 `InputContext` 的 UI 状态：候选排布、高亮、页码、preedit。
//!
//! 逻辑移植自 macOS 壳的 `apps/macos/src/host/session.rs`（候选分页与云端词位置由 Core 的
//! [`CandidateLayout`] 定，这里只管高亮与页码），差别是输出按 C ABI 的 getter 缓存成 `CString`。

use std::ffi::CString;

use qingjian_core::{Candidate, CandidateLayout, Cell};

fn cstring(text: &str) -> CString {
    CString::new(text).unwrap_or_default()
}

/// 一个候选格，文本与注释（词性 + 译文）都预先 NUL 结尾，getter 直接返回指针。
pub struct CandidateSlot {
    pub text: CString,
    pub comment: CString,
    pub cloud: bool,
}

/// 一次查询的一帧。所有 getter 读这里，直到下一次 [`QjSession::recompose`] 重建。
#[derive(Default)]
pub struct QjSession {
    /// 上一次查询的候选排布（本地候选 + 云端词）。
    layout: CandidateLayout,

    /// 高亮的格子下标（在整个排布里的绝对位置）。
    highlighted: usize,

    /// 当前页。
    page: usize,

    /// 这轮查询里用户用方向键 / 翻页键动过高亮（英文模式空格是否选词的判据）。
    navigated: bool,

    /// preedit 整串与光标（按字符数），另存分段供将来按段样式。
    preedit_text: CString,
    preedit_cursor: usize,
    preedit_segments: Vec<CString>,

    /// 上屏文本（commit），没有则为空。
    commit: Option<CString>,

    /// 短提示（整句补全 / 删候选提示），暂未接。
    aux: Option<CString>,

    /// 当前页实际列出的候选（跳过空格）；`page_indices[i]` 是它在整个排布里的绝对下标。
    slots: Vec<CandidateSlot>,
    page_indices: Vec<usize>,

    /// 高亮在 `slots` 里的下标（`frame` 的 cursor）。
    cursor: usize,
}

impl QjSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// 清空：缓冲区被放弃 / 会话结束。
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// 按当前缓冲区重新查询、标注、分页。查询失败（整段切不动）时退回显示原始字母。
    pub fn recompose(&mut self, engine: &qingjian_core::Engine, page_size: usize, slots: usize) {
        let mut preedit: String = engine.composition().text().to_owned();
        let mut cursor = engine.composition().cursor();
        let mut segments: Vec<CString> = Vec::new();
        match engine.query() {
            Ok(mut query) => {
                engine.annotate(&mut query.candidates);
                preedit = query.marked_text();
                cursor = query.marked_cursor();
                segments = query
                    .marked_segments()
                    .iter()
                    .map(|segment| cstring(&segment.text))
                    .collect();
                self.layout = CandidateLayout::new(query.candidates.items, page_size, slots);
            }
            Err(_) => {
                segments.push(cstring(&preedit));
                self.layout = CandidateLayout::default();
            }
        }
        self.preedit_text = cstring(&preedit);
        self.preedit_cursor = cursor;
        self.preedit_segments = segments;
        self.highlighted = first_candidate(&self.layout);
        self.page = self.highlighted / self.layout.page_size().max(1);
        self.navigated = false;
        self.rebuild_page();
    }

    /// 上屏文本的记录（commit 后由 C++ 取走）。
    pub fn set_commit(&mut self, text: &str) {
        self.commit = Some(cstring(text));
    }

    pub fn commit_ptr(&self) -> *const std::ffi::c_char {
        self.commit
            .as_ref()
            .map_or(std::ptr::null(), |text| text.as_ptr())
    }

    pub fn has_commit(&self) -> bool {
        self.commit.is_some()
    }

    /// 每次按键前清掉上一次的上屏结果。
    pub fn clear_commit(&mut self) {
        self.commit = None;
    }

    pub fn preedit_is_empty(&self) -> bool {
        self.preedit_text.is_empty()
    }

    pub fn preedit_ptr(&self) -> *const std::ffi::c_char {
        self.preedit_text.as_ptr()
    }

    pub fn preedit_cursor(&self) -> usize {
        self.preedit_cursor
    }

    pub fn preedit_segment_count(&self) -> usize {
        self.preedit_segments.len()
    }

    pub fn preedit_segment_ptr(&self, index: usize) -> *const std::ffi::c_char {
        self.preedit_segments
            .get(index)
            .map_or(std::ptr::null(), |segment| segment.as_ptr())
    }

    pub fn aux_ptr(&self) -> *const std::ffi::c_char {
        self.aux
            .as_ref()
            .map_or(std::ptr::null(), |text| text.as_ptr())
    }

    pub fn candidate_count(&self) -> usize {
        self.slots.len()
    }

    pub fn candidate_text_ptr(&self, index: usize) -> *const std::ffi::c_char {
        self.slots
            .get(index)
            .map_or(std::ptr::null(), |slot| slot.text.as_ptr())
    }

    pub fn candidate_comment_ptr(&self, index: usize) -> *const std::ffi::c_char {
        self.slots
            .get(index)
            .map_or(std::ptr::null(), |slot| slot.comment.as_ptr())
    }

    pub fn candidate_is_cloud(&self, index: usize) -> bool {
        self.slots.get(index).is_some_and(|slot| slot.cloud)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn page_index(&self) -> usize {
        self.page
    }

    pub fn page_count(&self) -> usize {
        self.layout.pages().max(1)
    }

    pub fn show_candidates(&self) -> bool {
        !self.slots.is_empty()
    }

    pub fn navigated(&self) -> bool {
        self.navigated
    }

    /// 整个排布里第 `absolute` 个候选的克隆。
    pub fn candidate(&self, absolute: usize) -> Option<Candidate> {
        self.layout.candidate(absolute).cloned()
    }

    /// 当前页实际列出的第 `listed` 个候选（对应 `qj_select` 的入参）。
    pub fn candidate_listed(&self, listed: usize) -> Option<Candidate> {
        let absolute = *self.page_indices.get(listed)?;
        self.candidate(absolute)
    }

    /// 当前页列出第几个就上屏第几个：`listed` 是列出顺序，不是页偏移（空格被跳过）。
    pub fn highlighted_listed(&self) -> usize {
        self.cursor
    }

    /// 高亮上下移动，越过页边自动翻页。返回是否有变化。
    pub fn move_highlight(&mut self, delta: isize) -> bool {
        let len = self.layout.len();
        if len == 0 {
            return false;
        }
        let current = self.highlighted as isize;
        let mut next = (current + delta).clamp(0, len as isize - 1) as usize;
        while self.layout.candidate(next).is_none() {
            let candidate = next as isize + delta.signum();
            if candidate < 0 || candidate >= len as isize || delta == 0 {
                return false;
            }
            next = candidate as usize;
        }
        if next == self.highlighted {
            return false;
        }
        self.highlighted = next;
        self.page = next / self.layout.page_size().max(1);
        self.navigated = true;
        self.rebuild_page();
        true
    }

    /// 翻页并选中新页第一个真实候选，跳过没有候选的页。
    pub fn turn_page(&mut self, delta: isize) -> bool {
        let pages = self.layout.pages().max(1);
        let current = self.page as isize;
        let mut next = (current + delta).clamp(0, pages as isize - 1) as usize;
        loop {
            if next == self.page {
                return false;
            }
            let size = self.layout.page_size().max(1);
            let start = next * size;
            if let Some(index) = (start..(start + size).min(self.layout.len()))
                .find(|&i| self.layout.candidate(i).is_some())
            {
                self.page = next;
                self.highlighted = index;
                break;
            }
            let following = next as isize + delta.signum();
            if following < 0 || following >= pages as isize || delta == 0 {
                return false;
            }
            next = following as usize;
        }
        self.navigated = true;
        self.rebuild_page();
        true
    }

    /// 重建当前页的列出候选与高亮下标。
    fn rebuild_page(&mut self) {
        self.slots.clear();
        self.page_indices.clear();
        let size = self.layout.page_size().max(1);
        let page = self.page;
        let cells = self.layout.page(page);
        for (offset, cell) in cells.iter().enumerate() {
            let Some(candidate) = cell.candidate() else {
                continue;
            };
            self.page_indices.push(page * size + offset);
            self.slots.push(CandidateSlot {
                text: cstring(&candidate.text),
                comment: cstring(&comment_of(candidate)),
                cloud: matches!(cell, Cell::Cloud(_)),
            });
        }
        self.cursor = self
            .page_indices
            .iter()
            .position(|&absolute| absolute == self.highlighted)
            .unwrap_or(0);
    }
}

fn first_candidate(layout: &CandidateLayout) -> usize {
    (0..layout.len())
        .find(|&index| layout.candidate(index).is_some())
        .unwrap_or(0)
}

/// 候选右侧的注释：`词性 译文`，多个义项用 ` · ` 连；没有译文就是空。
fn comment_of(candidate: &Candidate) -> String {
    let Some(translation) = &candidate.translation else {
        return String::new();
    };
    translation
        .senses()
        .iter()
        .map(|sense| {
            let part_of_speech = sense
                .part_of_speech
                .map(|part| format!("{part} "))
                .unwrap_or_default();
            format!("{part_of_speech}{}", sense.text)
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
