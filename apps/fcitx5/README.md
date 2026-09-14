# qingjian-fcitx5

青简的 Linux 壳（Fcitx5）。`qingjian-fcitx5`（Rust cdylib，持 `qingjian-core::Engine`）+ `addon/`（C++ 薄壳，只碰 fcitx5 API），
之间是同进程 C ABI；候选窗用 fcitx5 原生、不自绘。设计见 [`docs/design/linux-fcitx5.md`](../../docs/design/linux-fcitx5.md)。

## 构建与安装

依赖（Ubuntu / Debian）：

```bash
sudo apt install fcitx5 fcitx5-modules-dev libfcitx5core-dev libfcitx5config-dev \
    libfcitx5utils-dev extra-cmake-modules cmake make g++ pkg-config
```

```bash
cd apps/fcitx5/addon
cmake -B build -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build -j          # 内部会 cargo build --release -p qingjian-fcitx5
sudo cmake --install build
fcitx5 -r
```

装到用户本地（无 sudo）时要把本地与系统的目录都告诉 fcitx5，写进 `~/.config/environment.d/`
（fcitx5 在登录会话里启动，shell profile 不生效），再重新登录：

```bash
cd apps/fcitx5/addon
cmake -B build -DCMAKE_INSTALL_PREFIX=$HOME/.local
cmake --build build -j
cmake --install build
SYSTEM_FCITX5_DIR=$(pkg-config --variable=libdir Fcitx5Core)/fcitx5
mkdir -p ~/.config/environment.d
cat > ~/.config/environment.d/fcitx5-qingjian.conf <<EOF
FCITX_ADDON_DIRS=$HOME/.local/lib/fcitx5:$SYSTEM_FCITX5_DIR
FCITX_DATA_HOME=$HOME/.local/share/fcitx5
FCITX_DATA_DIRS=$HOME/.local/share/fcitx5:/usr/share/fcitx5
EOF
```

坑（Ubuntu 22.04 / fcitx5 5.0.14 实测）：`FCITX_DATA_HOME` / `FCITX_DATA_DIRS` 要指向 **`.../fcitx5` 目录本身**
（不是 XDG 的 `.../share` 根）；只设 `FCITX_ADDON_DIRS` 会让 fcitx5 找不到 `addon/` 与 `inputmethod/` 里的 conf。
装到 `/usr`（系统安装）不需要这些，还用不到 `FCITX_DATA_*`。

然后在 `fcitx5-configtool` 里把「青简」加到输入法列表。日志确认：

```bash
fcitx5 -r -d    # 应看到 Loaded addon qingjian
```

## 数据与路径

- 配置：`~/.config/qingjian/config.toml`（首次运行写模板）
- 用户数据（学习 / 释义 / 输入日志）：`~/.local/share/qingjian/`
- 随包数据（`dict.qj` / `lm.qj` / 释义表 / 英文词表 / emoji / 本地模型）：`/usr/share/qingjian/`
  （开发时设 `QINGJIAN_SHARE_DIR` 指向仓库 `data/generated/` 或 `assets/`）
- 日志：默认 stderr；设 `QINGJIAN_LOG_DIR` 按天写文件。`RUST_LOG` 控制级别

## 测试

Rust 侧的 C ABI / 会话 / 按键路由不依赖 fcitx5，本机就能测：

```bash
cargo test -p qingjian-fcitx5
```

C++ addon 与真机行为要在带 fcitx5 的桌面上验（见 `docs/design/linux-fcitx5.md` 的 M1–M5）。