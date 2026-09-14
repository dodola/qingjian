//! Linux 路径与数据文件定位：XDG 约定，详见 `docs/design/linux-fcitx5.md`「配置与数据路径」。

use std::path::{Path, PathBuf};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `$XDG_*_HOME`，没设或为空就按 XDG 缺省落到 home 下。
fn xdg(env: &str, fallback: &str) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(env)
        && !value.is_empty()
    {
        return Some(PathBuf::from(value));
    }
    home().map(|home| home.join(fallback))
}

pub fn config_dir() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config").map(|dir| dir.join("qingjian"))
}

pub fn data_dir() -> Option<PathBuf> {
    xdg("XDG_DATA_HOME", ".local/share").map(|dir| dir.join("qingjian"))
}

pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("config.toml"))
}

/// 随包数据目录：打包装到 `/usr/share/qingjian`，开发时可被 `QINGJIAN_SHARE_DIR` 覆盖。
pub fn share_dir() -> PathBuf {
    std::env::var_os("QINGJIAN_SHARE_DIR")
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from("/usr/share/qingjian"), PathBuf::from)
}

/// 在多个目录里找一个数据文件，同名的 `.qj` 优先于 TSV（与 CLI 一致）。`name` 带扩展名。
pub fn find_data_file(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let stem = Path::new(name).with_extension("");
    let packed = stem.with_extension("qj");
    for dir in dirs {
        for candidate in [dir.join(&packed), dir.join(name)] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 数据文件搜索目录：随包目录（含 `lexicon` / `glossary` / `emoji` 子目录，兼容开发期的仓库布局）
/// 与用户数据目录。
pub fn data_search_dirs(share_dir: &Path, user_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![
        share_dir.to_path_buf(),
        share_dir.join("lexicon"),
        share_dir.join("glossary"),
        share_dir.join("emoji"),
    ];
    if let Some(user_dir) = user_dir {
        dirs.push(user_dir.to_path_buf());
    }
    dirs
}
