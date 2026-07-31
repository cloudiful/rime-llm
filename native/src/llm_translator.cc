#include "llm_translator.h"

#include "llm_session.h"

#include <rime/candidate.h>
#include <rime/schema.h>
#include <rime/segmentation.h>
#include <rime/ticket.h>
#include <rime/translation.h>

namespace rime_llm {

LlmTranslator::LlmTranslator(const rime::Ticket& ticket,
                             std::shared_ptr<SessionState> state)
    : rime::Translator(ticket), state_(std::move(state)) {}

rime::an<rime::Translation> LlmTranslator::Query(
    const rime::string& input, const rime::Segment& segment) {
  if (!state_)
    return nullptr;
  auto translation = rime::New<rime::FifoTranslation>();
  if (segment.HasTag("llm_prediction")) {
    if (!state_->Visible())
      return nullptr;
    const auto candidates = state_->Candidates();
    for (size_t i = 0; i < candidates.size(); ++i) {
      auto candidate = rime::New<rime::SimpleCandidate>(
          "llm_prediction", segment.start, segment.end, candidates[i].text);
      candidate->set_quality(1000.0 - static_cast<double>(i));
      translation->Append(candidate);
    }
  } else {
    const auto candidates = state_->CandidatesForInput(input);
    for (const auto& item : candidates) {
      if (auto candidate = state_->CandidateForModel(
              item, segment.start, segment.end)) {
        translation->Append(candidate);
      }
    }
  }
  return translation->size() == 0 ? nullptr : translation;
}

LlmTranslator* LlmTranslatorComponent::Create(const rime::Ticket& ticket) {
  return new LlmTranslator(ticket, SessionState::Find(ticket.engine));
}

}  // namespace rime_llm
