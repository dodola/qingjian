---
title: 在 Linux 上
order: 4
description: Linux（Fcitx5）预览版的系统要求、从源码安装、当前可用的按键、尚未实现的功能与已知限制。
---

青简的 Linux 版通过 Fcitx5 输入法框架接入，目前为预览版：只提供从源码构建的方式，还没有安装包，功能也少于 macOS 与 Windows 版。

## 系统要求

- 64 位 Linux，Ubuntu 22.04、Debian 12 或同级发行版；已在 Ubuntu 22.04 与 Fcitx5 5.0.14 上验证编译与装载。
- 已安装且正在使用的 Fcitx5 5.0 或更新。
- 构建需要 Rust 工具链、CMake、g++ 与 Fcitx5 开发包。

## 从源码安装

先安装构建依赖：

```sh
sudo apt install fcitx5 fcitx5-modules-dev libfcitx5core-dev libfcitx5config-dev \
    libfcitx5utils-dev extra-cmake-modules cmake make g++ pkg-config
```

在源码目录构建并安装到系统：

```sh
cd apps/fcitx5/addon
cmake -B build -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build -j
sudo cmake --install build
fcitx5 -r
```

没有管理员权限时，可改用下面的步骤装到当前账户：

```sh
cd apps/fcitx5/addon
cmake -B build -DCMAKE_INSTALL_PREFIX=$HOME/.local
cmake --build build -j
cmake --install build
```

再新建 `~/.config/environment.d/fcitx5-qingjian.conf`，写入以下内容后重新登录（两个 `FCITX_DATA_*` 的路径需指向 `fcitx5` 目录本身）：

```sh
FCITX_ADDON_DIRS=$HOME/.local/lib/fcitx5:/usr/lib/x86_64-linux-gnu/fcitx5
FCITX_DATA_HOME=$HOME/.local/share/fcitx5
FCITX_DATA_DIRS=$HOME/.local/share/fcitx5:/usr/share/fcitx5
```

最后在 Fcitx5 配置（`fcitx5-configtool`）里把「青简」加入输入法列表。

## 词库数据

预览版不自带词库，需自行提供：缺省从 `/usr/share/qingjian/` 读取，可用环境变量 `QINGJIAN_SHARE_DIR` 指向其他目录，例如仓库中的 `assets/`。

## 现在可以用什么

拼音组句、整句输入、简拼、模糊音与双拼（按配置）、候选与翻页、上屏、学习，以及数据中有释义表时候选旁的译词。

| 操作 | Linux |
|---|---|
| 上屏高亮候选（无候选时上屏拼音） | `Space` |
| 上屏当前页第 N 个 | `1` 至 `9` |
| 将拼音原样上屏 | `Enter` |
| 删除光标前 / 后一个字母 | `Backspace` / `Delete` |
| 清空 | `Esc` |
| 翻页 | `Tab`、`PageUp` / `PageDown`、`[` `]` |
| 移动高亮 | `↑` / `↓` |
| 输入标点 | 中文标点自动转为全角 |

`Ctrl` 与 `Super` 的组合键交给应用处理。

## 尚未实现

- 没有设置界面：修改配置需手动编辑配置文件后重启 Fcitx5，位置见 [数据与日志](../help/data-and-logs.md)。
- 配置热加载、按应用设置、私密输入。
- 英文模式、`Shift` 输入大写、光标左右移动、问字、表达式与快捷候选。
- 云联想与「翻译选中文字」：没有界面开关。
- 候选窗口由 Fcitx5 的主题绘制，外观与 macOS / Windows 版不同。

## 卸载

见 [卸载](../help/uninstall.md)。数据与配置文件的位置见 [数据与日志](../help/data-and-logs.md)。