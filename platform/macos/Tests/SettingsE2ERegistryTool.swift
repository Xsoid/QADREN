import Foundation

// A second local controller for the Settings E2E. It changes Registry state
// while the rendered Settings window shows an older plan. It uses the same
// QADREN_DATA_ROOT as the app under test.
//
// Usage: settings-e2e-registry move-right-root SCENARIO_NAME NEW_RIGHT_ROOT
//        settings-e2e-registry revision SCENARIO_NAME
//        settings-e2e-registry local-enabled true|false

@main
struct SettingsE2ERegistryTool {
    static func main() throws {
        let arguments = CommandLine.arguments
        guard arguments.count >= 3 else { throw StorageError.operation("usage: settings-e2e-registry COMMAND NAME ...") }
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
