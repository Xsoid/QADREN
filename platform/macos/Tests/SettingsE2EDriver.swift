import AppKit
import ApplicationServices

// Accessibility driver for the rendered QADREN windows. The E2E script calls
// one command per step. Labels match the AX title, description, value, or
// identifier. Exit status 0 means the step passed.
//
// Usage: settings-e2e-driver PID COMMAND ARGS...
//   dump WINDOW                      print the labelled AX tree
//   menu MENU ITEM                   press a menu bar item
//   press WINDOW ROLE LABEL [INDEX]  AXPress the INDEX-th match
//   alert WINDOW LABEL               AXPress a button of the open alert only
//   select WINDOW LABEL              select the List or sidebar row with LABEL
//   set WINDOW ROLE LABEL VALUE      set the AX value of a text field
//   wait WINDOW ROLE LABEL [SECONDS] wait until a match exists
//   gone WINDOW ROLE LABEL [SECONDS] wait until no match exists
//   close WINDOW                     press the window close button
//   pick WINDOW POPUP INDEX ITEM     choose ITEM in the INDEX-th popup menu with label POPUP
//   within WINDOW ANCHOR ROLE LABEL  press LABEL in the nearest container that shows ANCHOR
//   date WINDOW INDEX SECONDS        set the INDEX-th date field to now + SECONDS
//   activate                         bring the app under test to the front
//   toggle WINDOW ANCHOR             press the switch or checkbox nearest to ANCHOR

let arguments = CommandLine.arguments
guard arguments.count >= 3, let pid = pid_t(arguments[1]) else {
    FileHandle.standardError.write(Data("usage: settings-e2e-driver PID COMMAND ...\n".utf8))
    exit(64)
}
guard AXIsProcessTrusted() else {
    FileHandle.standardError.write(Data("BLOCKED: Accessibility permission is required for this process\n".utf8))
    exit(77)
}
let application = AXUIElementCreateApplication(pid)

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    AXUIElementCopyAttributeValue(element, name as CFString, &value)
    return value
}

func role(_ element: AXUIElement) -> String { attribute(element, kAXRoleAttribute) as? String ?? "?" }

func label(_ element: AXUIElement) -> String {
    [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, "AXIdentifier"]
        .compactMap { attribute(element, $0) as? String }
        .filter { !$0.isEmpty }
        .joined(separator: " | ")
}

func children(_ element: AXUIElement) -> [AXUIElement] {
    attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
}

func window(_ name: String) -> AXUIElement? {
    let windows = attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []
    return windows.first {
        attribute($0, kAXTitleAttribute) as? String == name || attribute($0, "AXIdentifier") as? String == name
    }
}

func descendants(_ element: AXUIElement, depth: Int = 0) -> [AXUIElement] {
    guard depth < 60 else { return [element] }
    return [element] + children(element).flatMap { descendants($0, depth: depth + 1) }
}

func matches(in root: AXUIElement, role wanted: String, label text: String) -> [AXUIElement] {
    descendants(root).filter { (wanted == "*" || role($0) == wanted) && label($0).contains(text) }
}

func requireWindow(_ name: String) -> AXUIElement {
    guard let found = window(name) else { fail("window not found: \(name)") }
    return found
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("FAIL: \(message)\n".utf8))
    exit(1)
}

func dump(_ element: AXUIElement, depth: Int) {
    let text = label(element)
    if !text.isEmpty { print(String(repeating: " ", count: depth) + role(element) + " " + text) }
    guard depth < 60 else { return }
    for child in children(element) { dump(child, depth: depth + 1) }
}

func press(_ element: AXUIElement) {
    // SwiftUI enables a control after a state change. Do not press it early.
    let enabled = poll(seconds: 5) { attribute(element, kAXEnabledAttribute) as? Bool != false }
    guard enabled else { fail("control stayed disabled: \(label(element))") }
    let result = AXUIElementPerformAction(element, kAXPressAction as CFString)
    guard result == .success else { fail("AXPress returned \(result.rawValue)") }
}

func poll(seconds: Double, _ condition: () -> Bool) -> Bool {
    let deadline = Date().addingTimeInterval(seconds)
    repeat {
        if condition() { return true }
        usleep(200_000)
    } while Date() < deadline
    return condition()
}

