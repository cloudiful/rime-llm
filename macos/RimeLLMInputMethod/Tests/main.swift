import AppKit
import CoreGraphics
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func makeKey(_ keyCode: CGKeyCode, characters: String, flags: CGEventFlags = []) -> NSEvent {
    guard let source = CGEventSource(stateID: .hidSystemState),
          let event = CGEvent(keyboardEventSource: source, virtualKey: keyCode, keyDown: true)
    else {
        fail("cannot create CGEvent")
    }
    var units = Array(characters.utf16)
    event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
    event.flags = flags
    guard let nsEvent = NSEvent(cgEvent: event) else {
        fail("cannot create NSEvent")
    }
    return nsEvent
}

func expect(_ condition: Bool, _ message: String) {
    if !condition {
        fail("FAIL: \(message)")
    }
}

let letterA = makeKey(0, characters: "a")
let shiftA = makeKey(0, characters: "A", flags: .maskShift)
let digitOne = makeKey(18, characters: "1")
let keypadTwo = makeKey(84, characters: "2")
let space = makeKey(49, characters: " ")
let enter = makeKey(36, characters: "\r")
let escape = makeKey(53, characters: "\u{1B}")
let backspace = makeKey(51, characters: "\u{8}")
let left = makeKey(123, characters: "\u{1C}")
let cmdA = makeKey(0, characters: "a", flags: .maskCommand)
let optA = makeKey(0, characters: "å", flags: .maskAlternate)
let accented = makeKey(0, characters: "é")

// Letters are always consumed (they start a composition).
expect(
    KeyMapper.map(event: letterA, active: false) == ProtocolEvent(name: "letter", value: "a"),
    "plain letter maps to lowercase letter event"
)
expect(
    KeyMapper.map(event: shiftA, active: false) == ProtocolEvent(name: "letter", value: "a"),
    "shifted letter is lowercased"
)

// Digits only with an active composition.
expect(
    KeyMapper.map(event: digitOne, active: true) == ProtocolEvent(name: "digit", value: "1"),
    "top-row digit maps with composition"
)
expect(KeyMapper.map(event: digitOne, active: false) == nil, "digit passes through without composition")
expect(
    KeyMapper.map(event: keypadTwo, active: true) == ProtocolEvent(name: "digit", value: "2"),
    "keypad digit maps with composition"
)

// Navigation and editing keys only with an active composition.
expect(KeyMapper.map(event: space, active: true)?.name == "space", "space maps with composition")
expect(KeyMapper.map(event: space, active: false) == nil, "space passes through without composition")
expect(KeyMapper.map(event: enter, active: true)?.name == "enter", "enter maps with composition")
expect(KeyMapper.map(event: enter, active: false) == nil, "enter passes through without composition")
expect(KeyMapper.map(event: escape, active: true)?.name == "escape", "escape maps with composition")
expect(KeyMapper.map(event: escape, active: false) == nil, "escape passes through without composition")
expect(KeyMapper.map(event: backspace, active: true)?.name == "backspace", "backspace maps with composition")
expect(KeyMapper.map(event: backspace, active: false) == nil, "backspace passes through without composition")
expect(KeyMapper.map(event: left, active: true)?.name == "left", "left arrow maps with composition")
expect(KeyMapper.map(event: left, active: false) == nil, "left arrow passes through without composition")

// Modifier combinations and non-ASCII characters always pass through.
expect(KeyMapper.map(event: cmdA, active: true) == nil, "command shortcut passes through")
expect(KeyMapper.map(event: optA, active: true) == nil, "option character passes through")
expect(KeyMapper.map(event: accented, active: true) == nil, "accented character passes through")

let stateJSON = """
{
  "composition": {"input": "buru", "cursor": 4, "preedit_cursor": 5},
  "preedit": "bu ru",
  "candidates": [],
  "selected_index": 0,
  "page": 0,
  "page_size": 9,
  "predictions": [{"id": "p0", "text": "苹果", "score": 1.0, "type": "llm_prediction"}],
  "model_pending": false,
  "revision": 4,
  "event_seq": 1
}
""".data(using: .utf8)!
let decodedState = try! JSONDecoder().decode(StateWire.self, from: stateJSON)
expect(decodedState.composition.preeditCursor == 5, "preedit cursor decodes separately from raw cursor")
expect(decodedState.predictions.first?.kind == "llm_prediction", "prediction type field decodes as kind")

print("keymapper tests passed")
