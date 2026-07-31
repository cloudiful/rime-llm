#ifndef RIME_LLM_TRANSLATOR_H_
#define RIME_LLM_TRANSLATOR_H_

#include <rime/translator.h>

#include <memory>

namespace rime_llm {
class SessionState;

class LlmTranslator : public rime::Translator {
 public:
  LlmTranslator(const rime::Ticket& ticket, std::shared_ptr<SessionState> state);
  rime::an<rime::Translation> Query(const rime::string& input,
                                     const rime::Segment& segment) override;

 private:
  std::shared_ptr<SessionState> state_;
};

class LlmTranslatorComponent : public LlmTranslator::Component {
 public:
  LlmTranslator* Create(const rime::Ticket& ticket) override;
};

}  // namespace rime_llm

#endif
