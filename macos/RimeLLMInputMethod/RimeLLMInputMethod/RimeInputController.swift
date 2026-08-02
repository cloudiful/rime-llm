import AppKit
import InputMethodKit

/// IMK controller bridging one client session to a local ime-daemon session.
///
/// Key events are translated by `KeyMapper` and sent synchronously to the
/// daemon, which answers with dictionary candidates and commit/clear
/// effects. Model reranks and predictions arrive asynchronously over
/// WebSocket and only refresh the candidate panel when the daemon-issued
/// event sequence has advanced.
@objc(RimeInputController)
final class RimeInputController: IMKInputController {
    private let daemon = DaemonClient()
    private var candidatesWindow: IMKCandidates!
    private var lastState: StateWire?

    private var inputClient: IMKTextInput? {
        client() as? IMKTextInput
    }

    private var interactive: Bool {
        guard let state = lastState else { return false }
        return state.hasComposition || state.hasCandidates || state.hasPredictions
    }

    private var predictionOnly: Bool {
        guard let state = lastState else { return false }
        return !state.hasComposition && state.hasPredictions
    }

    override init!(server: IMKServer!, delegate: Any!, client inputClient: Any!) {
        super.init(server: server, delegate: delegate, client: inputClient)
        candidatesWindow = IMKCandidates(
            server: server,
            panelType: IMKCandidatePanelType(kIMKSingleColumnScrollingCandidatePanel)
        )
        candidatesWindow.setAttributes([
            "IMKCandidatesLineHeightKey": 22.0,
            "IMKCandidatesFontSizeKey": 18.0,
            IMKCandidatesSendServerKeyEventFirst: true,
        ])
        candidatesWindow.setDismissesAutomatically(false)
        daemon.onState = { [weak self] state in
            self?.apply(state, effects: nil)
        }
    }

    // MARK: Session lifecycle

    override func activateServer(_ sender: Any!) {
        super.activateServer(sender)
        ensureSession()
    }

    override func deactivateServer(_ sender: Any!) {
        super.deactivateServer(sender)
        candidatesWindow.hide()
    }

    override func inputControllerWillClose() {
        daemon.deleteSession()
        super.inputControllerWillClose()
    }

    private func ensureSession() {
        DaemonManager.shared.ensureRunning(probe: daemon.probe)
        guard daemon.createSession() else { return }
        daemon.startEvents()
    }

    // MARK: Key handling

    override func handle(_ event: NSEvent, client sender: Any) -> Bool {
        guard event.type == .keyDown else { return false }
        guard let protocolEvent = KeyMapper.map(event: event, active: interactive) else {
            return false
        }
        return handleKey(protocolEvent)
    }

    override func inputText(_ string: String, client sender: Any) -> Bool {
        guard let character = string.first, character.isASCII, character.isLetter else {
            return false
        }
        return handleKey(ProtocolEvent(name: "letter", value: String(Character(character.lowercased()))))
    }

    private func handleKey(_ event: ProtocolEvent) -> Bool {
        if predictionOnly {
            return handlePredictionKey(event)
        }
        if let response = sendKey(event) {
            apply(response.state, effects: response.effects)
            return true
        }
        // Daemon unavailable: release the key to the host application.
        clearUI()
        return false
    }

    private func sendKey(_ event: ProtocolEvent) -> KeyResponseWire? {
        if let response = daemon.sendKey(event) {
            return response
        }
        ensureSession()
        return daemon.sendKey(event)
    }

    /// Keys while only model predictions are visible (no composition):
    /// digits/space/enter select a prediction, escape dismisses them.
    private func handlePredictionKey(_ event: ProtocolEvent) -> Bool {
        guard let predictions = lastState?.predictions, !predictions.isEmpty else {
            return false
        }
        switch event.name {
        case "letter":
            // Predictions are a transient view over an empty composition.
            // Re-dispatch the letter so the first new character starts a
            // composition instead of being passed through to the host.
            clearUI()
            return handleKey(event)
        case "digit":
            guard let value = event.value.flatMap(Int.init),
                  value >= 1, value <= predictions.count
            else {
                return false
            }
            commit(predictions[value - 1].text)
            return true
        case "space", "enter":
            commit(predictions[0].text)
            return true
        case "escape":
            if let response = daemon.sendKey(event) {
                apply(response.state, effects: response.effects)
            } else {
                clearUI()
            }
            return true
        default:
            return false
        }
    }

