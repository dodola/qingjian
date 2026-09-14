//! 青简 Fcitx5 壳的 Rust 侧：持一个全局 Engine，经 C ABI 被 C++ addon 调用。
//!
//! 设计见 `docs/design/linux-fcitx5.md`。所有函数只从 fcitx5 主线程调用；字符串是 UTF-8，
//! getter 返回的指针在下一次调用前有效，C++ 侧立即拷贝。M0 只覆盖「装载 Engine + 按键出候选 +
//! 上屏」，C++ addon 与打包在 M1 / M5。

#![allow(clippy::missing_safety_doc)]
// 这是一层 C ABI：入参是 C++ 侧持有的会话句柄，有效性由「只从 fcitx5 主线程调用」这条约定保证，
// 逐个函数标 `unsafe` 只是噪音（clippy 的 not_unsafe_ptr_arg_deref 针对 Rust 调用方，不适用于 C）。
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod engine;
mod keys;
mod logging;
mod paths;
mod session;

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

pub use session::QjSession;

use engine::Assembled;

/// 进程内唯一的 Engine。fcitx5 是单线程主循环，用 thread_local 避开 `Send`（candle 不要求 `Send`）。
pub(crate) struct Global {
    pub(crate) engine: qingjian_core::Engine,
    pub(crate) page_size: usize,
    pub(crate) slots: usize,
    pub(crate) page_keys: (char, char),
    #[allow(dead_code)]
    log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

thread_local! {
    static GLOBAL: RefCell<Option<Global>> = const { RefCell::new(None) };
}

fn with_global<R>(f: impl FnOnce(&mut Global) -> R) -> Option<R> {
    GLOBAL.with(|slot| {
        let mut guard = slot.try_borrow_mut().ok()?;
        let global = guard.as_mut()?;
        Some(f(global))
    })
}

unsafe fn opt_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(pointer) };
    Some(text.to_string_lossy().into_owned())
}

unsafe fn opt_path(pointer: *const c_char) -> Option<PathBuf> {
    unsafe { opt_string(pointer) }.map(PathBuf::from)
}

/// 装配全局 Engine。`config_path` / `data_dir` / `share_dir` 为 NULL 时用 XDG / `/usr/share/qingjian` 缺省。
/// 返回 0 成功，非 0 失败（详情在日志里）。
#[unsafe(no_mangle)]
pub extern "C" fn qj_global_init(
    config_path: *const c_char,
    data_dir: *const c_char,
    share_dir: *const c_char,
) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        dotenvy::dotenv().ok();
        let log_guard = logging::init();
        let config_path = unsafe { opt_path(config_path) }.or_else(paths::config_file);
        let data_dir = unsafe { opt_path(data_dir) }.or_else(paths::data_dir);
        let share_dir = unsafe { opt_path(share_dir) }.unwrap_or_else(paths::share_dir);
        match engine::build(config_path.as_deref(), data_dir.as_deref(), &share_dir) {
            Ok(Assembled { engine, config }) => {
                let page_size = config.general.page_size();
                let slots = config.predict.slots;
                let page_keys = config.general.page_keys();
                GLOBAL.with(|slot| {
                    *slot.borrow_mut() = Some(Global {
                        engine,
                        page_size,
                        slots,
                        page_keys,
                        log_guard,
                    });
                });
                0
            }
            Err(error) => {
                tracing::error!(%error, "Engine 装配失败");
                1
            }
        }
    }));
    result.unwrap_or(2)
}

/// 释放全局 Engine（进程退出前调一次即可）。
#[unsafe(no_mangle)]
pub extern "C" fn qj_global_free() {
    GLOBAL.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            *guard = None;
        }
    });
}

/// 新建一个会话（每个 fcitx5 `InputContext` 一个）。失败返回 NULL。
#[unsafe(no_mangle)]
pub extern "C" fn qj_session_new() -> *mut QjSession {
    Box::into_raw(Box::new(QjSession::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_session_free(session: *mut QjSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// 记下当前应用的标识（Linux 上由 C++ 侧给，M1 接）。
#[unsafe(no_mangle)]
pub extern "C" fn qj_set_application(session: *mut QjSession, app: *const c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(_session) = (unsafe { session.as_mut() }) else {
            return;
        };
        let app = unsafe { opt_string(app) };
        with_global(|global| global.engine.set_application(app));
    }));
}

/// 处理一次按键，返回是否吃掉。`is_release` 非 0 时直接放行（只处理按下）。
#[unsafe(no_mangle)]
pub extern "C" fn qj_process_key(
    session: *mut QjSession,
    keysym: u32,
    modifiers: u32,
    is_release: c_int,
) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(session) = (unsafe { session.as_mut() }) else {
            return 0;
        };
        session.clear_commit();
        if is_release != 0 {
            return 0;
        }
        with_global(|global| keys::process(global, session, keysym, modifiers) as c_int)
            .unwrap_or(0)
    }));
    match result {
        Ok(consumed) => consumed,
        Err(_) => {
            tracing::error!("按键处理 panic，丢弃本次组句");
            with_global(|global| global.engine.clear());
            if let Some(session) = unsafe { session.as_mut() } {
                session.clear();
            }
            0
        }
    }
}

