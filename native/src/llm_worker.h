#ifndef RIME_LLM_WORKER_H_
#define RIME_LLM_WORKER_H_

#include "llm_json.h"

#include <chrono>
#include <condition_variable>
#include <deque>
#include <functional>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace rime_llm {

struct WorkerConfig {
  std::string endpoint = "http://127.0.0.1:32123";
  std::string session_id;
  std::string mode = "free";
  size_t max_candidates = 5;
  size_t candidate_max_candidates = 16;
  int candidate_timeout_ms = 1500;
  size_t max_tokens = 8;
  int timeout_ms = 15000;
};

struct PredictionResult {
  uint64_t request_id = 0;
  uint64_t revision = 0;
  std::vector<PredictionCandidate> candidates;
};

class LlmWorker {
 public:
  using ResultCallback = std::function<void(PredictionResult)>;

  explicit LlmWorker(WorkerConfig config);
  ~LlmWorker();

  void SetResultCallback(ResultCallback callback);
  void Start();
  void Stop();
  void SubmitCommit(const std::string& text);
  std::vector<ModelCandidate> FetchCandidates(const std::string& input) const;
  void SubmitPrediction(uint64_t request_id, int delay_ms);
  void Invalidate(uint64_t request_id);

 private:
  struct PendingRequest {
    uint64_t request_id = 0;
    uint64_t serial = 0;
    std::chrono::steady_clock::time_point due;
  };

  void Run();
  void ResetService();
  void Commit(const std::string& text);
  void Predict(PendingRequest request);
  std::string Endpoint(const char* path) const;

  WorkerConfig config_;
  ResultCallback result_callback_;
  std::thread thread_;
  std::mutex mutex_;
  std::condition_variable wakeup_;
  bool stopped_ = false;
  bool started_ = false;
  bool reset_done_ = false;
  uint64_t server_revision_ = 0;
  uint64_t serial_ = 0;
  std::deque<std::string> commits_;
  std::optional<PendingRequest> pending_;
};

}  // namespace rime_llm

#endif
