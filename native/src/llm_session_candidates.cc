#include "llm_session.h"

#include <rime/context.h>

#include <utility>

namespace rime_llm {
namespace {

bool IsPinyinInput(const std::string& input) {
  if (input.empty())
    return false;
  for (unsigned char character : input) {
    if (!((character >= 'a' && character <= 'z') ||
          (character >= 'A' && character <= 'Z')))
      return false;
  }
  return true;
}

}  // namespace

void SessionState::OnContextUpdate(rime::Context* context) {
  if (stopped_ || context != context_ || context->input().empty() ||
      (!model_candidates_.empty() && context->input() == model_input_))
    return;
  HideForInput();
}

std::vector<ModelCandidate> SessionState::CandidatesForInput(
    const std::string& input) {
  if (!config_.candidates_enabled || !IsPinyinInput(input))
    return {};
  if (input != model_input_) {
    model_input_ = input;
    model_candidates_ = worker_.FetchCandidates(input);
  }
  return model_candidates_;
}

bool SessionState::ShouldReplaceCandidates() const {
  return config_.replace_candidates && context_ &&
         context_->input() == model_input_ && !model_candidates_.empty();
}

}  // namespace rime_llm