    // MARK: Applying state

    private func apply(_ state: StateWire, effects: EffectsWire?) {
        let isNewer = lastState.map { state.eventSeq > $0.eventSeq } ?? true
        if isNewer {
            lastState = state
            render(state)
        }
        if let text = effects?.commit {
            inputClient?.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
            daemon.commitAck(text: text)
        }
    }

    private func commit(_ text: String) {
        inputClient?.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
        daemon.commitAck(text: text)
        clearUI()
    }

    private func render(_ state: StateWire) {
        if state.preedit.isEmpty {
            inputClient?.setMarkedText(
                "",
                selectionRange: NSRange(location: 0, length: 0),
                replacementRange: NSRange(location: NSNotFound, length: 0)
            )
        } else {
            let attributes: [NSAttributedString.Key: Any] = [
                .underlineStyle: NSUnderlineStyle.single.rawValue,
                .underlineColor: NSColor.labelColor,
            ]
            let marked = NSAttributedString(string: state.preedit, attributes: attributes)
            inputClient?.setMarkedText(
                marked,
                selectionRange: NSRange(location: state.composition.preeditCursor, length: 0),
                replacementRange: NSRange(location: NSNotFound, length: 0)
            )
        }

        if state.hasCandidates {
            let pageSize = max(1, state.pageSize)
            guard state.page >= 0 else {
                candidatesWindow.hide()
                return
            }
            let (pageStart, overflow) = state.page.multipliedReportingOverflow(by: pageSize)
            guard !overflow, pageStart < state.candidates.count else {
                candidatesWindow.hide()
                return
            }
            let pageEnd = min(pageStart + pageSize, state.candidates.count)
            let page = state.candidates[pageStart..<pageEnd]
            let selectedLine = state.selectedIndex >= pageStart
                ? state.selectedIndex - pageStart
                : 0
            showPanel(items: page.map { $0.text }, selectedLine: selectedLine)
        } else if state.hasPredictions && !state.hasComposition {
            showPanel(items: state.predictions.map { $0.text }, selectedLine: 0)
        } else {
            candidatesWindow.hide()
        }
    }

    private func showPanel(items: [String], selectedLine: Int) {
        candidatesWindow.setCandidateData(items)
        candidatesWindow.update()
        if !candidatesWindow.isVisible() {
            candidatesWindow.show(kIMKLocateCandidatesBelowHint)
        }
        guard selectedLine >= 0, selectedLine < items.count else { return }
        let identifier = candidatesWindow.candidateIdentifier(atLineNumber: selectedLine)
        if identifier != NSNotFound {
            _ = candidatesWindow.selectCandidate(withIdentifier: identifier)
        }
    }

    private func clearUI() {
        lastState = nil
        candidatesWindow.hide()
        inputClient?.setMarkedText(
            "",
            selectionRange: NSRange(location: 0, length: 0),
            replacementRange: NSRange(location: NSNotFound, length: 0)
        )
    }

    // MARK: Candidate panel callbacks

    override func candidateSelected(_ candidateString: NSAttributedString) {
        let text = candidateString.string
        guard !text.isEmpty else { return }
        if let state = lastState, state.hasCandidates {
            // Route dictionary candidate clicks through the daemon so its
            // state machine (partial consumption) stays in sync.
            if let index = state.candidates.firstIndex(where: { $0.text == text }) {
                let event = ProtocolEvent(name: "select", value: String(index))
                if let response = daemon.sendKey(event) {
                    apply(response.state, effects: response.effects)
                    return
                }
            }
        }
        commit(text)
    }

    // MARK: Composition end

    override func commitComposition(_ sender: Any!) {
        if let state = lastState, state.hasComposition {
            inputClient?.insertText(
                state.composition.input,
                replacementRange: NSRange(location: NSNotFound, length: 0)
            )
            daemon.commitAck(text: state.composition.input)
            _ = daemon.sendKey(ProtocolEvent(name: "escape", value: nil))
            clearUI()
        }
        super.commitComposition(sender)
    }

    override func hidePalettes() {
        candidatesWindow.hide()
        super.hidePalettes()
    }
}
