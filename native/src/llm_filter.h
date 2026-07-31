#ifndef RIME_LLM_FILTER_H_
#define RIME_LLM_FILTER_H_

#include <rime/filter.h>

#include <memory>

namespace rime_llm {
class SessionState;

class LlmCandidateFilter : public rime::Filter {
 public:
  explicit LlmCandidateFilter(const rime::Ticket& ticket);

  rime::an<rime::Translation> Apply(
      rime::an<rime::Translation> translation,
      rime::CandidateList* candidates) override;
  bool AppliesToSegment(rime::Segment* segment) override;

 private:
  std::shared_ptr<SessionState> state_;
};

class LlmCandidateFilterComponent : public LlmCandidateFilter::Component {
 public:
  LlmCandidateFilter* Create(const rime::Ticket& ticket) override;
};

}  // namespace rime_llm

#endif
