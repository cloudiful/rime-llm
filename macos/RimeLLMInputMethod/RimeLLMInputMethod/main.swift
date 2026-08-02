import AppKit
import InputMethodKit

// The server must outlive the application run loop; the top-level binding
// keeps it alive for the process lifetime.
let connectionName =
    Bundle.main.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String
    ?? "RimeLLMInputMethod_Connection"
let server = IMKServer(name: connectionName, bundleIdentifier: Bundle.main.bundleIdentifier)
guard server != nil else {
    fatalError("Failed to create IMKServer")
}
NSApplication.shared.run()