let command = arguments[2]
let rest = Array(arguments.dropFirst(3))
switch command {
case "dump":
    dump(requireWindow(rest[0]), depth: 0)
case "menu":
    guard let bar = attribute(application, kAXMenuBarAttribute) else { fail("menu bar unavailable") }
    let menuBar = bar as! AXUIElement
    guard let top = children(menuBar).first(where: { label($0).contains(rest[0]) }),
          let menu = children(top).first,
          let item = children(menu).first(where: { attribute($0, kAXTitleAttribute) as? String == rest[1] })
    else { fail("menu item not found: \(rest[0]) > \(rest[1])") }
    press(item)
case "press":
    let found = matches(in: requireWindow(rest[0]), role: rest[1], label: rest[2])
    let index = rest.count > 3 ? Int(rest[3])! : 0
    guard found.count > index else { fail("\(rest[1]) '\(rest[2])' not found (\(found.count) matches)") }
    press(found[index])
case "alert":
    // SwiftUI alert buttons carry the AppKit identifier `action-button-N`.
    let buttons = matches(in: requireWindow(rest[0]), role: "AXButton", label: rest[1]).filter {
        (attribute($0, "AXIdentifier") as? String)?.hasPrefix("action-button") == true
    }
    guard let button = buttons.first else { fail("alert button not found: \(rest[1])") }
    // A pressed alert button closes the alert before AX returns, so the action
    // result can be an error. The closed alert is the proof of the press.
    _ = AXUIElementPerformAction(button, kAXPressAction as CFString)
    let closed = poll(seconds: 5) {
        matches(in: requireWindow(rest[0]), role: "AXButton", label: rest[1]).allSatisfy {
            (attribute($0, "AXIdentifier") as? String)?.hasPrefix("action-button") != true
        }
    }
    guard closed else { fail("alert stayed open after pressing \(rest[1])") }
case "select":
    guard var row = matches(in: requireWindow(rest[0]), role: "*", label: rest[1]).first else {
        fail("row not found: \(rest[1])")
    }
    while role(row) != "AXRow", let parent = attribute(row, kAXParentAttribute) {
        row = parent as! AXUIElement
    }
    guard AXUIElementSetAttributeValue(row, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success else {
        fail("row selection failed: \(rest[1])")
    }
case "set":
    guard let field = matches(in: requireWindow(rest[0]), role: rest[1], label: rest[2]).first else {
        fail("field not found: \(rest[2])")
    }
    AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    guard AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, rest[3] as CFString) == .success else {
        fail("value change failed: \(rest[2])")
    }
case "wait", "gone":
    let seconds = rest.count > 3 ? Double(rest[3])! : 10
    let expectPresent = command == "wait"
    let passed = poll(seconds: seconds) {
        guard let root = window(rest[0]) else { return !expectPresent }
        return matches(in: root, role: rest[1], label: rest[2]).isEmpty != expectPresent
    }
    guard passed else { fail("\(command) timed out for \(rest[1]) '\(rest[2])'") }
case "close":
    guard let button = attribute(requireWindow(rest[0]), kAXCloseButtonAttribute) else {
        fail("close button not found")
    }
    press(button as! AXUIElement)
case "pick":
    let popups = matches(in: requireWindow(rest[0]), role: "AXPopUpButton", label: rest[1])
    let index = Int(rest[2])!
    guard popups.count > index else { fail("popup '\(rest[1])' not found (\(popups.count) matches)") }
    press(popups[index])
    var item: AXUIElement?
    _ = poll(seconds: 3) {
        item = descendants(popups[index]).first { role($0) == "AXMenuItem" && label($0).contains(rest[3]) }
        return item != nil
    }
    guard let item else { fail("menu item not found: \(rest[3])") }
    press(item)
case "within":
    let root = requireWindow(rest[0])
    guard let anchor = descendants(root).first(where: { label($0) == rest[1] || label($0).hasPrefix(rest[1] + " |") }) else {
        fail("anchor not found: \(rest[1])")
    }
    var container = anchor
    while let parent = attribute(container, kAXParentAttribute) {
        container = parent as! AXUIElement
        if let target = matches(in: container, role: rest[2], label: rest[3]).first {
            press(target)
            exit(0)
        }
        if role(container) == "AXWindow" { break }
    }
    fail("\(rest[2]) '\(rest[3])' not found near '\(rest[1])'")
case "date":
    let fields = descendants(requireWindow(rest[0])).filter { role($0) == "AXDateTimeArea" }
    let index = Int(rest[1])!
    guard fields.count > index else { fail("date field not found (\(fields.count) fields)") }
    let target = Date().addingTimeInterval(Double(rest[2])!) as CFDate
    let result = AXUIElementSetAttributeValue(fields[index], kAXValueAttribute as CFString, target)
    guard result == .success else { fail("date value change returned \(result.rawValue)") }
    print(attribute(fields[index], kAXValueAttribute).map { "\($0)" } ?? "no value")
case "activate":
    // The Finder open panel is a sheet on the key window. It needs the app
    // under test to be active.
    guard NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows]) == true else {
        fail("cannot activate the app under test")
    }
    usleep(500_000)
case "toggle":
    let root = requireWindow(rest[0])
    guard let anchor = descendants(root).first(where: { label($0).contains(rest[1]) }) else {
        fail("anchor not found: \(rest[1])")
    }
    var container = anchor
    while let parent = attribute(container, kAXParentAttribute) {
        container = parent as! AXUIElement
        if let target = descendants(container).first(where: { ["AXCheckBox", "AXSwitch"].contains(role($0)) }) {
            press(target)
            exit(0)
        }
        if role(container) == "AXWindow" { break }
    }
    fail("no switch near '\(rest[1])'")
default:
    fail("unknown command \(command)")
}
