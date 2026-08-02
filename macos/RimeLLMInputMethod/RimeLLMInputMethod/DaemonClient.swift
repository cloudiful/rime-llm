import Foundation

// MARK: - Wire types (ime-daemon protocol, snake_case JSON)

struct CompositionWire: Codable {
    let input: String
    let cursor: Int
    let preeditCursor: Int

    enum CodingKeys: String, CodingKey {
        case input, cursor
        case preeditCursor = "preedit_cursor"
    }
}

struct CandidateWire: Codable {
    let id: String
    let text: String
    let preedit: String
    let consumedkeys: Int
    let baseScore: Float
    let kind: String

    enum CodingKeys: String, CodingKey {
        case id, text, preedit, consumedkeys, kind
        case baseScore = "base_score"
    }
}

struct PredictionWire: Codable {
    let id: String
    let text: String
    let score: Float
    let kind: String

    enum CodingKeys: String, CodingKey {
        case id, text, score
        case kind = "type"
    }
}

struct StateWire: Codable {
    let composition: CompositionWire
    let preedit: String
    let candidates: [CandidateWire]
    let selectedIndex: Int
    let page: Int
    let pageSize: Int
    let predictions: [PredictionWire]
    let modelPending: Bool
    let revision: UInt64
    let eventSeq: UInt64

    enum CodingKeys: String, CodingKey {
        case composition, preedit, candidates, page
        case selectedIndex = "selected_index"
        case pageSize = "page_size"
        case predictions
        case modelPending = "model_pending"
        case revision
        case eventSeq = "event_seq"
    }

    var hasComposition: Bool { !composition.input.isEmpty }
    var hasCandidates: Bool { !candidates.isEmpty }
    var hasPredictions: Bool { !predictions.isEmpty }
}

struct EffectsWire: Codable {
    let commit: String?
    let clear: Bool?
}

struct KeyResponseWire: Codable {
    let sessionId: String
    let state: StateWire
    let effects: EffectsWire

    enum CodingKeys: String, CodingKey {
        case state, effects
        case sessionId = "session_id"
    }
}

struct SessionResponseWire: Codable {
    let sessionId: String
    let state: StateWire

    enum CodingKeys: String, CodingKey {
        case state
        case sessionId = "session_id"
    }
}

// MARK: - Client

/// HTTP + WebSocket client for the local ime-daemon.
///
/// All request/response methods run synchronously with a short timeout:
/// the daemon answers key events with dictionary candidates immediately
/// and performs model reranking asynchronously, so blocking briefly on the
/// main thread is safe and keeps key handling free of data races.
final class DaemonClient {
    struct Configuration {
        var baseURL: URL
        var keyTimeout: TimeInterval = 1.0
        var probeTimeout: TimeInterval = 0.5
        var sessionRetries = 30

        init() {
            let env = ProcessInfo.processInfo.environment
            if let override = env["RIME_LLM_DAEMON_URL"],
               let url = URL(string: override)
            {
                baseURL = url
            } else {
                baseURL = URL(string: "http://127.0.0.1:32124")!
            }
        }
    }

    let configuration: Configuration

    private(set) var sessionId: String?
    private(set) var lastState: StateWire?
    private let session: URLSession
    private let decoder = JSONDecoder()
    private var websocketTask: URLSessionWebSocketTask?
    private var stopped = false
    private var reconnectScheduled = false

    /// Called on the main queue whenever a newer state snapshot arrives.
    var onState: ((StateWire) -> Void)?

    init(configuration: Configuration = Configuration()) {
        self.configuration = configuration
        let urlSessionConfig = URLSessionConfiguration.ephemeral
        urlSessionConfig.timeoutIntervalForRequest = configuration.keyTimeout
        urlSessionConfig.timeoutIntervalForResource = 10
        session = URLSession(configuration: urlSessionConfig)
    }

    // MARK: Session lifecycle

    /// True when the daemon answers /healthz.
    func probe() -> Bool {
        guard let url = url(path: "healthz") else { return false }
        var request = URLRequest(url: url)
        request.timeoutInterval = configuration.probeTimeout
        return send(request: request, timeout: configuration.probeTimeout) != nil
    }

