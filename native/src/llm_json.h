#ifndef RIME_LLM_JSON_H_
#define RIME_LLM_JSON_H_

#include <cstdint>
#include <string>
#include <vector>

namespace rime_llm {

struct PredictionCandidate {
  std::string id;
  std::string text;
  double score = 0;
  std::string type;
};

struct PredictionResponse {
  uint64_t revision = 0;
  std::vector<PredictionCandidate> candidates;
};

struct ModelCandidate {
  std::string id;
  std::string text;
  std::string preedit;
  uint64_t consumedkeys = 0;
  double score = 0;
  std::string type;
};

struct CandidatesResponse {
  std::vector<ModelCandidate> candidates;
};

std::string JsonEscape(const std::string& value);
std::string BuildResetRequest(const std::string& session_id);
std::string BuildCommitRequest(const std::string& session_id,
                               const std::string& text);
std::string BuildCandidatesRequest(const std::string& session_id,
                                   const std::string& input,
                                   size_t max_candidates);
std::string BuildPredictionRequest(const std::string& session_id,
                                   uint64_t revision,
                                   const std::string& mode,
                                   size_t max_candidates,
                                   size_t max_tokens);

bool ParseRevisionResponse(const std::string& body, uint64_t* revision);
bool ParseCandidatesResponse(const std::string& body,
                             CandidatesResponse* response);
bool ParsePredictionResponse(const std::string& body,
                             PredictionResponse* response);
bool IsChineseText(const std::string& text);

}  // namespace rime_llm

#endif
