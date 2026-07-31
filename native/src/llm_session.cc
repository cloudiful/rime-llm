#include "llm_session.h"

#include <dispatch/dispatch.h>
#include <X11/keysym.h>

#include <rime/config.h>
#include <rime/context.h>
#include <rime/engine.h>
#include <rime/key_event.h>
#include <rime/schema.h>
#include <rime/segmentation.h>
#include <rime/ticket.h>

#include <algorithm>
#include <unordered_map>

namespace rime_llm {
namespace {

std::mutex registry_mutex;
std::unordered_map<rime::Engine*, std::weak_ptr<SessionState>> registry;

template <typename T>
void ReadInt(rime::Config* config, const char* key, T* value) {
  int parsed = 0;
  if (config && config->GetInt(key, &parsed))
    *value = static_cast<T>(parsed);
}

struct Completion {
  std::weak_ptr<SessionState> state;
  PredictionResult result;
};

void ApplyCompletion(void* raw) {
  std::unique_ptr<Completion> completion(static_cast<Completion*>(raw));
  if (auto state = completion->state.lock())
    state->ApplyResult(std::move(completion->result));
}

}  // namespace

LlmConfig ReadConfig(const rime::Ticket& ticket) {
  LlmConfig result;
  auto* config = ticket.schema ? ticket.schema->config() : nullptr;
  if (!config)
    return result;
  config->GetBool("prediction/enabled", &result.enabled);
  config->GetBool("llm_candidate_translator/enabled", &result.candidates_enabled);
  config->GetBool("llm_candidate_translator/replace_candidates",
                  &result.replace_candidates);
  config->GetString("prediction/endpoint", &result.endpoint);
  config->GetString("prediction/mode", &result.mode);
  config->GetString("prediction/trigger", &result.trigger);
  ReadInt(config, "prediction/idle_delay_ms", &result.idle_delay_ms);
  ReadInt(config, "prediction/max_candidates", &result.max_candidates);
  ReadInt(config, "llm_candidate_translator/max_candidates",
          &result.candidate_max_candidates);
  ReadInt(config, "llm_candidate_translator/max_paths",
          &result.candidate_max_paths);
  ReadInt(config, "llm_candidate_translator/max_homophones",
          &result.candidate_max_homophones);
  ReadInt(config, "llm_candidate_translator/max_wait_ms",
          &result.candidate_timeout_ms);
  ReadInt(config, "prediction/max_tokens", &result.max_tokens);
  ReadInt(config, "prediction/timeout_ms", &result.timeout_ms);
  result.idle_delay_ms = std::max(0, std::min(60000, result.idle_delay_ms));
  result.max_candidates = std::max<size_t>(1, std::min<size_t>(16, result.max_candidates));
  result.candidate_max_candidates =
      std::max<size_t>(1, std::min<size_t>(16, result.candidate_max_candidates));
  result.candidate_max_paths =
      std::max<size_t>(1, std::min<size_t>(512, result.candidate_max_paths));
  result.candidate_max_homophones =
      std::max<size_t>(1, std::min<size_t>(32, result.candidate_max_homophones));
  result.candidate_timeout_ms =
      std::max(100, std::min(1500, result.candidate_timeout_ms));
  result.max_tokens = std::max<size_t>(1, std::min<size_t>(32, result.max_tokens));
  result.timeout_ms = std::max(100, std::min(60000, result.timeout_ms));
  if (result.mode != "dictionary" && result.mode != "hybrid")
    result.mode = "free";
  return result;
}

SessionState::SessionState(rime::Engine* engine,
                           LlmConfig config,
                           std::string session_id)
    : engine_(engine),
      context_(engine ? engine->context() : nullptr),
      config_(std::move(config)),
      session_id_(std::move(session_id)),
      candidate_graph_(engine),
      worker_(WorkerConfig{config_.endpoint, session_id_, config_.mode,
                           config_.max_candidates, config_.candidate_max_candidates,
                           config_.candidate_timeout_ms, config_.max_tokens,
                           config_.timeout_ms}) {}

SessionState::~SessionState() {
  stopped_ = true;
  commit_connection_.disconnect();
  update_connection_.disconnect();
  worker_.Stop();
  Remove(engine_, this);
}

void SessionState::Start() {
  if (started_ || !config_.enabled || !context_)
    return;
  started_ = true;
  const auto weak = weak_from_this();
  worker_.SetResultCallback([weak](PredictionResult result) {
    if (auto state = weak.lock())
      state->PostResult(std::move(result));
  });
  commit_connection_ = context_->commit_notifier().connect([weak](rime::Context* ctx) {
    if (auto state = weak.lock())
      state->OnCommit(ctx);
  });
  update_connection_ = context_->update_notifier().connect([weak](rime::Context* ctx) {
    if (auto state = weak.lock())
      state->OnContextUpdate(ctx);
  });
  worker_.Start();
}

std::shared_ptr<SessionState> SessionState::Acquire(rime::Engine* engine,
                                                     const LlmConfig& config,
                                                     const std::string& session_id) {
  if (!engine || !config.enabled)
    return nullptr;
  std::lock_guard<std::mutex> lock(registry_mutex);
  if (auto existing = registry[engine].lock())
    return existing;
  auto state = std::make_shared<SessionState>(engine, config, session_id);
  registry[engine] = state;
  state->Start();
  return state;
}

std::shared_ptr<SessionState> SessionState::Find(rime::Engine* engine) {
  std::lock_guard<std::mutex> lock(registry_mutex);
  auto found = registry.find(engine);
  return found == registry.end() ? nullptr : found->second.lock();
}

void SessionState::Remove(rime::Engine* engine, SessionState* state) {
  std::lock_guard<std::mutex> lock(registry_mutex);
  auto found = registry.find(engine);
  if (found != registry.end() && found->second.expired())
    registry.erase(found);
  (void)state;
}

void SessionState::PostResult(PredictionResult result) {
  auto* completion = new Completion{weak_from_this(), std::move(result)};
  dispatch_async_f(dispatch_get_main_queue(), completion, &ApplyCompletion);
}

void SessionState::ApplyResult(PredictionResult result) {
  if (stopped_ || result.request_id != request_id_ || !context_ ||
      !context_->input().empty() || result.candidates.empty()) {
    return;
  }
  candidates_ = std::move(result.candidates);
  if (!config_.trigger.empty())
    return;
  visible_ = true;
  InstallPredictionSegment();
}

void SessionState::OnCommit(rime::Context* context) {
  if (stopped_ || context != context_)
    return;
  const std::string text = context->GetCommitText();
  if (text.empty())
    return;
  Invalidate();
  worker_.SubmitCommit(text);
  if (IsChineseText(text))
    worker_.SubmitPrediction(request_id_, config_.idle_delay_ms);
}

bool SessionState::MatchesTrigger(const rime::KeyEvent& key) const {
  if (config_.trigger.empty())
    return false;
  rime::KeyEvent configured;
  return configured.Parse(config_.trigger) && configured == key;
}

bool SessionState::ShowCached() {
  if (stopped_ || candidates_.empty() || !context_)
    return false;
  visible_ = true;
  InstallPredictionSegment();
  return true;
}

void SessionState::Cancel() {
  if (!visible_ && candidates_.empty())
    return;
  Invalidate();
}

bool SessionState::Accept(size_t index) {
  if (!visible_ || index >= candidates_.size() || !engine_ || !context_)
    return false;
  const std::string text = candidates_[index].text;
  Invalidate();
  context_->Clear();
  engine_->CommitText(text);
  worker_.SubmitCommit(text);
  if (IsChineseText(text))
    worker_.SubmitPrediction(request_id_, config_.idle_delay_ms);
  return true;
}

void SessionState::InsertSpace() {
  if (!engine_ || !context_)
    return;
  Invalidate();
  context_->Clear();
  engine_->CommitText(" ");
}

void SessionState::HideForInput() {
  Invalidate();
}

void SessionState::Invalidate() {
  ++request_id_;
  worker_.Invalidate(request_id_);
  candidates_.clear();
  model_candidates_.clear();
  candidate_paths_.clear();
  model_input_.clear();
  if (visible_)
    ClearPredictionContext();
  visible_ = false;
}

bool SessionState::IsPredictionContext() const {
  return context_ && !context_->composition().empty() &&
         context_->composition().back().HasTag("llm_prediction");
}

void SessionState::ClearPredictionContext() {
  if (IsPredictionContext())
    context_->Clear();
}

void SessionState::InstallPredictionSegment() {
  if (!context_ || !visible_ || candidates_.empty())
    return;
  if (!IsPredictionContext()) {
    const size_t end = context_->input().size();
    rime::Segment segment(static_cast<int>(end), static_cast<int>(end));
    segment.tags.insert("llm_prediction");
    segment.tags.insert("placeholder");
    context_->composition().AddSegment(std::move(segment));
  }
  context_->update_notifier()(context_);
}

}  // namespace rime_llm
