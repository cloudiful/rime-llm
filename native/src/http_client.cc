#include "http_client.h"

#include <cerrno>
#include <cstring>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

#include <algorithm>
#include <cstdlib>

namespace rime_llm {
namespace {

struct ParsedUrl {
  std::string host;
  std::string port;
  std::string path;
};

bool ParseUrl(const std::string& url, ParsedUrl* result) {
  const std::string prefix = "http://";
  if (url.compare(0, prefix.size(), prefix) != 0)
    return false;
  const auto authority_start = prefix.size();
  const auto path_start = url.find('/', authority_start);
  const auto authority = url.substr(
      authority_start, path_start == std::string::npos
                          ? std::string::npos
                          : path_start - authority_start);
  const auto colon = authority.rfind(':');
  result->host = colon == std::string::npos ? authority
                                             : authority.substr(0, colon);
  result->port = colon == std::string::npos ? "80" : authority.substr(colon + 1);
  result->path = path_start == std::string::npos ? "/" : url.substr(path_start);
  return !result->host.empty() && !result->port.empty() && !result->path.empty();
}

bool SendAll(int fd, const std::string& request) {
  size_t sent = 0;
  while (sent < request.size()) {
    const ssize_t count = send(fd, request.data() + sent, request.size() - sent, 0);
    if (count <= 0)
      return false;
    sent += static_cast<size_t>(count);
  }
  return true;
}

std::string DecodeChunkedBody(const std::string& body) {
  std::string decoded;
  size_t cursor = 0;
  while (cursor < body.size()) {
    const auto line_end = body.find("\r\n", cursor);
    if (line_end == std::string::npos)
      return {};
    const auto size_text = body.substr(cursor, line_end - cursor);
    const auto chunk_size = std::strtoul(size_text.c_str(), nullptr, 16);
    cursor = line_end + 2;
    if (chunk_size == 0)
      return decoded;
    if (chunk_size > body.size() - cursor ||
        body.size() - cursor < chunk_size + 2)
      return {};
    decoded.append(body, cursor, chunk_size);
    cursor += chunk_size + 2;
  }
  return {};
}

}  // namespace

bool PostJson(const std::string& url,
              const std::string& payload,
              int timeout_ms,
              HttpResponse* response,
              std::string* error) {
  ParsedUrl parsed;
  if (!ParseUrl(url, &parsed)) {
    if (error)
      *error = "only plain http URLs are supported";
    return false;
  }

  addrinfo hints{};
  hints.ai_socktype = SOCK_STREAM;
  hints.ai_family = AF_UNSPEC;
  addrinfo* addresses = nullptr;
  if (getaddrinfo(parsed.host.c_str(), parsed.port.c_str(), &hints, &addresses) != 0) {
    if (error)
      *error = "failed to resolve HTTP host";
    return false;
  }

  int fd = -1;
  for (auto* address = addresses; address; address = address->ai_next) {
    fd = socket(address->ai_family, address->ai_socktype, address->ai_protocol);
    if (fd < 0)
      continue;
    const auto seconds = std::max(timeout_ms, 1) / 1000;
    const auto micros = (std::max(timeout_ms, 1) % 1000) * 1000;
    timeval timeout{seconds, micros};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
    if (connect(fd, address->ai_addr, address->ai_addrlen) == 0)
      break;
    close(fd);
    fd = -1;
  }
  freeaddrinfo(addresses);
  if (fd < 0) {
    if (error)
      *error = "failed to connect to HTTP service";
    return false;
  }

  const std::string request =
      "POST " + parsed.path + " HTTP/1.1\r\nHost: " + parsed.host +
      "\r\nContent-Type: application/json\r\nAccept: application/json\r\n"
      "Connection: close\r\nContent-Length: " + std::to_string(payload.size()) +
      "\r\n\r\n" + payload;
  if (!SendAll(fd, request)) {
    close(fd);
    if (error)
      *error = "failed to send HTTP request";
    return false;
  }

  std::string raw;
  char buffer[8192];
  while (true) {
    const ssize_t count = recv(fd, buffer, sizeof(buffer), 0);
    if (count == 0)
      break;
    if (count < 0) {
      close(fd);
      if (error)
        *error = "failed to read HTTP response";
      return false;
    }
    raw.append(buffer, static_cast<size_t>(count));
    if (raw.size() > 4 * 1024 * 1024) {
      close(fd);
      if (error)
        *error = "HTTP response is too large";
      return false;
    }
  }
  close(fd);

  const auto header_end = raw.find("\r\n\r\n");
  const auto status_end = raw.find("\r\n");
  if (header_end == std::string::npos || status_end == std::string::npos) {
    if (error)
      *error = "malformed HTTP response";
    return false;
  }
  const int status = std::atoi(raw.substr(9, status_end - 9).c_str());
  std::string body = raw.substr(header_end + 4);
  const auto transfer = raw.substr(status_end, header_end - status_end);
  if (transfer.find("Transfer-Encoding: chunked") != std::string::npos ||
      transfer.find("transfer-encoding: chunked") != std::string::npos) {
    body = DecodeChunkedBody(body);
  }
  if (status < 200 || status >= 300) {
    if (error)
      *error = "HTTP service returned status " + std::to_string(status);
    return false;
  }
  response->status = status;
  response->body = std::move(body);
  return true;
}

}  // namespace rime_llm
