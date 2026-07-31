#include <rime/registry.h>
#include <rime_api.h>

#include "llm_filter.h"
#include "llm_processor.h"
#include "llm_translator.h"

static void rime_llm_predict_initialize() {
  auto& registry = rime::Registry::instance();
  registry.Register("llm_predictor", new rime_llm::LlmProcessorComponent);
  registry.Register("llm_predict_translator",
                    new rime_llm::LlmTranslatorComponent);
  registry.Register("llm_candidate_filter",
                    new rime_llm::LlmCandidateFilterComponent);
}

static void rime_llm_predict_finalize() {}

RIME_REGISTER_MODULE(llm_predict)
