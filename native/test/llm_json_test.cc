#include "llm_json.h"

#include <cassert>
#include <iostream>

using namespace rime_llm;

int main() {
  assert(IsChineseText("苹果"));
  assert(!IsChineseText("苹果!"));
  assert(!IsChineseText("banana"));

  const auto request = BuildPredictionRequest("session", 3, "free", 5, 0);
  assert(request.find("\"revision\":3") != std::string::npos);
  assert(request.find("\"max_tokens\":1") != std::string::npos);

  const std::vector<CandidatePath> paths = {
      {"p0", "不如", "bu ru", 4, 1.25},
      {"p1", "不入", "bu ru", 4, 0.75},
  };
  const auto candidates_request =
      BuildCandidatesRequest("session", "buru", 0, paths);
  assert(candidates_request.find("\"input\":\"buru\"") != std::string::npos);
  assert(candidates_request.find("\"max_candidates\":1") != std::string::npos);
  assert(candidates_request.find("\"paths\":[") != std::string::npos);
  assert(candidates_request.find("\"id\":\"p0\"") != std::string::npos);
  assert(candidates_request.find("\"base_score\":1.250000") !=
         std::string::npos);

  CandidatesResponse candidates;
  assert(ParseCandidatesResponse(
       R"({"status":"ready","candidates":[{"id":"p0","text":"不如","preedit":"bu ru","consumedkeys":4,"score":0.9,"type":"llm_phrase"},{"id":"bad","text":"hello","preedit":"hello","consumedkeys":5}]})",
       &candidates));
  assert(candidates.candidates.size() == 1);
  assert(candidates.candidates[0].id == "p0");
  assert(candidates.candidates[0].text == "不如");

  PredictionResponse response;
  assert(ParsePredictionResponse(
      R"({"status":"ready","revision":3,"candidates":[{"id":"g0","text":"苹果","score":0.9,"type":"llm_prediction"},{"id":"bad","text":"hello"}]})",
      &response));
  assert(response.revision == 3);
  assert(response.candidates.size() == 1);
  assert(response.candidates[0].text == "苹果");

  uint64_t revision = 0;
  assert(ParseRevisionResponse(R"({"status":"ok","revision":4})", &revision));
  assert(revision == 4);
  std::cout << "native json tests passed\n";
}
