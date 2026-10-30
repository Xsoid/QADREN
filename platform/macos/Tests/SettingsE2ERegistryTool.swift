import Foundation

// A second local controller for the Settings E2E. It changes Registry state
// while the rendered Settings window shows an older plan. It uses the same
// QADREN_DATA_ROOT as the app under test.
//
// Usage: settings-e2e-registry move-right-root SCENARIO_NAME NEW_RIGHT_ROOT
//        settings-e2e-registry revision SCENARIO_NAME
//        settings-e2e-registry local-enabled true|false
//        settings-e2e-registry setup-scheduled NAME LEFT RIGHT TZ WEEKDAY HOUR MINUTE

@main
struct SettingsE2ERegistryTool {
    static func main() throws {
        let arguments = CommandLine.arguments
        // Safety: this tool changes Registry state. Refuse to touch the user's
        // real Application Support data; a test must name its data root.
        guard let root = ProcessInfo.processInfo.environment[QADRENDataLocation.overrideVariable],
              root.hasPrefix("/"), !root.isEmpty else {
            FileHandle.standardError.write(Data("settings-e2e-registry: QADREN_DATA_ROOT is required\n".utf8))
            exit(64)
        }
        guard arguments.count >= 3 else { throw StorageError.operation("usage: settings-e2e-registry COMMAND NAME ...") }
        if arguments[1] == "setup-scheduled", arguments.count == 9 {
            // One Local connection, a one-way Scenario, and one weekly slot.
            let connection = try StorageRegistryBridge.ensureLocalComputer()
            let scenario = try StorageRegistryBridge.createScenario(
                name: arguments[2], leftID: connection, leftRoot: arguments[3],
                rightID: connection, rightRoot: arguments[4], direction: "left_to_right"
            )
            try StorageRegistryBridge.createScenarioSchedule(
                scenarioID: scenario, timezone: arguments[5], intervalHours: 0,
                daysMask: 1 << UInt8(arguments[6])!, hour: UInt8(arguments[7])!, minute: UInt8(arguments[8])!
            )
            print(scenario)
            return
        }
        if arguments[1] == "local-enabled" {
            for entry in try StorageRegistryBridge.list() where entry.adapter == "local_filesystem" {
                try StorageRegistryBridge.setEnabled(id: entry.id, enabled: arguments[2] == "true")
            }
            return
        }
        guard let scenario = try StorageRegistryBridge.listScenarios().first(where: { $0.name == arguments[2] }) else {
            throw StorageError.operation("Scenario not found: \(arguments[2])")
        }
        switch arguments[1] {
        case "move-right-root":
            try StorageRegistryBridge.updateScenario(
                id: scenario.id, name: scenario.name,
                leftID: scenario.left_connection_id, leftRoot: scenario.left_root,
                rightID: scenario.right_connection_id, rightRoot: arguments[3],
                direction: scenario.direction
            )
        case "revision":
            print(scenario.revision)
        default:
            throw StorageError.operation("unknown command \(arguments[1])")
        }
    }
}
