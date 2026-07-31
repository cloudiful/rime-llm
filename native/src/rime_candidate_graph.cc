#include "rime_candidate_graph.h"

#include <rime/algo/syllabifier.h>
#include <rime/dict/dictionary.h>
#include <rime/dict/user_dictionary.h>
#include <rime/engine.h>
#include <rime/gear/translator_commons.h>
#include <rime/language.h>
#include <rime/schema.h>
#include <rime/ticket.h>

#include <algorithm>
#include <map>
#include <unordered_map>
#include <utility>

namespace rime_llm {
namespace {

class TranslatorOptionsView : public rime::TranslatorOptions {
 public:
  explicit TranslatorOptionsView(const rime::Ticket& ticket)
      : TranslatorOptions(ticket) {}

  const rime::hash_set<std::string>& blacklist() const {
    return blacklist_;
  }
};

struct Edge {
  size_t start = 0;
  size_t end = 0;
  rime::DictEntry entry;
};

using EdgeMap = std::map<size_t, std::vector<Edge>>;

struct PartialPath {
  std::string text;
  double score = 0.0;
  std::vector<RimeCandidateComponent> components;
};

bool BetterPath(const PartialPath& left, const PartialPath& right) {
  return left.score > right.score;
}

void KeepBestPaths(std::vector<PartialPath>* paths, size_t limit) {
  if (!paths || limit == 0)
    return;
  std::stable_sort(paths->begin(), paths->end(), BetterPath);
  if (paths->size() > limit)
    paths->resize(limit);
}

void AddEntry(EdgeMap* edges,
              size_t start,
              size_t end,
              const rime::an<rime::DictEntry>& entry) {
  if (!edges || !entry || end <= start)
    return;
  auto& same_span = (*edges)[start];
  auto found = std::find_if(
      same_span.begin(), same_span.end(), [end, &entry](const Edge& existing) {
        return existing.end == end && existing.entry.text == entry->text;
      });
  if (found != same_span.end()) {
    if (entry->weight > found->entry.weight)
      found->entry = *entry;
    return;
  }
  same_span.push_back(Edge{start, end, *entry});
}

template <typename Collector>
void AddCollector(EdgeMap* edges,
                  size_t start,
                  const rime::an<Collector>& collector,
                  size_t input_length,
                  size_t max_homophones) {
  if (!edges || !collector || max_homophones == 0)
    return;
  for (auto& [end, iterator] : *collector) {
    size_t count = 0;
    while (!iterator.exhausted() && count < max_homophones) {
      auto entry = iterator.Peek();
      if (entry && end <= input_length) {
        AddEntry(edges, start, end, entry);
        ++count;
      }
      if (!iterator.Next())
        break;
    }
  }
}

void SortAndLimitEdges(EdgeMap* edges, size_t max_homophones) {
  if (!edges)
    return;
  for (auto& [start, same_start] : *edges) {
    std::stable_sort(same_start.begin(), same_start.end(),
                     [](const Edge& left, const Edge& right) {
                       if (left.end != right.end)
                         return left.end < right.end;
                       return left.entry.weight > right.entry.weight;
                     });
    std::vector<Edge> limited;
    limited.reserve(same_start.size());
    for (size_t begin = 0; begin < same_start.size();) {
      size_t group_end = begin + 1;
      while (group_end < same_start.size() &&
             same_start[group_end].end == same_start[begin].end) {
        ++group_end;
      }
      const size_t keep = std::min(max_homophones, group_end - begin);
      limited.insert(limited.end(), same_start.begin() + begin,
                     same_start.begin() + begin + keep);
      begin = group_end;
    }
    same_start = std::move(limited);
    (void)start;
  }
}

}  // namespace

RimeCandidateGraph::RimeCandidateGraph(rime::Engine* engine) : engine_(engine) {
  if (!engine_ || !engine_->schema())
    return;

  const rime::Ticket translator_ticket(engine_, "translator");
  if (auto component = rime::Dictionary::Require("dictionary")) {
    dictionary_.reset(component->Create(translator_ticket));
    if (dictionary_ && !dictionary_->Load())
      dictionary_.reset();
  }
  if (auto component = rime::UserDictionary::Require("user_dictionary")) {
    user_dictionary_.reset(component->Create(translator_ticket));
    if (user_dictionary_) {
      user_dictionary_->Load();
      if (dictionary_)
        user_dictionary_->Attach(dictionary_->primary_table(),
                                 dictionary_->prism());
    }
  }

  if (user_dictionary_)
    language_ = std::make_unique<rime::Language>(user_dictionary_->name());
  else if (dictionary_)
    language_ = std::make_unique<rime::Language>(
                 rime::Language::get_language_component(dictionary_->name()));
}

RimeCandidateGraph::~RimeCandidateGraph() = default;

bool RimeCandidateGraph::ready() const {
  return engine_ && dictionary_ && dictionary_->loaded() &&
         dictionary_->prism() && language_;
}

std::vector<RimeCandidatePath> RimeCandidateGraph::Build(
    const std::string& input, size_t max_paths, size_t max_homophones) const {
  if (!ready() || input.empty() || max_paths == 0 || max_homophones == 0)
    return {};

  const rime::Ticket translator_ticket(engine_, "translator");
  const TranslatorOptionsView options(translator_ticket);
  rime::Syllabifier syllabifier(options.delimiters(), options.enable_completion(),
                                options.strict_spelling());
  rime::SyllableGraph syllable_graph;
  if (syllabifier.BuildSyllableGraph(input, *dictionary_->prism(),
                                     &syllable_graph) <= 0 ||
      syllable_graph.interpreted_length != input.size()) {
    return {};
  }

  EdgeMap edges;
  for (const auto& [start, _] : syllable_graph.edges) {
    const auto system_entries = dictionary_->Lookup(
        syllable_graph, start, &options.blacklist(), false);
    AddCollector(&edges, start, system_entries, input.size(), max_homophones);
    if (user_dictionary_) {
      const auto user_entries = user_dictionary_->Lookup(
          syllable_graph, start, 0, 0);
      AddCollector(&edges, start, user_entries, input.size(), max_homophones);
    }
  }
  SortAndLimitEdges(&edges, max_homophones);

  std::vector<std::vector<PartialPath>> paths(input.size() + 1);
  paths[0].push_back(PartialPath{});
  for (size_t start = 0; start < input.size(); ++start) {
    if (paths[start].empty())
      continue;
    auto edge_it = edges.find(start);
    if (edge_it == edges.end())
      continue;
    for (const auto& partial : paths[start]) {
      for (const auto& edge : edge_it->second) {
        if (edge.end > input.size())
          continue;
        PartialPath next = partial;
        next.text += edge.entry.text;
        next.score += edge.entry.weight;
        next.components.push_back(
            RimeCandidateComponent{edge.entry, edge.start, edge.end});
        paths[edge.end].push_back(std::move(next));
      }
    }
    for (size_t end = start + 1; end <= input.size(); ++end) {
      if (!paths[end].empty())
        KeepBestPaths(&paths[end], max_paths);
    }
  }

  auto& complete = paths[input.size()];
  KeepBestPaths(&complete, max_paths);
  std::vector<RimeCandidatePath> result;
  std::unordered_map<std::string, size_t> seen_text;
  for (auto& path : complete) {
    if (path.text.empty() || path.components.empty() ||
        seen_text.find(path.text) != seen_text.end()) {
      continue;
    }
    const size_t index = result.size();
    seen_text.emplace(path.text, index);
    RimeCandidatePath candidate;
    candidate.request.id = "p" + std::to_string(index);
    candidate.request.text = std::move(path.text);
    candidate.request.preedit = input;
    candidate.request.consumedkeys = input.size();
    candidate.request.base_score = path.score;
    candidate.components = std::move(path.components);
    result.push_back(std::move(candidate));
    if (result.size() >= max_paths)
      break;
  }
  return result;
}

}  // namespace rime_llm
