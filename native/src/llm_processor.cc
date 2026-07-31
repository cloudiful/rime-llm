#include "llm_processor.h"

#include "llm_session.h"

#include <X11/keysym.h>

#include <rime/engine.h>
#include <rime/key_event.h>
#include <rime/schema.h>
#include <rime/ticket.h>

namespace rime_llm {

LlmProcessor::LlmProcessor(const rime::Ticket& ticket,
                           std::shared_ptr<SessionState> state)
    : rime::Processor(ticket), state_(std::move(state)) {}

rime::ProcessResult LlmProcessor::ProcessKeyEvent(const rime::KeyEvent& key) {
  if (!state_)
    return rime::kNoop;
  const bool plain = key.modifier() == 0;
  if (!state_->Visible()) {
    if (plain && state_->HasCachedCandidates() && state_->MatchesTrigger(key))
      return state_->ShowCached() ? rime::kAccepted : rime::kNoop;
    return rime::kNoop;
  }
  if (plain && (key.keycode() == XK_Tab || key.keycode() == XK_Return ||
                key.keycode() == XK_KP_Enter)) {
    return state_->Accept(0) ? rime::kAccepted : rime::kNoop;
  }
  if (plain && key.keycode() >= '1' && key.keycode() <= '9') {
    return state_->Accept(static_cast<size_t>(key.keycode() - '1'))
               ? rime::kAccepted
               : rime::kNoop;
  }
  if (plain && key.keycode() == XK_space) {
    state_->InsertSpace();
    return rime::kAccepted;
  }
  if (plain && key.keycode() == XK_Escape) {
    state_->Cancel();
    return rime::kAccepted;
  }
  if (key.keycode() >= 0x20 && key.keycode() <= 0x7e) {
    state_->HideForInput();
  } else if (plain && key.keycode() == XK_BackSpace) {
    state_->HideForInput();
  }
  return rime::kNoop;
}

LlmProcessor* LlmProcessorComponent::Create(const rime::Ticket& ticket) {
  const auto config = ReadConfig(ticket);
  std::string schema_id = ticket.schema ? ticket.schema->schema_id() : "default";
  const auto session_id = "rime-llm:" + schema_id + ":" +
                          std::to_string(reinterpret_cast<uintptr_t>(ticket.engine));
  auto state = SessionState::Acquire(ticket.engine, config, session_id);
  return new LlmProcessor(ticket, std::move(state));
}

}  // namespace rime_llm
