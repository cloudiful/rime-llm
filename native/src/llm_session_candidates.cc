#include "llm_session.h"

#include <rime/candidate.h>
#include <rime/context.h>
#include <rime/gear/translator_commons.h>

#include <algorithm>
#include <unordered_set>
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
    candidate_paths_ = candidate_graph_.Build(
        input, config_.candidate_max_paths, config_.candidate_max_homophones);
    std::vector<CandidatePath> request_paths;
    request_paths.reserve(candidate_paths_.size());
    for (const auto& path : candidate_paths_)
      request_paths.push_back(path.request);

    model_candidates_.clear();
    if (!request_paths.empty())
      model_candidates_ = worker_.FetchCandidates(input, request_paths);

    std::unordered_set<std::string> seen_ids;
    std::vector<ModelCandidate> valid_candidates;
    valid_candidates.reserve(model_candidates_.size());
    for (const auto& candidate : model_candidates_) {
      if (candidate.consumedkeys != input.size() || candidate.text.empty() ||
          !seen_ids.insert(candidate.id).second) {
        continue;
      }
      const auto path = std::find_if(
          candidate_paths_.begin(), candidate_paths_.end(),
          [&candidate](const RimeCandidatePath& item) {
            return item.request.id == candidate.id &&
                   item.request.text == candidate.text &&
                   item.request.consumedkeys == candidate.consumedkeys;
          });
      if (path != candidate_paths_.end())
        valid_candidates.push_back(candidate);
    }
    model_candidates_ = std::move(valid_candidates);
  }
  return model_candidates_;
}

rime::an<rime::Candidate> SessionState::CandidateForModel(
    const ModelCandidate& model_candidate,
    int segment_start,
    int segment_end) const {
  const auto path = std::find_if(
      candidate_paths_.begin(), candidate_paths_.end(),
      [&model_candidate](const RimeCandidatePath& item) {
        return item.request.id == model_candidate.id &&
               item.request.text == model_candidate.text &&
               item.request.consumedkeys == model_candidate.consumedkeys;
      });
  if (path == candidate_paths_.end() || !candidate_graph_.language() ||
      segment_start < 0 || segment_end < segment_start ||
      model_candidate.consumedkeys == 0 ||
      model_candidate.consumedkeys >
          static_cast<size_t>(segment_end - segment_start)) {
    return nullptr;
  }

  auto sentence = rime::New<rime::Sentence>(candidate_graph_.language());
  size_t previous_end = 0;
  for (const auto& component : path->components) {
    if (component.start != previous_end || component.end <= component.start)
      return nullptr;
    sentence->Extend(component.entry, component.end, component.entry.weight);
    previous_end = component.end;
  }
  if (sentence->empty() || previous_end != model_candidate.consumedkeys ||
      sentence->text() != model_candidate.text) {
    return nullptr;
  }

  sentence->set_type("llm_candidate");
  sentence->set_start(static_cast<size_t>(segment_start));
  sentence->set_end(static_cast<size_t>(segment_start) +
                    model_candidate.consumedkeys);
  sentence->set_preedit(model_candidate.preedit.empty()
                            ? path->request.preedit
                            : model_candidate.preedit);
  sentence->set_quality(1000.0 + model_candidate.score);
  return sentence;
}

bool SessionState::ShouldReplaceCandidates() const {
  return config_.replace_candidates && context_ &&
         context_->input() == model_input_ && !model_candidates_.empty();
}

}  // namespace rime_llm
