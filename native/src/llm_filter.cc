#include "llm_filter.h"

#include "llm_session.h"

#include <rime/candidate.h>
#include <rime/segmentation.h>
#include <rime/translation.h>

namespace rime_llm {
namespace {

bool IsModelCandidate(const rime::an<rime::Candidate>& candidate) {
  if (!candidate)
    return false;
  const auto genuine = rime::Candidate::GetGenuineCandidate(candidate);
  return genuine && genuine->type() == "llm_candidate";
}

class ModelOnlyTranslation : public rime::Translation {
 public:
  explicit ModelOnlyTranslation(rime::an<rime::Translation> translation)
      : translation_(std::move(translation)) {
    LocateNextCandidate();
  }

  bool Next() override {
    if (exhausted())
      return false;
    if (!translation_ || !translation_->Next()) {
      set_exhausted(true);
      return false;
    }
    return LocateNextCandidate();
  }

  rime::an<rime::Candidate> Peek() override {
    return exhausted() || !translation_ ? nullptr : translation_->Peek();
  }

 private:
  bool LocateNextCandidate() {
    while (translation_ && !translation_->exhausted()) {
      if (IsModelCandidate(translation_->Peek())) {
        set_exhausted(false);
        return true;
      }
      if (!translation_->Next())
        break;
    }
    set_exhausted(true);
    return false;
  }

  rime::an<rime::Translation> translation_;
};

}  // namespace

LlmCandidateFilter::LlmCandidateFilter(const rime::Ticket& ticket)
    : Filter(ticket), state_(SessionState::Find(ticket.engine)) {}

rime::an<rime::Translation> LlmCandidateFilter::Apply(
    rime::an<rime::Translation> translation,
    rime::CandidateList* /*candidates*/) {
  const bool replace = state_ && state_->ShouldReplaceCandidates();
  if (!replace)
    return translation;
  return rime::New<ModelOnlyTranslation>(std::move(translation));
}

bool LlmCandidateFilter::AppliesToSegment(rime::Segment* segment) {
  return !segment || !segment->HasTag("llm_prediction");
}

LlmCandidateFilter* LlmCandidateFilterComponent::Create(
    const rime::Ticket& ticket) {
  return new LlmCandidateFilter(ticket);
}

}  // namespace rime_llm
