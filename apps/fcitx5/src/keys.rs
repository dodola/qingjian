//! 平台无关按键路由（移植自 macOS 壳 `apps/macos/src/imk/controller.rs` 的分发，M0 版）。
//!
//! 这里只做「键 → Engine 调用 → 会话状态」；fcitx5 的 keysym 由 C++ 侧归一化后喂进来。
//! M0 覆盖字母 / 退格 / 回车 / 空格 / 数字选词 / 方向 / 翻页 / Esc / 标点；完整的英文直输段、
//! 问字、表达式、修饰键快捷键与英文模式见 docs/design/linux-fcitx5.md 的 M2。

use crate::Global;
use crate::session::QjSession;

pub const MOD_SHIFT: u32 = 1;
pub const MOD_CONTROL: u32 = 4;
pub const MOD_SUPER: u32 = 64;

const KEYSYM_BACKSPACE: u32 = 0xff08;
const KEYSYM_TAB: u32 = 0xff09;
const KEYSYM_RETURN: u32 = 0xff0d;
const KEYSYM_ESCAPE: u32 = 0xff1b;
const KEYSYM_UP: u32 = 0xff52;
const KEYSYM_DOWN: u32 = 0xff54;
const KEYSYM_PAGE_UP: u32 = 0xff55;
const KEYSYM_PAGE_DOWN: u32 = 0xff56;
const KEYSYM_DELETE: u32 = 0xffff;
const KEYSYM_KP_ENTER: u32 = 0xff8d;

/// 处理一次按下。返回是否吃掉（fcitx5 `filterAndAccept`）。
pub fn process(global: &mut Global, session: &mut QjSession, keysym: u32, modifiers: u32) -> bool {
    let composing = !global.engine.composition().is_empty();
    // Ctrl / Super 组合留给应用（M0；快捷键见 M2）。
    if modifiers & (MOD_CONTROL | MOD_SUPER) != 0 {
        return false;
    }
    match keysym {
        KEYSYM_BACKSPACE if composing => {
            global.engine.backspace();
            recompose(global, session);
            true
        }
        KEYSYM_DELETE if composing => {
            global.engine.delete_forward();
            recompose(global, session);
            true
        }
        KEYSYM_ESCAPE if composing => {
            global.engine.clear();
            session.clear();
            true
        }
        KEYSYM_RETURN | KEYSYM_KP_ENTER if composing => commit_raw(global, session),
        KEYSYM_TAB if composing => {
            if session.turn_page(1) {
                global.engine.note_page_turn();
            }
            true
        }
        KEYSYM_UP if composing => {
            session.move_highlight(-1);
            true
        }
        KEYSYM_DOWN if composing => {
            session.move_highlight(1);
            true
        }
        KEYSYM_PAGE_UP if composing => {
            if session.turn_page(-1) {
                global.engine.note_page_turn();
            }
            true
        }
        KEYSYM_PAGE_DOWN if composing => {
            if session.turn_page(1) {
                global.engine.note_page_turn();
            }
            true
        }
        _ => text(global, session, keysym, modifiers & MOD_SHIFT != 0),
    }
}

fn text(global: &mut Global, session: &mut QjSession, keysym: u32, shift: bool) -> bool {
    let Some(character) = keysym_to_char(keysym, shift) else {
        return false;
    };
    if !global.engine.composition().is_empty() {
        if character == ' ' {
            return commit_highlighted(global, session);
        }
        if ('1'..='9').contains(&character) {
            let listed = (character as u8 - b'1') as usize;
            if session.candidate_listed(listed).is_some() {
                return commit_listed(global, session, listed);
            }
            return commit_raw(global, session);
        }
        let (previous, next) = global.page_keys;
        if character == previous {
            if session.turn_page(-1) {
                global.engine.note_page_turn();
            }
            return true;
        }
        if character == next {
            if session.turn_page(1) {
                global.engine.note_page_turn();
            }
            return true;
        }
        if is_pinyin_key(character) {
            global.engine.push(character);
            recompose(global, session);
            return true;
        }
        // 其他字符：先把当前高亮上屏，再看能不能补一个全角标点（`nihao,` → 你好，）。
        let mut committed = commit_highlighted_text(global, session);
        if let Some(full_width) = global.engine.punctuate(character) {
            match &mut committed {
                Some(text) => text.push_str(full_width),
                None => committed = Some(full_width.to_owned()),
            }
        }
        if let Some(text) = committed {
            session.set_commit(&text);
            recompose(global, session);
            return true;
        }
        return false;
    }
    // 不在组句中：标点转全角上屏；字母开始组句；其余交给应用。
    if let Some(full_width) = global.engine.punctuate(character) {
        session.set_commit(full_width);
        return true;
    }
    if character.is_ascii_lowercase() {
        global.engine.push(character);
        recompose(global, session);
        return true;
    }
    false
}

fn commit_highlighted(global: &mut Global, session: &mut QjSession) -> bool {
    let Some(text) = commit_highlighted_text(global, session) else {
        return false;
    };
    session.set_commit(&text);
    recompose(global, session);
    true
}

fn commit_listed(global: &mut Global, session: &mut QjSession, listed: usize) -> bool {
    let Some(candidate) = session.candidate_listed(listed) else {
        return false;
    };
    let text = global.engine.commit(&candidate);
    session.set_commit(&text);
    recompose(global, session);
    true
}

fn commit_raw(global: &mut Global, session: &mut QjSession) -> bool {
    let raw = global.engine.take_raw();
    if raw.is_empty() {
        return false;
    }
    session.set_commit(&raw);
    recompose(global, session);
    true
}

/// 取出当前高亮该上屏的文本（没有候选就原样上屏拼音），但先不记录，方便调用方追加标点。
fn commit_highlighted_text(global: &mut Global, session: &QjSession) -> Option<String> {
    if let Some(candidate) = session.candidate_listed(session.highlighted_listed()) {
        return Some(global.engine.commit(&candidate));
    }
    let raw = global.engine.take_raw();
    (!raw.is_empty()).then_some(raw)
}

fn recompose(global: &Global, session: &mut QjSession) {
    session.recompose(&global.engine, global.page_size, global.slots);
}

fn is_pinyin_key(character: char) -> bool {
    character.is_ascii_lowercase() || character == '\''
}

/// ASCII keysym 转字符；字母按 Shift 决定大小写，其余原样。M0 只处理可打印 ASCII。
fn keysym_to_char(keysym: u32, shift: bool) -> Option<char> {
    match keysym {
        0x20..=0x7e => {
            let character = keysym as u8 as char;
            if shift && character.is_ascii_alphabetic() {
                Some(character.to_ascii_uppercase())
            } else {
                Some(character)
            }
        }
        _ => None,
    }
}
