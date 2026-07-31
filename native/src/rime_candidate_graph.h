#ifndef RIME_LLM_CANDIDATE_GRAPH_H_
#define RIME_LLM_CANDIDATE_GRAPH_H_

#include "llm_json.h"

#include <rime/common.h>
#include <rime/dict/vocabulary.h>

#include <memory>
#include <string>
#include <vector>

namespace rime {
class Dictionary;
class Engine;
class Language;
class UserDictionary;
}

namespace rime_llm {

struct RimeCandidateComponent {
  rime::DictEntry entry;
  size_t start = 0;
  size_t end = 0;
};

struct RimeCandidatePath {
  CandidatePath request;
  std::vector<RimeCandidateComponent> components;
};

class RimeCandidateGraph {
 public:
  explicit RimeCandidateGraph(rime::Engine* engine);
  ~RimeCandidateGraph();

  bool ready() const;
  std::vector<RimeCandidatePath> Build(const std::string& input,
                                       size_t max_paths,
                                       size_t max_homophones) const;
  const rime::Language* language() const { return language_.get(); }

 private:
  rime::Engine* engine_ = nullptr;
  std::unique_ptr<rime::Dictionary> dictionary_;
  std::unique_ptr<rime::UserDictionary> user_dictionary_;
  std::unique_ptr<rime::Language> language_;
};

}  // namespace rime_llm

#endif
