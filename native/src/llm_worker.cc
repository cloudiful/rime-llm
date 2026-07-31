#include "llm_worker.h"

#include "http_client.h"

#include <algorithm>

namespace rime_llm {

LlmWorker::LlmWorker(WorkerConfig config) : config_(std::move(config)) {
  config_.max_candidates = std::max<size_t>(1, std::min<size_t>(16, config_.max_candidates));
  config_.max_tokens = std::max<size_t>(1, std::min<size_t>(32, config_.max_tokens));
  config_.timeout_ms = std::max(100, std::min(60000, config_.timeout_ms));
}

LlmWorker::~LlmWorker() {
  Stop();
}

void LlmWorker::SetResultCallback(ResultCallback callback) {
  std::lock_guard<std::mutex> lock(mutex_);
  result_callback_ = std::move(callback);
}

void LlmWorker::SetCandidateCallback(CandidateCallback callback) {
  std::lock_guard<std::mutex> lock(mutex_);
  candidate_callback_ = std::move(callback);
}

void LlmWorker::Start() {
  std::lock_guard<std::mutex> lock(mutex_);
  if (started_)
    return;
  started_ = true;
  thread_ = std::thread(&LlmWorker::Run, this);
}

void LlmWorker::Stop() {
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!started_ || stopped_)
      return;
    stopped_ = true;
    commits_.clear();
    pending_.reset();
  }
  wakeup_.notify_all();
  if (thread_.joinable())
    thread_.join();
}

void LlmWorker::SubmitCommit(const std::string& text) {
  if (text.empty())
    return;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (stopped_)
      return;
    commits_.push_back(text);
  }
  wakeup_.notify_one();
}

void LlmWorker::SubmitCandidates(uint64_t request_id, const std::string& input) {
  if (input.empty())
    return;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (stopped_)
      return;
    pending_ = PendingRequest{
        PendingKind::kCandidates,
        request_id,
        ++serial_,
        input,
        std::chrono::steady_clock::now(),
    };
  }
  wakeup_.notify_one();
}

void LlmWorker::SubmitPrediction(uint64_t request_id, int delay_ms) {
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (stopped_)
      return;
    pending_ = PendingRequest{
        PendingKind::kPrediction,
        request_id,
        ++serial_,
        {},
        std::chrono::steady_clock::now() + std::chrono::milliseconds(std::max(0, delay_ms)),
    };
  }
  wakeup_.notify_one();
}

void LlmWorker::Invalidate(uint64_t request_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  if (pending_ && pending_->request_id <= request_id)
    pending_.reset();
}

std::string LlmWorker::Endpoint(const char* path) const {
  if (config_.endpoint.empty())
    return path;
  if (config_.endpoint.back() == '/')
    return config_.endpoint.substr(0, config_.endpoint.size() - 1) + path;
  return config_.endpoint + path;
}

void LlmWorker::Run() {
  while (true) {
    std::unique_lock<std::mutex> lock(mutex_);
    if (stopped_)
      return;
    if (!reset_done_) {
      reset_done_ = true;
      lock.unlock();
      ResetService();
      continue;
    }
    if (!commits_.empty()) {
      std::string text = std::move(commits_.front());
      commits_.pop_front();
      lock.unlock();
      Commit(text);
      continue;
    }
    if (pending_) {
      const auto serial = pending_->serial;
      if (std::chrono::steady_clock::now() < pending_->due) {
        wakeup_.wait_until(lock, pending_->due, [this, serial] {
          return stopped_ || !pending_ || pending_->serial != serial || !commits_.empty();
        });
        continue;
      }
      PendingRequest request = *pending_;
      pending_.reset();
      lock.unlock();
      if (request.kind == PendingKind::kCandidates)
        Candidates(std::move(request));
      else
        Predict(std::move(request));
      continue;
    }
    wakeup_.wait(lock, [this] { return stopped_ || !commits_.empty() || pending_; });
  }
}

void LlmWorker::ResetService() {
  HttpResponse response;
  std::string error;
  if (!PostJson(Endpoint("/reset"), BuildResetRequest(config_.session_id),
                config_.timeout_ms, &response, &error)) {
    return;
  }
  uint64_t revision = 0;
  if (ParseRevisionResponse(response.body, &revision)) {
    std::lock_guard<std::mutex> lock(mutex_);
    server_revision_ = revision;
  }
}

void LlmWorker::Commit(const std::string& text) {
  HttpResponse response;
  std::string error;
  if (!PostJson(Endpoint("/commit"), BuildCommitRequest(config_.session_id, text),
                config_.timeout_ms, &response, &error)) {
    return;
  }
  uint64_t revision = 0;
  if (ParseRevisionResponse(response.body, &revision)) {
    std::lock_guard<std::mutex> lock(mutex_);
    server_revision_ = revision;
  }
}

void LlmWorker::Candidates(PendingRequest request) {
  HttpResponse response;
  std::string error;
  if (!PostJson(Endpoint("/candidates"),
                BuildCandidatesRequest(config_.session_id, request.input,
                                       config_.max_candidates),
                config_.timeout_ms, &response, &error)) {
    return;
  }
  CandidatesResponse parsed;
  if (!ParseCandidatesResponse(response.body, &parsed))
    return;
  CandidateResult result;
  result.request_id = request.request_id;
  result.input = std::move(request.input);
  result.candidates = std::move(parsed.candidates);
  CandidateCallback callback;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    callback = candidate_callback_;
  }
  if (callback)
    callback(std::move(result));
}

void LlmWorker::Predict(PendingRequest request) {
  uint64_t revision = 0;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    revision = server_revision_;
  }
  HttpResponse response;
  std::string error;
  if (!PostJson(Endpoint("/predict"),
                BuildPredictionRequest(config_.session_id, revision, config_.mode,
                                       config_.max_candidates, config_.max_tokens),
                config_.timeout_ms, &response, &error)) {
    return;
  }
  PredictionResponse parsed;
  if (!ParsePredictionResponse(response.body, &parsed))
    return;
  PredictionResult result;
  result.request_id = request.request_id;
  result.revision = parsed.revision;
  result.candidates = std::move(parsed.candidates);
  ResultCallback callback;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    callback = result_callback_;
  }
  if (callback)
    callback(std::move(result));
}

}  // namespace rime_llm
