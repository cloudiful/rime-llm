#ifndef RIME_LLM_PROCESSOR_H_
#define RIME_LLM_PROCESSOR_H_

#include <rime/processor.h>

#include <memory>

namespace rime_llm {
class SessionState;

class LlmProcessor : public rime::Processor {
 public:
  LlmProcessor(const rime::Ticket& ticket, std::shared_ptr<SessionState> state);
  rime::ProcessResult ProcessKeyEvent(const rime::KeyEvent& key_event) override;

 private:
  std::shared_ptr<SessionState> state_;
};

class LlmProcessorComponent : public LlmProcessor::Component {
 public:
  LlmProcessor* Create(const rime::Ticket& ticket) override;
};

}  // namespace rime_llm

#endif
