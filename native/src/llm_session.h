#ifndef RIME_LLM_SESSION_H_
#define RIME_LLM_SESSION_H_

#include "llm_json.h"
#include "llm_worker.h"

#include <rime/common.h>

#include <memory>
#include <mutex>

namespace rime {
class Context;
class Engine;
class KeyEvent;
struct Ticket;
}

namespace rime_llm {

struct LlmConfig {
  bool enabled = true;
  bool candidates_enabled = true;
  bool replace_candidates = true;
  std::string endpoint = "http://127.0.0.1:32123";
  std::string mode = "free";
  std::string trigger;
  int idle_delay_ms = 200;
  size_t max_candidates = 5;
  size_t candidate_max_candidates = 16;
  int candidate_timeout_ms = 1500;
  size_t max_tokens = 8;
  int timeout_ms = 15000;
};

LlmConfig ReadConfig(const rime::Ticket& ticket);

class SessionState : public std::enable_shared_from_this<SessionState> {
 public:
  SessionState(rime::Engine* engine, LlmConfig config, std::string session_id);
  ~SessionState();

  void Start();
  void OnCommit(rime::Context* context);
  void OnContextUpdate(rime::Context* context);
  bool MatchesTrigger(const rime::KeyEvent& key) const;
  bool Visible() const { return visible_; }
  bool HasCachedCandidates() const { return !candidates_.empty(); }
  bool ShowCached();
  void Cancel();
  bool Accept(size_t index);
  void InsertSpace();
  void HideForInput();
  std::vector<PredictionCandidate> Candidates() const { return candidates_; }
  std::vector<ModelCandidate> CandidatesForInput(const std::string& input);
  bool ShouldReplaceCandidates() const;
  void ApplyResult(PredictionResult result);

  static std::shared_ptr<SessionState> Acquire(rime::Engine* engine,
                                                const LlmConfig& config,
                                                const std::string& session_id);
  static std::shared_ptr<SessionState> Find(rime::Engine* engine);
  static void Remove(rime::Engine* engine, SessionState* state);

 private:
  void PostResult(PredictionResult result);
  void Invalidate();
  void InstallPredictionSegment();
  void ClearPredictionContext();
  bool IsPredictionContext() const;

  rime::Engine* engine_;
  rime::Context* context_;
  LlmConfig config_;
  std::string session_id_;
  rime_llm::LlmWorker worker_;
  std::vector<PredictionCandidate> candidates_;
  std::vector<ModelCandidate> model_candidates_;
  std::string model_input_;
  uint64_t request_id_ = 0;
  bool visible_ = false;
  bool started_ = false;
  bool stopped_ = false;
  rime::connection commit_connection_;
  rime::connection update_connection_;
};

}  // namespace rime_llm

#endif
