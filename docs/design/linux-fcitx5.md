# Linux：Fcitx5

Phase 5 的 Linux 壳，先做 Fcitx5，IBus 之后另做。Core 一行不改，平台层只做两件事：
把 fcitx5 的按键翻成 Engine 的输入、把 Engine 的结果画进 fcitx5 的候选窗（见 [architecture.md](architecture.md) 「总体结构」与架构约束）。

现状：**M0 骨架已落地**（`apps/fcitx5`，C ABI + Engine 装配 + 会话分页，测试见 `apps/fcitx5/tests/c_abi.rs`）；
**M1 已在 fcitx5 5.0.14（Ubuntu 22.04）上编译并验证 load**：addon `OnDemand=False` 强制加载时出现 `Loaded addon qingjian`，
Rust 侧 `Engine 就绪`（词库 92821 条，约 215 ms）。输入法条目注册 / 端到端打字待带桌面会话的 `fcitx5-configtool` 验证
（`OnDemand=True` 是 IM addon 的正常设置，只有输入框激活时才加载，`--enable` 不触发）。本文件是落地计划，代码写起来之后以代码为准并回改这里。

参考实现（同类、Rust 引擎 + fcitx5 前端，已各自发布）：

- [togatoga/karukan](https://github.com/togatoga/karukan)：Rust crate `karukan-fcitx5`（`crate-type = ["cdylib"]`）经 C FFI 被一个 C++ addon（`fcitx5/fcitx5-addon/`，CMake）调用。
- [BonoJovi/Bonolith](https://github.com/BonoJovi/Bonolith)：同一形态，`src/ffi.rs` + `fcitx5/bonolith_engine.cpp`。

两者都证明：**Rust 引擎 + C++ 薄壳 + C FFI 是当前 fcitx5 接入 Rust 的可行路径**。

## 结论：C++ 薄壳 + Rust cdylib（C ABI）

fcitx5 的 addon 是运行时由 fcitx5 装载的 **C++ 模块**，注册入口是 `FCITX_ADDON_FACTORY`，没有稳定的纯 Rust 接口
（crates.io 上的 `fcitx5-dbus` 是 D-Bus 客户端，不是 addon；不存在成型的 `fcitx5-rs` addon binding）。
所以壳分两层：

```text
fcitx5 主进程
├── qingjian.so            C++ addon：只碰 fcitx5 API（InputMethodEngineV2 / InputContext /
│                          InputPanel / CandidateList / KeyEvent / commitString / preedit）
└── libqingjian_fcitx5.so  Rust cdylib：持 qingjian-core::Engine，实现平台无关的按键路由与分页，
                           C ABI 暴露给上面
```

与 Windows 结构同构，只是没有 IPC：

| | Windows | Linux / Fcitx5 |
|---|---|---|
| 只转发的一层 | `qingjian_tsf.dll` | `qingjian.so`（C++） |
| 持 Engine 的一层 | `qingjian-server.exe`（独立进程） | `libqingjian_fcitx5.so`（同进程 cdylib） |
| 之间 | 命名管道 + `qingjian-platform::protocol` | 同进程 C ABI |

为什么 Windows 要拆进程而 Linux 不用：TSF DLL 会被加载进每一个应用进程、Engine 会被复制几十份，所以 Engine 必须出进程；
fcitx5 是一个常驻进程服务所有输入框，cdylib 只加载一次，同进程即可（也更简单，省掉协议与异步编辑会话）。

**为什么不让 C++ 直接调 Core（rlib / cxx）**：Core 是 Rust crate，跨语言边界用 cdylib + 手写 C ABI 最省事，
也不把 `cxx` / `bindgen` 这类代码生成工具引进来。C++ 侧只做转发，逻辑全在 Rust，和 karukan 一致。

**备选（不选）**：走 fcitx5 的 D-Bus 控制接口（`org.fcitx.Fcitx5`）做纯 Rust 客户端，像 IBus 那样。
但拿 InputContext、控制候选窗、拿 surrounding text 都不如 addon 直接，且第三方 D-Bus 控制面不是稳定公开接口。
IBus 阶段再用 `zbus` 走这条路。

## 目录与 crate

放进 `apps/`，与 mac / windows 并列。Rust 是 workspace 成员，C++ 不进 cargo：

```text
apps/fcitx5/
├── Cargo.toml              # package qingjian-fcitx5，[lib] crate-type = ["cdylib"]
├── src/
│   ├── lib.rs              # C ABI（extern "C"），薄，只做类型转换与借用
│   ├── engine.rs           # 全局 Engine 装配（单例）：词库 / LM / 翻译 / 学习 / 预测 / 神经
│   ├── session.rs          # 每个 InputContext 的 UI 状态：分页、高亮、preedit（移植 mac host/session.rs）
│   ├── keys.rs             # 平台无关按键路由（移植 mac imk/controller.rs 的 dispatch 逻辑）
│   ├── paths.rs            # Linux 路径约定（见下「配置与数据」）
│   └── logging.rs          # tracing 初始化与日志目录
└── addon/                  # C++，CMake 构建
    ├── CMakeLists.txt
    ├── src/qingjian.cpp
    ├── src/qingjian.h
    ├── qingjian-addon.conf.in      # addon 描述（装到 addondir）
    ├── qingjian.conf               # input method 描述（装到 inputmethod dir）
    ├── icons/fcitx-qingjian-{16,24,32,48,128}.png
    └── org.fcitx.Fcitx5.Addon.Qingjian.metainfo.xml.in
```

`Cargo.toml` 的依赖与 `apps/cli` 相同那棵 Core 依赖树，外加 `libc`（keysym 常量、`c_char`）与 `tracing*`。
不依赖任何 Windows / macOS 专属 crate，Linux 上直接能编（CLI 已实测整棵树在 Ubuntu 可编）。

## 进程与线程模型

- fcitx5 是一个**单线程主循环**进程，addon 回调（`keyEvent` / `activate` / `deactivate` / 候选 `select`）都在主线程。所有 C ABI 调用限定从主线程发起，FFI 头注释写死。
- 因此 Rust 侧用 `thread_local!` + `RefCell` 持全局 Engine（与 mac `host::with` 同一理由：避开 `Send` 约束，candle 也没要求 `Send`）；跨线程只经 Core 自己的后台线程 + 通道。
- **一个全局 Engine**（词库、语言模型、释义表、学习各一份，加载贵）。**每个 fcitx5 `InputContext` 一份 Session**（分页 / 高亮 / preedit 显示态）。
  fcitx5 会给每个应用窗口 / 输入框建 `InputContext`，切换时通知 `activate` / `deactivate`。
- 焦点切换（`deactivate`）：`commit_raw` 交出未上屏拼音、`break_chain`、`flush_learning`——对齐 mac `deactivateServer:`
  （`apps/macos/src/imk/controller.rs:118`）与 Windows 的 `Commit`（`apps/windows/server/src/dispatch/message.rs:45`）。
- 进程退出（fcitx5 关闭）时 `flush_learning` 一次；学习数据本身走 `write_atomic`，崩了最多丢一分钟（见 architecture.md「崩溃不丢」）。

## Engine 门面：壳要调什么

壳只碰 `Engine` 的公开方法（`crates/qingjian-core/src/engine/`），C ABI 是它们的一层薄包装。
按键路由要用的全集（现有 mac 壳用到的那套）：

| 类别 | 方法 | 位置 |
|---|---|---|
| 喂字 | `push` / `backspace` / `set_input` / `take_raw` | `composing.rs:143/153/284/292` |
| 光标编辑 | `delete_forward` / `delete_syllable_backward` / `delete_to_start` / `move_cursor_left|right|home|end` / `move_cursor_syllable_left|right` | `composing.rs:168…228` |
| 模式判定 | `expression_mode` / `raw_mode` / `question_mode` / `unicode_entry` / `bare_question` / `takes_semicolon` | `composing.rs:230…265`、`setup.rs:60` |
| 标点与直通 | `punctuate` / `note_passthrough` | `composing.rs:10/29` |
| 查询与标注 | `query()` / `annotate(&mut CandidateList)` | `query/mod.rs:19`、`commit/mod.rs:16` |
| 上屏 | `commit(&Candidate)` / `commit_translation(&Candidate, sense)` | `commit/mod.rs:63/51` |
| 学习生命周期 | `break_chain` / `note_backspace` / `note_page_turn` / `flush_learning` / `note_displayed` / `forget` | `composing.rs:52/110/74`、`learning/mod.rs:168/36/142` |
| 应用与隐私 | `set_application` / `set_private` | `composing.rs:65`、`privacy.rs:9` |
| 英文模式 | `set_english_mode` / `english_mode` / `restore_bare_question` | `setup.rs:234/238`、`composing.rs:271` |
| 联想 / 重排 | `prediction_enabled` / `prediction_policy` / `request_prediction` / `accept_prediction` / `poll_glosses` / `request_rescoring` / `poll_rescoring` / `set_rescoring_context` | `engine/prediction/`、`engine/learning/mod.rs:16`、`engine/rescoring/mod.rs:40/98/113` |
| 装配 | `Engine::new(dict).with_translator(..).with_learner(..).with_predictor(..).with_sentence_scorer(..)`、`set_extra_dictionaries` | `setup.rs` |

结果类型：`Query { segmentations, candidates, tail, text, cursor, rest, timings, correction, decoded_keys, typed_display }`
（`query/result.rs:25`），`Candidate { text, kind, syllables, reading, translation }`（`candidate/mod.rs:28`），
分页由 Core 的 `CandidateLayout`（`candidate/layout/candidate_layout.rs`）定，壳只管高亮与页码。

## 按键路由：移植 mac 的分发

mac 的 `dispatch_event`（`apps/macos/src/imk/controller.rs:176`–`556`）与 Windows 的 `Router`（`apps/windows/server/src/dispatch/`）
是同一个平台无关逻辑的两份实现，Linux 是第三份。规则（Fcitx5 版）：

- fcitx5 `KeyEvent` 有 release；**只处理 press**，release 直接放行。修饰键从 `keyEvent.key().states()` 取 Shift / Ctrl / Alt / Super，映射成 Core 认识的 `KeyModifiers` 位。
- 组句中的字母、`'`、双拼的 `;`、表达式字符、`-`（进直输段）、半角标点（除翻页键）→ `push`。
- `Backspace` / `Delete` / `Ctrl+Backspace` / `Alt+Backspace` / 左右上下 / Home / End / PageUp / PageDown / Enter / Esc / Tab → 对应的编辑或翻页动作。
- 数字 1–9 选当前页第几个候选；修饰键（配置 `[shortcut]`）+ 数字 → 上屏第 n 个译词 / 删候选。
- Shift 大写字母、Caps（英文模式）按 mac 的规则走：英文模式下数字 / `_` / `'` / `-` 进缓冲区，空格回车先原样上屏再交回应用。
- 其它字符按 `punctuate` 转全角，转不了就 `note_passthrough` 交回应用。

fcitx5 与 IMK 的差异：

- 候选选择：fcitx5 的 `CandidateList` 自带 selection key。像 karukan 一样，`setSelectionKey("1 2 ... 9")` 只当**行标签**，
  数字键由 Rust 侧先消费（`qj_select`），避免 fcitx5 抢在 Engine 之前处理。鼠标点候选走 `CandidateWord::select` → `qj_select`。
- 「先上屏再把这个键交给应用」：fcitx5 用 `keyEvent.filterAndAccept()` 表示吃掉、不 accept 表示继续给应用，语义比 TSF 简单，按 mac 的写法即可。
- `process_key` 返回值 `consumed`：Rust 侧一次调用内部完成「按键 → Engine → 更新会话状态」，C++ 据此 accept 并刷新 UI。

**待定（技术债）**：这段路由已在 mac / win / linux 三处重复。建议之后抽一个 `qingjian-input`（纯「键 → Engine 调用 → UI 想画什么」的状态机，
不碰任何平台 API，`keysym` 由各壳归一化后喂进来），三个壳共用。本次先原样移植，不顺手重构。

## UI 映射

C++ 侧把 Rust 暴露的一帧状态翻译成 fcitx5 的 `InputPanel`：

| 青简概念 | fcitx5 |
|---|---|
| preedit（`MarkedSegment`：`Typed` 下划线 + `Rest` 淡色） | `inputPanel.setClientPreedit`（有 `CapabilityFlag::Preedit`）否则 `setPreedit`；整段 `Underline`，`setCursor(marked_cursor())` |
| 候选列表（`CandidateLayout` 当前页） | 自定义 `CandidateList`：`setLayoutHint(Horizontal / Vertical)`（配置外观）、`setPageSize`、`setSelectionKey`；`CandidateWord::select` → `qj_select` |
| 译文 / 词性（`Candidate.translation`） | `CandidateWord::setComment`（fcitx5 ≥ 5.1.9）显示在候选右侧；老版本把注释拼进 `text`（karukan 的 `FCITX5_HAS_CANDIDATE_SET_COMMENT` 写法） |
| 云端词 ☁ | 候选文本前缀，或 comment |
| 高亮 | `setGlobalCursorIndex(qj_candidate_cursor())` |
| 整句补全 / 删候选提示（`notice`） | `setAuxUp` / `setAuxDown` |
| 日语假名注音 | 现状：不做（等有日语输入模式） |

**候选窗不用青简自绘**。mac 是自绘 NSPanel、Windows 是 Server 自绘分层窗；Linux 上**候选窗交给 fcitx5 与当前主题**
（Wayland 下第三方自绘候选窗的定位 / 层级 / 焦点很难做对，也不符合桌面观感）。代价是候选窗样式与 mac / win 不一致——接受。
`qingjian-render`（自绘渲染器 spike）在 Linux 壳里暂不使用。

**中 / 英模式**：mac 用 Caps Lock，Windows 用单击 Shift，都是青简自己的模式。fcitx5 有原生的「输入法启用 / 停用」。
两个方案：

1. 注册两个 `InputMethodEntry`（`qingjian` 中文、`qingjian` + english 变体），addon 按 entry 名调 `set_english_mode`；
   用户用 fcitx5 的中英切换键在两个 entry 间切。
2. entry 只有一个，青简内部保留自己的 Caps / Shift 切换（与 mac / win 手感一致），fcitx5 的启用 / 停用只管整个输入法。
   **倾向方案 2**（行为与其他平台一致），具体待真机试用后定。

## 配置与数据路径（Linux 约定，新决定）

Shell 负责路径，Core 与 `qingjian-platform` 不硬编码平台路径（现状：mac 的路径在 `apps/macos/src/app/paths.rs`）。
Linux 按 XDG：

| 内容 | 路径 | 说明 |
|---|---|---|
| 配置 | `$XDG_CONFIG_HOME/qingjian/config.toml`（默认 `~/.config/qingjian/`） | `qingjian-platform::Config::load/set_value`，模板与非 Linux 共用 |
| 学习数据 / 释义 / 输入日志 / 统计 / 词汇 | `$XDG_DATA_HOME/qingjian/`（默认 `~/.local/share/qingjian/`） | 学习 crate 的六张 TSV + `user-glossary-<语言>.tsv` + `input-log.jsonl` + `usage.tsv` + `user-vocab.tsv` |
| 用户导入的词库 | `<数据目录>/dicts/` | `qingjian-platform::extra_dictionaries::load(..)` 直接用 |
| 随包数据（`.qj` 词库 / LM / 释义 / 英文词表 / emoji / 本地模型 `.qjm`） | `/usr/share/qingjian/`（打包安装）；开发时回退仓库 `data/generated/`、`assets/` | 打包脚本决定 |
| 日志 | `$XDG_STATE_HOME/qingjian/logs/`（默认 `~/.local/state/qingjian/logs/`）；`QINGJIAN_LOG_DIR` 可覆盖 | tracing-appender 按天滚，留 7 天 |
| 密钥 | `<配置目录>/.env` 或 `config.toml` 的 `api_key` | `dotenvy` 读入，同 mac |

`extra_dictionaries::load(bundled_dir, user_dir, &config.dictionaries)`（`crates/qingjian-platform/src/extra_dictionaries.rs:36`）
签名已平台无关，bundled = `/usr/share/qingjian/dicts`，user = 数据目录 `dicts/`，领域词库 / 导入 / 开关逻辑零改动。

配置热加载：mac 靠每秒看 `config.toml` 的 mtime（`host/config_watch.rs`）。Linux 同样起一个 mtime 轮询（fcitx5 event loop 定时器），
变更后走同一份 `apply_config`（推给 Engine：模糊音、双拼、模式键、Predictor 重建、释义表切换、附加词库重装）。**只有 `config.toml` 一条通路**，不各存一套状态。

## 异步：云联想、释义兜底、本地整句重排

Core 全是 `submit` / `poll` 的非阻塞接口，壳要用定时器收结果。fcitx5 没有回调式定时器给 addon，但有事件循环：

```cpp
instance_->eventLoop().addTimeEvent(/* 时钟 */, /* 间隔 */, ...);
```

对齐 mac 的节拍（`host/predict_monitor.rs`、`host/rescore_monitor.rs`、`host/model.rs`）：

- 停键约 80 ms 请求 `request_rescoring`，之后约 20 ms `poll_rescoring`，到了重查一次、只重画当前页（翻过页 / 动过高亮不动）。
- 云联想：`request_prediction` 后轮询 `poll_prediction`；释义兜底每秒 `poll_glosses`；结果到了 `updateUserInterface(InputPanel)`。
- 本地整句模型：启动后台线程加载并预热（candle Linux 走 **CPU**，无 Metal / Accelerate；加载与每键耗时要真机测），
  `set_async_sentence_scorer` 接上；配置 `[model] enabled` 开关。
- 前文：`ic->surroundingText()`（需要 `CapabilityFlag::SurroundingText`）；Linux 上很多应用不支持（见 csslayer 的
  [Why surrounding text is the worst feature](https://www.csslayer.info/wordpress/fcitx-dev/why-surrounding-text-is-the-worst-feature-in-the-linux-input-method-world/)），
  取不到就清空、只用本地。**私密输入**：fcitx5 有 `CapabilityFlag::Password`，命中时 `set_private(true)`、不读前文、不发云端（对齐 mac Secure Input / Windows `IS_PRIVATE`）。

## C ABI

手写、`extern "C"`、`#[unsafe(no_mangle)]`，不透明会话指针 + getter。字符串一律 UTF-8，指针在**下一次调用前有效**，C++ 立即拷贝。
所有函数限定主线程。Engine 是进程内单例（thread_local），不是句柄；会话用 `QjSession*`。

```c
// 全局 Engine（进程内一份，重）。返回 0 成功
int  qj_global_init(const char* config_path, const char* data_dir, const char* share_dir);
void qj_global_free(void);

// 每个 InputContext 一份会话
QjSession* qj_session_new(void);
void       qj_session_free(QjSession*);
void       qj_set_application(QjSession*, const char* app);
void       qj_set_surrounding(QjSession*, const char* text, uint32_t cursor);
void       qj_set_private(QjSession*, int private);
void       qj_focus_out(QjSession*);              // deactivate：原样上屏 + break_chain + flush + 重查
int        qj_poll(QjSession*);                   // 定时器：收释义兜底 / 本地整句重排，返回是否有更新

// 输入
int  qj_process_key(QjSession*, uint32_t keysym, uint32_t modifiers, int is_release);
int  qj_select(QjSession*, uint32_t listed);      // 当前页列出的第几个候选（0 起）

// 结果读取（process_key / qj_select / qj_poll / qj_focus_out 之后）
int         qj_is_empty(QjSession*);              // 组句缓冲区是否为空
int         qj_has_commit(QjSession*);     const char* qj_commit_text(QjSession*);
void        qj_clear_commit(QjSession*);            // 取走后清除，上屏是一次性的（否则 Shift 切上下文会重复上屏）
int         qj_has_preedit(QjSession*);    const char* qj_preedit_text(QjSession*);
uint32_t    qj_preedit_cursor(QjSession*);
uint32_t    qj_preedit_segment_count(QjSession*);  const char* qj_preedit_segment(QjSession*, uint32_t);
int         qj_show_candidates(QjSession*);
uint32_t    qj_candidate_count(QjSession*);        // 当前页列出的候选（跳过空格）
const char* qj_candidate_text(QjSession*, uint32_t);
const char* qj_candidate_comment(QjSession*, uint32_t);   // 词性 + 译文，可为空
int         qj_candidate_is_cloud(QjSession*, uint32_t);
uint32_t    qj_candidate_cursor(QjSession*);       // 高亮在列出候选里的下标
uint32_t    qj_page_index(QjSession*);             uint32_t qj_page_count(QjSession*);
const char* qj_aux(QjSession*);                    // 整句补全 / 删候选提示（暂未接）
```

`qj_select` / `qj_candidate_*` 的 `index` 是「当前页列出的第几个」，与页偏移不同：`CandidateKind::Custom`
造成的空格被跳过（C++ 只需按 getter 顺序建 `CandidateList`，点选回传同一个下标）。分页 / 高亮逻辑在
`apps/fcitx5/src/session.rs`（移植自 mac `host/session.rs`，含分页单测的思路）。

M0 已实现的按键（`apps/fcitx5/src/keys.rs`）：字母 / `'`、退格、Delete、回车、空格、数字选词、方向、Tab /
翻页、PageUp/PageDown、Esc、标点转全角。英文直输段、问字、表达式、修饰键快捷键、英文模式留 M2。

## 打包与安装

照 karukan 的 CMake：

- `addon/CMakeLists.txt`：`find_package(Fcitx5Core 5.0 REQUIRED)`、`ECM`、`xkbcommon`；
  一个 custom target 调 `cargo build --release -p qingjian-fcitx5`，addon `target_link_libraries(... ${QINGJIAN_RUST_LIB})`，
  `PREFIX ""`（fcitx5 要 `qingjian.so` 不是 `libqingjian.so`），`INSTALL_RPATH "\$ORIGIN"`，把 cdylib 与 addon 装进 `FCITX_INSTALL_ADDONDIR` 同一目录。
- `qingjian.conf`（inputmethod，含 `Name` / `Icon` / `Library=qingjian`）注册输入法，用户在 `fcitx5-configtool` 里添加。
- 依赖（Ubuntu）：`fcitx5 fcitx5-modules-dev libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev extra-cmake-modules cmake g++ pkg-config libxkbcommon-dev`。
- 本地开发安装到 `~/.local` 时要在 `~/.config/environment.d/` 设 `FCITX_ADDON_DIRS`（本地路径 + 系统路径都要给，否则 fcitx5 找不到自带 addon）——karukan 的坑，照抄。实测（Ubuntu 22.04 / fcitx5 5.0.14）还要设 `FCITX_DATA_HOME` 与 `FCITX_DATA_DIRS`，且都指向 **`.../fcitx5` 目录本身**（不是 XDG 的 `.../share` 根），否则 `addon/*.conf` 与 `inputmethod/*.conf` 不被发现；装到 `/usr` 不需要。
- 发行：`.deb`（`cargo-deb` 管不了 C++ 侧，用 `nfpm` 或手写 debian/）、AUR、tar。CI 在 ubuntu runner 构建，跑到「`fcitx5 -r -d` 日志出现 `Loaded addon qingjian`」为止。
- **验证**：CI / 本机只能编译 + C ABI 单测（像 Windows 只交叉 `check`）；真机验证在带 fcitx5 的 Linux 桌面，
  在终端 / 浏览器 / 编辑器里逐键试，surrounding text、Wayland 候选窗定位、密码框逐个记录（对齐 todo 里 mac 那批「特殊应用逐个验证」）。

## 分阶段（tracer bullets）

| 阶段 | 内容 | 验收 |
|---|---|---|
| M0 骨架（已完成 2026-09-15） | workspace 加 `apps/fcitx5`，C ABI + Engine 装配 + Session + 基础按键路由；C ABI 测试 harness（Rust 测试直接调 `extern "C"`） | `cargo test -p qingjian-fcitx5` 过（2 条）；`qj_process_key` 能出候选、`qj_select` 能上屏 |
| M1 最小可用（已完成 2026-09-15：编译 + load 验证；端到端待桌面） | C++ addon（`addon/`：CMake + `qingjian.cpp/.h` + addon / inputmethod conf + metainfo + 图标）：按键转发、preedit、候选列表、commit、点选、失焦上屏；Rust 导出符号与 C++ 声明逐一对上 | fcitx5 日志 `Loaded addon qingjian` + `Engine 就绪`（已验）；任意输入框能打「你好」（待 `fcitx5-configtool` 加输入法后真机验） |
| M2 手感 | 翻页 / 高亮 / 光标移动、标点、英文直输段、英文模式、快捷候选（日期 / 算式）、emoji | 与 CLI 交互模式行为一致 |
| M3 持久化与配置 | 学习落盘、配置热加载、按应用设置（`[apps]`）、私密输入 | 重启 fcitx5 后学习数据还在；密码框不学不联网 |
| M4 特色 | 译文 comment、云联想、释义兜底、本地整句重排（CPU） | 译词随候选显示；停顿后整句重排 |
| M5 发布 | CMake 打包、图标、metainfo、CI、用户文档 | `.deb` 装机可用；CI 在 runner 上 load 成功 |

## 风险与未决

- **按键路由三份重复**：见上「待定」，建议后续抽 `qingjian-input`。
- **中 / 英模式映射**：两个 entry vs 青简自管，待真机定（倾向自管）。
- **fcitx5 版本差异**：候选 comment 要 5.1.9+；老版本走拼接兜底。
- **候选窗不自绘**：样式与 mac / win 不一致；`qingjian-render` 不在 Linux 用。
- **surrounding text 支持差**：联想与整句前文会时有时无。
- **candle CPU 性能**：Linux 无 Metal / Accelerate，本地整句重排的每键耗时未知，超了就缩前文或默认关。
- **IPC 缺席的取舍**：Engine 与 fcitx5 同进程，Engine 崩了会连累 fcitx5——用 `catch_unwind` 在 FFI 边界拦 panic（对齐 mac `imk::catch_panic`），
  拦下后清状态、当前键交回应用。
- **许可**：本项目 GPL-3.0-or-later，fcitx5 是 LGPL-2.1+，兼容。