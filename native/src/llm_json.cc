#include "llm_json.h"

#include <cstdlib>

namespace rime_llm {
namespace {

bool StringValue(const std::string& object,
                 const std::string& key,
                 std::string* value) {
  const auto key_pos = object.find("\"" + key + "\"");
  if (key_pos == std::string::npos)
    return false;
  const auto colon = object.find(':', key_pos);
  if (colon == std::string::npos)
    return false;
  const auto quote = object.find('"', colon + 1);
  if (quote == std::string::npos)
    return false;
  std::string result;
  bool escaped = false;
  for (size_t i = quote + 1; i < object.size(); ++i) {
    const char ch = object[i];
    if (escaped) {
      switch (ch) {
        case '"': result.push_back('"'); break;
        case '\\': result.push_back('\\'); break;
        case '/': result.push_back('/'); break;
        case 'n': result.push_back('\n'); break;
        case 'r': result.push_back('\r'); break;
        case 't': result.push_back('\t'); break;
        default: result.push_back(ch); break;
      }
      escaped = false;
    } else if (ch == '\\') {
      escaped = true;
    } else if (ch == '"') {
      *value = std::move(result);
      return true;
    } else {
      result.push_back(ch);
    }
  }
  return false;
}

bool NumberValue(const std::string& object,
                 const std::string& key,
                 double* value) {
  const auto key_pos = object.find("\"" + key + "\"");
  if (key_pos == std::string::npos)
    return false;
  const auto colon = object.find(':', key_pos);
  if (colon == std::string::npos)
    return false;
  char* end = nullptr;
  const double parsed = std::strtod(object.c_str() + colon + 1, &end);
  if (end == object.c_str() + colon + 1)
    return false;
  *value = parsed;
  return true;
}

bool UIntValue(const std::string& object,
               const std::string& key,
               uint64_t* value) {
  double parsed = 0;
  if (!NumberValue(object, key, &parsed) || parsed < 0)
    return false;
  *value = static_cast<uint64_t>(parsed);
  return true;
}

}  // namespace

std::string JsonEscape(const std::string& value) {
  std::string result = "\"";
  for (unsigned char ch : value) {
    switch (ch) {
      case '"': result += "\\\""; break;
      case '\\': result += "\\\\"; break;
      case '\n': result += "\\n"; break;
      case '\r': result += "\\r"; break;
      case '\t': result += "\\t"; break;
      default:
        if (ch < 0x20)
          result += "\\u00" + std::string(1, "0123456789abcdef"[ch >> 4]) +
                    std::string(1, "0123456789abcdef"[ch & 0xf]);
        else
          result.push_back(static_cast<char>(ch));
    }
  }
  result.push_back('"');
  return result;
}

std::string BuildResetRequest(const std::string& session_id) {
  return "{\"session_id\":" + JsonEscape(session_id) + "}";
}

std::string BuildCommitRequest(const std::string& session_id,
                               const std::string& text) {
  return "{\"session_id\":" + JsonEscape(session_id) +
         ",\"text\":" + JsonEscape(text) + "}";
}

std::string BuildCandidatesRequest(const std::string& session_id,
                                   const std::string& input,
                                   size_t max_candidates,
                                   const std::vector<CandidatePath>& paths) {
  max_candidates = max_candidates == 0 ? 1 : max_candidates;
  std::string result = "{\"session_id\":" + JsonEscape(session_id) +
                       ",\"input\":" + JsonEscape(input) +
                       ",\"max_candidates\":" +
                       std::to_string(max_candidates) + ",\"paths\":[";
  for (size_t i = 0; i < paths.size(); ++i) {
    if (i > 0)
      result.push_back(',');
    const auto& path = paths[i];
    result += "{\"id\":" + JsonEscape(path.id) +
              ",\"text\":" + JsonEscape(path.text) +
              ",\"preedit\":" + JsonEscape(path.preedit) +
              ",\"consumedkeys\":" +
              std::to_string(path.consumedkeys) +
              ",\"base_score\":" +
              std::to_string(path.base_score) + "}";
  }
  result += "]}";
  return result;
}

std::string BuildPredictionRequest(const std::string& session_id,
                                   uint64_t revision,
                                   const std::string& mode,
                                   size_t max_candidates,
                                   size_t max_tokens) {
  max_candidates = max_candidates == 0 ? 1 : max_candidates;
  max_tokens = max_tokens == 0 ? 1 : max_tokens;
  return "{\"session_id\":" + JsonEscape(session_id) +
         ",\"revision\":" + std::to_string(revision) +
         ",\"mode\":" + JsonEscape(mode) +
         ",\"max_candidates\":" + std::to_string(max_candidates) +
         ",\"max_tokens\":" + std::to_string(max_tokens) + "}";
}

bool ParseRevisionResponse(const std::string& body, uint64_t* revision) {
  return UIntValue(body, "revision", revision);
}

bool ParseCandidatesResponse(const std::string& body,
                             CandidatesResponse* response) {
  if (!response)
    return false;
  response->candidates.clear();
  std::string status;
  if (!StringValue(body, "status", &status) || status != "ready")
    return false;
  const auto array_start = body.find("\"candidates\"");
  if (array_start == std::string::npos)
    return false;
  const auto end = body.find(']', array_start);
  if (end == std::string::npos)
    return false;
  size_t cursor = array_start;
  while (true) {
    const auto object_start = body.find('{', cursor);
    if (object_start == std::string::npos || object_start > end)
      break;
    const auto object_end = body.find('}', object_start);
    if (object_end == std::string::npos || object_end > end)
      return false;
    const std::string object = body.substr(object_start, object_end - object_start + 1);
    ModelCandidate candidate;
    double consumedkeys = 0;
    if (!StringValue(object, "id", &candidate.id) ||
        !StringValue(object, "text", &candidate.text) ||
        !StringValue(object, "preedit", &candidate.preedit) ||
        !NumberValue(object, "consumedkeys", &consumedkeys) ||
        consumedkeys < 1 || !IsChineseText(candidate.text)) {
      cursor = object_end + 1;
      continue;
    }
    candidate.consumedkeys = static_cast<uint64_t>(consumedkeys);
    StringValue(object, "type", &candidate.type);
    NumberValue(object, "score", &candidate.score);
    response->candidates.push_back(std::move(candidate));
    cursor = object_end + 1;
  }
  return !response->candidates.empty();
}

bool ParsePredictionResponse(const std::string& body,
                             PredictionResponse* response) {
  if (!response)
    return false;
  response->candidates.clear();
  std::string status;
  if (!StringValue(body, "status", &status) || status != "ready")
    return false;
  if (!UIntValue(body, "revision", &response->revision))
    return false;
  const auto array_start = body.find("\"candidates\"");
  if (array_start == std::string::npos)
    return false;
  const auto end = body.find(']', array_start);
  if (end == std::string::npos)
    return false;
  size_t cursor = array_start;
  while (true) {
    const auto object_start = body.find('{', cursor);
    if (object_start == std::string::npos || object_start > end)
      break;
    const auto object_end = body.find('}', object_start);
    if (object_end == std::string::npos || object_end > end)
      return false;
    const std::string object = body.substr(object_start, object_end - object_start + 1);
    PredictionCandidate candidate;
    if (!StringValue(object, "id", &candidate.id) ||
        !StringValue(object, "text", &candidate.text) ||
        !IsChineseText(candidate.text)) {
      cursor = object_end + 1;
      continue;
    }
    StringValue(object, "type", &candidate.type);
    NumberValue(object, "score", &candidate.score);
    response->candidates.push_back(std::move(candidate));
    cursor = object_end + 1;
  }
  return !response->candidates.empty();
}

bool IsChineseText(const std::string& text) {
  if (text.empty())
    return false;
  size_t i = 0;
  while (i < text.size()) {
    const unsigned char first = static_cast<unsigned char>(text[i]);
    uint32_t code = 0;
    size_t width = 0;
    if (first <= 0x7f) {
      code = first;
      width = 1;
    } else if ((first & 0xe0) == 0xc0 && i + 1 < text.size()) {
      code = (first & 0x1f) << 6 | (static_cast<unsigned char>(text[i + 1]) & 0x3f);
      width = 2;
    } else if ((first & 0xf0) == 0xe0 && i + 2 < text.size()) {
      code = (first & 0x0f) << 12 |
             (static_cast<unsigned char>(text[i + 1]) & 0x3f) << 6 |
             (static_cast<unsigned char>(text[i + 2]) & 0x3f);
      width = 3;
    } else if ((first & 0xf8) == 0xf0 && i + 3 < text.size()) {
      code = (first & 0x07) << 18 |
             (static_cast<unsigned char>(text[i + 1]) & 0x3f) << 12 |
             (static_cast<unsigned char>(text[i + 2]) & 0x3f) << 6 |
             (static_cast<unsigned char>(text[i + 3]) & 0x3f);
      width = 4;
    } else {
      return false;
    }
    if (!((code >= 0x3400 && code <= 0x4dbf) ||
          (code >= 0x4e00 && code <= 0x9fff) ||
          (code >= 0xf900 && code <= 0xfaff))) {
      return false;
    }
    i += width;
  }
  return true;
}

}  // namespace rime_llm
