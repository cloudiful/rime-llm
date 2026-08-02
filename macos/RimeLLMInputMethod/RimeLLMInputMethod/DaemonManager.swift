import Foundation

/// Launches the bundled ime-daemon child process when no daemon is already
/// reachable, and terminates it when the input method quits.
final class DaemonManager {
    static let shared = DaemonManager()

    private let lock = NSLock()
    private var process: Process?

    /// Probes the daemon and starts the bundled binary if unreachable.
    func ensureRunning(probe: () -> Bool) {
        lock.lock()
        defer { lock.unlock() }
        if process != nil { return }
        if probe() { return }
        guard let executable = Bundle.main.url(forResource: "ime-daemon", withExtension: nil) else {
            return
        }

        let process = Process()
        process.executableURL = executable
        var environment = ProcessInfo.processInfo.environment
        if let resources = Bundle.main.resourceURL {
            let dictionaryRoot = resources.appendingPathComponent("data/rime-ice", isDirectory: true)
            if FileManager.default.fileExists(atPath: dictionaryRoot.path) {
                environment["RIME_LLM_DICTIONARY_ROOT"] = dictionaryRoot.path
            }
        }
        process.environment = environment
        process.standardOutput = outputHandle()
        process.standardError = outputHandle()
        process.terminationHandler = { [weak self] _ in
            self?.lock.lock()
            self?.process = nil
            self?.lock.unlock()
        }
        do {
            try process.run()
        } catch {
            return
        }
        self.process = process
    }

    /// Terminates the daemon only when this input method launched it.
    func stop() {
        lock.lock()
        let process = self.process
        self.process = nil
        lock.unlock()
        if process?.isRunning == true {
            process?.terminate()
        }
    }

    private func outputHandle() -> FileHandle {
        let logDirectory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/RimeLLM", isDirectory: true)
        let logFile = logDirectory.appendingPathComponent("ime-daemon.log")
        do {
            try FileManager.default.createDirectory(at: logDirectory, withIntermediateDirectories: true)
            if !FileManager.default.fileExists(atPath: logFile.path) {
                FileManager.default.createFile(atPath: logFile.path, contents: nil)
            }
            let handle = try FileHandle(forUpdating: logFile)
            handle.seekToEndOfFile()
            return handle
        } catch {
            return .nullDevice
        }
    }
}