    /// Creates a session, retrying while the daemon finishes starting up.
    func createSession() -> Bool {
        guard sessionId == nil else { return true }
        guard let url = url(path: "v1/sessions") else { return false }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        for _ in 0..<configuration.sessionRetries {
            if let data = send(request: request, timeout: configuration.keyTimeout),
               let response = try? decoder.decode(SessionResponseWire.self, from: data)
            {
                sessionId = response.sessionId
                apply(response.state)
                return true
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        return false
    }

    func deleteSession() {
        guard let sessionId, let url = url(path: "v1/sessions/\(sessionId)") else { return }
        stopEvents()
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        _ = send(request: request, timeout: configuration.probeTimeout)
        self.sessionId = nil
        lastState = nil
    }

    // MARK: Key handling

    /// Sends one key event; nil means the daemon is unreachable and the
    /// caller should pass the key through to the host application.
    func sendKey(_ event: ProtocolEvent) -> KeyResponseWire? {
        guard let sessionId, let url = url(path: "v1/sessions/\(sessionId)/key") else {
            return nil
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        guard let payload = try? JSONEncoder().encode(KeyRequestWire(event: event.name, value: event.value)),
              let data = send(request: request, body: payload, timeout: configuration.keyTimeout),
              let response = try? decoder.decode(KeyResponseWire.self, from: data)
        else {
            return nil
        }
        apply(response.state)
        return response
    }

    /// Records committed text (user frequency + model context) without
    /// blocking the caller; refreshed predictions arrive over WebSocket.
    func commitAck(text: String) {
        guard let sessionId, let url = url(path: "v1/sessions/\(sessionId)/commit-ack"),
              let payload = try? JSONEncoder().encode(CommitAckRequestWire(text: text))
        else {
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = configuration.keyTimeout
        request.httpBody = payload
        let task = session.dataTask(with: request) { _, _, _ in }
        task.resume()
    }

    // MARK: WebSocket events

    func startEvents() {
        guard let sessionId, let url = url(path: "v1/sessions/\(sessionId)/events") else {
            return
        }
        stopped = false
        let task = session.webSocketTask(with: url)
        websocketTask = task
        task.resume()
        receiveLoop(task)
    }

    func stopEvents() {
        stopped = true
        websocketTask?.cancel(with: .goingAway, reason: nil)
        websocketTask = nil
    }

    private func receiveLoop(_ task: URLSessionWebSocketTask) {
        task.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure:
                self.scheduleReconnect()
            case .success(.string(let text)):
                if let data = text.data(using: .utf8),
                   let state = try? self.decoder.decode(StateWire.self, from: data)
                {
                    self.apply(state)
                }
                self.receiveLoop(task)
            case .success(.data):
                self.receiveLoop(task)
            @unknown default:
                self.receiveLoop(task)
            }
        }
    }

    private func scheduleReconnect() {
        guard !stopped, sessionId != nil, !reconnectScheduled else { return }
        reconnectScheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            guard let self else { return }
            self.reconnectScheduled = false
            guard !self.stopped, self.sessionId != nil else { return }
            self.startEvents()
        }
    }

    private func apply(_ state: StateWire) {
        if let last = lastState, state.eventSeq <= last.eventSeq {
            return
        }
        lastState = state
        DispatchQueue.main.async { [weak self] in
            self?.onState?(state)
        }
    }

    // MARK: Transport helpers

    private func url(path: String) -> URL? {
        configuration.baseURL
            .appendingPathComponent(path)
    }

    private func send(request: URLRequest, body: Data? = nil, timeout: TimeInterval) -> Data? {
        var request = request
        request.httpBody = body
        request.timeoutInterval = timeout
        let semaphore = DispatchSemaphore(value: 0)
        var result: Data?
        let task = session.dataTask(with: request) { data, _, _ in
            result = data
            semaphore.signal()
        }
        task.resume()
        if semaphore.wait(timeout: .now() + timeout) == .timedOut {
            task.cancel()
            return nil
        }
        return result
    }
}

private struct KeyRequestWire: Encodable {
    let event: String
    let value: String?
}

private struct CommitAckRequestWire: Encodable {
    let text: String
}
