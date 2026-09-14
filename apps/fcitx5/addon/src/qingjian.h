// 青简 Fcitx5 addon 的 C++ 薄壳：只碰 fcitx5 API，输入逻辑全在 Rust cdylib（C ABI）。
// 设计见 docs/design/linux-fcitx5.md。

#ifndef QINGJIAN_FCITX5_ADDON_H
#define QINGJIAN_FCITX5_ADDON_H

#include <cstdint>
#include <string>

#include <fcitx/addonfactory.h>
#include <fcitx/addonmanager.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputcontextproperty.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/instance.h>

// Rust 侧的不透明会话句柄（C ABI，见 apps/fcitx5/src/lib.rs）。
struct QjSession;

namespace fcitx {

class QingjianEngine;
class QingjianState;

/// 一个候选：文本 + 注释（词性 + 译文），点选时回调会话状态。
class QingjianCandidateWord : public CandidateWord {
public:
    QingjianCandidateWord(QingjianState *state, Text text, int index,
                          const std::string &comment);

    void select(InputContext *inputContext) const override;

private:
    QingjianState *state_;
    int index_;
};

/// 每个 InputContext 一份会话状态（缓冲区、候选页、preedit）。
class QingjianState : public InputContextProperty {
public:
    QingjianState(QingjianEngine *engine, InputContext *inputContext);
    ~QingjianState() override;

    void keyEvent(KeyEvent &keyEvent);
    void activate();
    void deactivate();
    void reset();
    void selectCandidate(int index);

private:
    void updateUI();

    QingjianEngine *engine_;
    InputContext *inputContext_;
    QjSession *session_{nullptr};
};

/// addon 本体：把 fcitx5 回调转发给各 InputContext 的 QingjianState。
class QingjianEngine : public InputMethodEngineV2 {
public:
    explicit QingjianEngine(Instance *instance);
    ~QingjianEngine() override;

    void keyEvent(const InputMethodEntry &entry, KeyEvent &keyEvent) override;
    void activate(const InputMethodEntry &entry, InputContextEvent &event) override;
    void deactivate(const InputMethodEntry &entry, InputContextEvent &event) override;
    void reset(const InputMethodEntry &entry, InputContextEvent &event) override;

private:
    Instance *instance_;
    FactoryFor<QingjianState> factory_;
};

/// fcitx5 装载 addon 时创建 QingjianEngine。
class QingjianEngineFactory : public AddonFactory {
public:
    AddonInstance *create(AddonManager *manager) override {
        return new QingjianEngine(manager->instance());
    }
};

} // namespace fcitx

#endif // QINGJIAN_FCITX5_ADDON_H