/// 点选当前页列出的第 `listed` 个候选（0 起）。
#[unsafe(no_mangle)]
pub extern "C" fn qj_select(session: *mut QjSession, listed: u32) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(session) = (unsafe { session.as_mut() }) else {
            return 0;
        };
        with_global(|global| {
            let Some(candidate) = session.candidate_listed(listed as usize) else {
                return 0;
            };
            let text = global.engine.commit(&candidate);
            session.set_commit(&text);
            session.recompose(&global.engine, global.page_size, global.slots);
            1
        })
        .unwrap_or(0)
    }));
    result.unwrap_or(0)
}

/// 输入框失焦 / 输入法停用：原样上屏缓冲、断链、落盘。
#[unsafe(no_mangle)]
pub extern "C" fn qj_focus_out(session: *mut QjSession) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(session) = (unsafe { session.as_mut() }) else {
            return;
        };
        with_global(|global| {
            let raw = global.engine.take_raw();
            if !raw.is_empty() {
                session.set_commit(&raw);
            }
            global.engine.break_chain();
            global.engine.flush_learning();
            session.recompose(&global.engine, global.page_size, global.slots);
        });
    }));
}

/// 定时器轮询：收释义兜底与本地整句重排，有更新就重查一次。返回是否有更新。
#[unsafe(no_mangle)]
pub extern "C" fn qj_poll(session: *mut QjSession) -> c_int {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Some(session) = (unsafe { session.as_mut() }) else {
            return 0;
        };
        with_global(|global| {
            let learned = global.engine.poll_glosses();
            let rescored = global.engine.poll_rescoring();
            let changed = learned > 0 || rescored;
            if changed {
                session.recompose(&global.engine, global.page_size, global.slots);
            }
            changed as c_int
        })
        .unwrap_or(0)
    }));
    result.unwrap_or(0)
}

/// 光标前文（本地整句模型用）。空串表示没有 / 不适用。
#[unsafe(no_mangle)]
pub extern "C" fn qj_set_surrounding(_session: *mut QjSession, text: *const c_char, _cursor: u32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let text = unsafe { opt_string(text) };
        with_global(|global| {
            global
                .engine
                .set_rescoring_context(text.filter(|text| !text.is_empty()));
        });
    }));
}

/// 私密输入：密码框里不学不记、不读前文、不发云端。
#[unsafe(no_mangle)]
pub extern "C" fn qj_set_private(_session: *mut QjSession, private: c_int) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        with_global(|global| global.engine.set_private(private != 0));
    }));
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_is_empty(_session: *mut QjSession) -> c_int {
    with_global(|global| global.engine.composition().is_empty() as c_int).unwrap_or(1)
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_has_commit(session: *mut QjSession) -> c_int {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.has_commit() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_commit_text(session: *mut QjSession) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.commit_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_has_preedit(session: *mut QjSession) -> c_int {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    (!session.preedit_is_empty()) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_preedit_text(session: *mut QjSession) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.preedit_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_preedit_cursor(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.preedit_cursor() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_preedit_segment_count(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.preedit_segment_count() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_preedit_segment(session: *mut QjSession, index: u32) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.preedit_segment_ptr(index as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_show_candidates(session: *mut QjSession) -> c_int {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.show_candidates() as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_candidate_count(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.candidate_count() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_candidate_text(session: *mut QjSession, index: u32) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.candidate_text_ptr(index as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_candidate_comment(session: *mut QjSession, index: u32) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.candidate_comment_ptr(index as usize)
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_candidate_is_cloud(session: *mut QjSession, index: u32) -> c_int {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.candidate_is_cloud(index as usize) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_candidate_cursor(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.cursor() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_page_index(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 0;
    };
    session.page_index() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_page_count(session: *mut QjSession) -> u32 {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return 1;
    };
    session.page_count() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn qj_aux(session: *mut QjSession) -> *const c_char {
    let Some(session) = (unsafe { session.as_ref() }) else {
        return std::ptr::null();
    };
    session.aux_ptr()
}
