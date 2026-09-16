//! C ABI 冒烟测试：不依赖 fcitx5，直接调 `extern "C"` 函数，验证「装载 Engine → 敲拼音出候选 → 上屏」。
//!
//! 数据用仓库自带的 `assets/`（词库 / 释义表 / emoji），所以这两条测试在 Ubuntu 上 `cargo test` 就能跑。

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use qingjian_fcitx5::{
    qj_candidate_count, qj_candidate_text, qj_clear_commit, qj_commit_text, qj_focus_out,
    qj_global_free, qj_global_init, qj_has_commit, qj_has_preedit, qj_is_empty, qj_process_key,
    qj_select, qj_session_free, qj_session_new,
};

/// 仓库自带的产品数据目录（`assets/`），`dict.tsv` / `glossary-en.tsv` / emoji 都在里面。
fn repo_assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

struct Harness {
    dir: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("qingjian-fcitx5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let share = repo_assets();
        assert!(
            share.join("lexicon/dict.tsv").is_file(),
            "需要仓库自带词库 assets/lexicon/dict.tsv"
        );
        let config = CString::new(dir.join("config.toml").to_str().unwrap()).unwrap();
        let data = CString::new(dir.to_str().unwrap()).unwrap();
        let share = CString::new(share.to_str().unwrap()).unwrap();
        assert_eq!(
            qj_global_init(config.as_ptr(), data.as_ptr(), share.as_ptr()),
            0,
            "Engine 装配应成功"
        );
        Self { dir }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        qj_global_free();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn press(session: *mut qingjian_fcitx5::QjSession, keys: &str) {
    for ch in keys.chars() {
        assert_eq!(
            qj_process_key(session, ch as u32, 0, 0),
            1,
            "字符 {ch:?} 应被吃掉"
        );
    }
}

#[test]
fn types_nihao_and_commits_first_candidate() {
    let _harness = Harness::new();
    let session = qj_session_new();
    assert!(!session.is_null());

    press(session, "nihao");
    assert_eq!(qj_is_empty(session), 0, "组句中缓冲区不空");
    assert_eq!(qj_has_preedit(session), 1, "应有 preedit");
    assert!(qj_candidate_count(session) >= 1, "应有候选");

    let first = unsafe { CStr::from_ptr(qj_candidate_text(session, 0)) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(first, "你好");

    assert_eq!(qj_select(session, 0), 1);
    assert_eq!(qj_has_commit(session), 1);
    let committed = unsafe { CStr::from_ptr(qj_commit_text(session)) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(committed, "你好");
    assert_eq!(qj_is_empty(session), 1, "上屏后缓冲区清空");

    qj_session_free(session);
}

#[test]
fn commit_is_single_shot() {
    let _harness = Harness::new();
    let session = qj_session_new();

    press(session, "nihao");
    assert_eq!(qj_select(session, 0), 1);
    assert_eq!(qj_has_commit(session), 1);
    qj_clear_commit(session);
    assert_eq!(qj_has_commit(session), 0, "取走后不应再有上屏文本");

    // 失焦 / 重置（Shift 切上下文会触发）不允许把上一次的上屏文本再交一次
    qj_focus_out(session);
    assert_eq!(qj_has_commit(session), 0);

    qj_session_free(session);
}

#[test]
fn backspace_and_escape_clear_composition() {
    let _harness = Harness::new();
    let session = qj_session_new();

    press(session, "ni");
    assert_eq!(qj_is_empty(session), 0);
    assert_eq!(qj_process_key(session, 0xff08, 0, 0), 1, "退格被吃掉");
    assert_eq!(qj_process_key(session, 0xff08, 0, 0), 1);
    assert_eq!(qj_is_empty(session), 1, "退格删空后缓冲区为空");

    press(session, "nihao");
    assert_eq!(qj_process_key(session, 0xff1b, 0, 0), 1, "Esc 被吃掉");
    assert_eq!(qj_is_empty(session), 1, "Esc 放弃组句");
    assert_eq!(qj_has_commit(session), 0, "Esc 不上屏任何文本");

    qj_session_free(session);
}
