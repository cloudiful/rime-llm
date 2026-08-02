import AppKit

/// A key event translated into the ime-daemon protocol event name and
/// optional value.
struct ProtocolEvent: Equatable {
    let name: String
    let value: String?
}

/// Maps raw NSEvent keyDown events to ime-daemon protocol events.
///
/// Modifier combinations (Command/Control/Option) always pass through so
/// shortcuts and alt-character input keep working. Keys that need an active
/// composition (digits, arrows, paging, space, enter, escape, backspace,
/// delete) pass through when no composition or prediction list is active.
enum KeyMapper {
    static func map(event: NSEvent, active: Bool) -> ProtocolEvent? {
        let flags = event.modifierFlags
        if !flags.intersection([.command, .control, .option]).isEmpty {
            return nil
        }

        switch event.keyCode {
        case 51: // backspace
            return active ? ProtocolEvent(name: "backspace", value: nil) : nil
        case 117: // forward delete
            return active ? ProtocolEvent(name: "delete", value: nil) : nil
        case 53: // escape
            return active ? ProtocolEvent(name: "escape", value: nil) : nil
        case 36: // return
            return active ? ProtocolEvent(name: "enter", value: nil) : nil
        case 49: // space
            return active ? ProtocolEvent(name: "space", value: nil) : nil
        case 123: // left arrow
            return active ? ProtocolEvent(name: "left", value: nil) : nil
        case 124: // right arrow
            return active ? ProtocolEvent(name: "right", value: nil) : nil
        case 116: // page up
            return active ? ProtocolEvent(name: "pageup", value: nil) : nil
        case 121: // page down
            return active ? ProtocolEvent(name: "pagedown", value: nil) : nil
        default:
            break
        }

        if let digit = digit(for: event.keyCode) {
            return active ? ProtocolEvent(name: "digit", value: String(digit)) : nil
        }
        if let character = letter(for: event) {
            return ProtocolEvent(name: "letter", value: String(character))
        }
        return nil
    }

    /// Top row digits use keyCodes 18...26 (1...9) and 29 (0); the keypad
    /// uses 82 (0), 83...89 (1...7), 91 (8) and 92 (9).
    static func digit(for keyCode: UInt16) -> Int? {
        switch keyCode {
        case 18...26:
            return Int(keyCode - 17)
        case 29:
            return 0
        case 82:
            return 0
        case 83...89:
            return Int(keyCode - 82)
        case 91:
            return 8
        case 92:
            return 9
        default:
            return nil
        }
    }

    /// A single ASCII letter from `charactersIgnoringModifiers`, lowercased.
    /// Non-ASCII characters (for example from alternative keyboard layouts)
    /// pass through to the host application.
    static func letter(for event: NSEvent) -> Character? {
        guard let text = event.charactersIgnoringModifiers, text.count == 1,
              let first = text.first, first.isASCII, first.isLetter
        else {
            return nil
        }
        return Character(first.lowercased())
    }
}
