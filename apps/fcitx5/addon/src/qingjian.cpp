// 青简 Fcitx5 addon 实现。逻辑全在 Rust 侧（qingjian_core::Engine），这里只做：
// 按键 → qj_process_key、结果 → InputPanel（preedit / 候选 / aux）、点选 → qj_select。

#include "qingjian.h"

#include <memory>

#include <fcitx-utils/key.h>
#include <fcitx-utils/log.h>
#include <fcitx-utils/macros.h>
#include <fcitx/inputpanel.h>
#include <fcitx/surroundingtext.h>
#include <fcitx/text.h>

extern "C" {
int qj_global_init(const char *config_path, const char *data_dir,
                   const char *share_dir);
void qj_global_free(void);
QjSession *qj_session_new(void);
void qj_session_free(QjSession *session);
void qj_set_application(QjSession *session, const char *app);
void qj_set_surrounding(QjSession *session, const char *text, uint32_t cursor);
void qj_set_private(QjSession *session, int32_t private_input);
void qj_focus_out(QjSession *session);
int32_t qj_process_key(QjSession *session, uint32_t keysym, uint32_t modifiers,
                       int32_t is_release);
int32_t qj_select(QjSession *session, uint32_t listed);
int32_t qj_has_commit(QjSession *session);
const char *qj_commit_text(QjSession *session);
void qj_clear_commit(QjSession *session);
int32_t qj_has_preedit(QjSession *session);
const char *qj_preedit_text(QjSession *session);
uint32_t qj_preedit_cursor(QjSession *session);
int32_t qj_show_candidates(QjSession *session);
uint32_t qj_candidate_count(QjSession *session);
const char *qj_candidate_text(QjSession *session, uint32_t index);
const char *qj_candidate_comment(QjSession *session, uint32_t index);
int32_t qj_candidate_is_cloud(QjSession *session, uint32_t index);
uint32_t qj_candidate_cursor(QjSession *session);
const char *qj_aux(QjSession *session);
}

