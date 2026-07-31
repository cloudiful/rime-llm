#ifndef RIME_LLM_HTTP_CLIENT_H_
#define RIME_LLM_HTTP_CLIENT_H_

#include <string>

namespace rime_llm {

struct HttpResponse {
  int status = 0;
  std::string body;
};

bool PostJson(const std::string& url,
              const std::string& payload,
              int timeout_ms,
              HttpResponse* response,
              std::string* error);

}  // namespace rime_llm

#endif