namespace fcitx {

namespace {

// 与 Rust 侧 keys.rs 的位掩码一致。
constexpr uint32_t kShiftMask = 1;
constexpr uint32_t kControlMask = 4;
constexpr uint32_t kAltMask = 8;
constexpr uint32_t kSuperMask = 64;

std::string textOr(const char *text) { return text ? std::string(text) : std::string(); }

} // namespace

// --- QingjianCandidateWord ---

QingjianCandidateWord::QingjianCandidateWord(QingjianState *state, Text text,
                                             int index,
                                             const std::string &comment)
    : CandidateWord(std::move(text)), state_(state), index_(index) {
#ifdef FCITX5_HAS_CANDIDATE_SET_COMMENT
    if (!comment.empty()) {
        setComment(Text(comment));
    }
#else
    // fcitx5 < 5.1.9 没有右侧注释，把注释拼进候选文本。
    if (!comment.empty()) {
        Text merged = this->text();
        merged.append(" " + comment);
        setText(std::move(merged));
    }
#endif
}

void QingjianCandidateWord::select(InputContext *inputContext) const {
    FCITX_UNUSED(inputContext);
    if (state_) {
        state_->selectCandidate(index_);
    }
}

// --- QingjianState ---

QingjianState::QingjianState(QingjianEngine *engine, InputContext *inputContext)
    : engine_(engine), inputContext_(inputContext) {
    session_ = qj_session_new();
}

QingjianState::~QingjianState() {
    if (session_) {
        qj_session_free(session_);
    }
}

void QingjianState::keyEvent(KeyEvent &keyEvent) {
    if (!session_) {
        return;
    }
    const Key &key = keyEvent.key();
    const KeyStates states = key.states();
    uint32_t modifiers = 0;
    if (states.test(KeyState::Shift)) {
        modifiers |= kShiftMask;
    }
    if (states.test(KeyState::Ctrl)) {
        modifiers |= kControlMask;
    }
    if (states.test(KeyState::Alt)) {
        modifiers |= kAltMask;
    }
    if (states.test(KeyState::Super)) {
        modifiers |= kSuperMask;
    }
    const int32_t isRelease = keyEvent.isRelease() ? 1 : 0;
    if (qj_process_key(session_, static_cast<uint32_t>(key.sym()), modifiers,
                       isRelease)) {
        keyEvent.filterAndAccept();
    }
    updateUI();
}

void QingjianState::activate() {
    if (!session_) {
        return;
    }
    const std::string &program = inputContext_->program();
    qj_set_application(session_, program.empty() ? nullptr : program.c_str());

    if (inputContext_->capabilityFlags().test(CapabilityFlag::SurroundingText) &&
        inputContext_->surroundingText().isValid()) {
        const auto &surrounding = inputContext_->surroundingText();
        qj_set_surrounding(session_, surrounding.text().c_str(),
                           static_cast<uint32_t>(surrounding.cursor()));
    } else {
        qj_set_surrounding(session_, "", 0);
    }

    const int32_t isPrivate =
        inputContext_->capabilityFlags().test(CapabilityFlag::PasswordOrSensitive)
            ? 1
            : 0;
    qj_set_private(session_, isPrivate);
    updateUI();
}

void QingjianState::deactivate() {
    if (!session_) {
        return;
    }
    // 原样交出未上屏的拼音，断链并落盘（对齐 mac deactivateServer / Windows Commit）。
    qj_focus_out(session_);
    updateUI();
    inputContext_->inputPanel().reset();
    inputContext_->updatePreedit();
    inputContext_->updateUserInterface(UserInterfaceComponent::InputPanel);
}

void QingjianState::reset() { deactivate(); }

void QingjianState::selectCandidate(int index) {
    if (!session_) {
        return;
    }
    if (qj_select(session_, static_cast<uint32_t>(index))) {
        updateUI();
    }
}

void QingjianState::updateUI() {
    if (!session_) {
        return;
    }
    InputPanel &panel = inputContext_->inputPanel();

    if (qj_has_commit(session_)) {
        const std::string committed = textOr(qj_commit_text(session_));
        // 上屏是一次性的：立刻取走并清除，否则后面的 activate / deactivate / reset（Shift 切上下文等）
        // 会把同一段文字再 commit 一次。
        qj_clear_commit(session_);
        if (!committed.empty()) {
            inputContext_->commitString(committed);
        }
    }

    Text preedit;
    if (qj_has_preedit(session_)) {
        const std::string text = textOr(qj_preedit_text(session_));
        if (!text.empty()) {
            preedit.append(text, TextFormatFlag::Underline);
            preedit.setCursor(static_cast<int>(qj_preedit_cursor(session_)));
        }
    }
    if (inputContext_->capabilityFlags().test(CapabilityFlag::Preedit)) {
        panel.setClientPreedit(preedit);
    } else {
        panel.setPreedit(preedit);
    }

    const std::string aux = textOr(qj_aux(session_));
    panel.setAuxUp(aux.empty() ? Text() : Text(aux));

    if (qj_show_candidates(session_)) {
        auto list = std::make_unique<CommonCandidateList>();
        list->setLayoutHint(CandidateLayoutHint::Vertical);
        // 行标签只用来显示；数字键由 Rust 侧消费（与 karukan 同做法）。
        list->setSelectionKey(Key::keyListFromString("1 2 3 4 5 6 7 8 9"));
        const uint32_t count = qj_candidate_count(session_);
        list->setPageSize(static_cast<int>(count > 0 ? count : 1));
        for (uint32_t i = 0; i < count; ++i) {
            std::string label = textOr(qj_candidate_text(session_, i));
            if (qj_candidate_is_cloud(session_, i)) {
                label = "☁ " + label;
            }
            Text candidate;
            candidate.append(label);
            list->append<QingjianCandidateWord>(
                this, std::move(candidate), static_cast<int>(i),
                textOr(qj_candidate_comment(session_, i)));
        }
        list->setGlobalCursorIndex(
            static_cast<int>(qj_candidate_cursor(session_)));
        panel.setCandidateList(std::move(list));
    } else {
        panel.setCandidateList(nullptr);
    }

    inputContext_->updatePreedit();
    inputContext_->updateUserInterface(UserInterfaceComponent::InputPanel);
}

// --- QingjianEngine ---

QingjianEngine::QingjianEngine(Instance *instance)
    : instance_(instance),
      factory_([this](InputContext &ic) {
          return new QingjianState(this, &ic);
      }) {
    if (qj_global_init(nullptr, nullptr, nullptr) != 0) {
        FCITX_ERROR() << "Qingjian: 输入引擎装配失败，青简暂时不可用（详情见日志）";
    }
    instance_->inputContextManager().registerProperty("qingjianState",
                                                      &factory_);
}

QingjianEngine::~QingjianEngine() { qj_global_free(); }

void QingjianEngine::keyEvent(const InputMethodEntry &entry, KeyEvent &event) {
    FCITX_UNUSED(entry);
    event.inputContext()->propertyFor(&factory_)->keyEvent(event);
}

void QingjianEngine::activate(const InputMethodEntry &entry,
                              InputContextEvent &event) {
    FCITX_UNUSED(entry);
    event.inputContext()->propertyFor(&factory_)->activate();
}

void QingjianEngine::deactivate(const InputMethodEntry &entry,
                                InputContextEvent &event) {
    FCITX_UNUSED(entry);
    event.inputContext()->propertyFor(&factory_)->deactivate();
}

void QingjianEngine::reset(const InputMethodEntry &entry,
                           InputContextEvent &event) {
    FCITX_UNUSED(entry);
    event.inputContext()->propertyFor(&factory_)->reset();
}

} // namespace fcitx

FCITX_ADDON_FACTORY(fcitx::QingjianEngineFactory);