import Foundation
import Security

@_silgen_name("qadren_registry_list")
private func registryList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_list")
private func scenarioList(_ path: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_create_scoped")
private func scenarioCreate(
    _ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?, _ scope: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_set_enabled")
private func scenarioSetEnabled(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch_get")
private func scenarioCreateBatchGet(
    _ path: UnsafePointer<CChar>?, _ batchID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch_request_cancellation")
private func requestScenarioCreateBatchCancellationFFI(
    _ path: UnsafePointer<CChar>?, _ batchID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch_summaries")
private func scenarioCreateBatchSummariesFFI(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch_summary")
private func scenarioCreateBatchSummaryFFI(
    _ path: UnsafePointer<CChar>?, _ batchID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_set_retry_until_success")
private func scenarioSetRetryUntilSuccessFFI(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ enabled: Int32
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_create_batch_set_paused")
private func setScenarioCreateBatchPausedFFI(
    _ path: UnsafePointer<CChar>?, _ batchID: UnsafePointer<CChar>?, _ paused: Int32
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_pending_create_batches")
private func pendingScenarioCreateBatchesFFI(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_dry_run")
private func scenarioDryRun(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ leftSnapshot: UnsafePointer<CChar>?, _ rightSnapshot: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_queue")
private func scenarioConflictQueue(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ relativePath: UnsafePointer<CChar>?, _ kind: UnsafePointer<CChar>?, _ sourceIsLeft: Int32,
    _ leftSnapshot: UnsafePointer<CChar>?, _ rightSnapshot: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_list")
private func scenarioConflictList(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_conflicts_select")
private func scenarioConflictSelect(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ sourceIsLeft: Int32,
    _ decision: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_local_scenario_metadata_snapshot")
private func localScenarioMetadataSnapshot(
    _ registry: UnsafePointer<CChar>?, _ connectionID: UnsafePointer<CChar>?, _ root: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_file_copy")
private func runLocalScenarioCopyJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_file_replace")
private func runLocalScenarioReplaceJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64,
    _ sourceRoot: UnsafePointer<CChar>?, _ sourceConnection: UnsafePointer<CChar>?, _ sourceRelative: UnsafePointer<CChar>?,
    _ destinationRoot: UnsafePointer<CChar>?, _ destinationConnection: UnsafePointer<CChar>?, _ destinationRelative: UnsafePointer<CChar>?,
    _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_conflict_replace")
private func runLocalScenarioConflictReplaceJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ conflictID: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_run_local_scenario_conflict_delete")
private func runLocalScenarioConflictDeleteJob(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?,
    _ conflictID: UnsafePointer<CChar>?, _ jobID: UnsafePointer<CChar>?, _ consent: Bool
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_jobs_dispatch_scenario_copy_baselines")
private func dispatchScenarioCopyBaselines(
    _ jobs: UnsafePointer<CChar>?, _ registry: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_list")
private func scenarioSchedulesList(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_due")
private func scenarioSchedulesDue(
    _ path: UnsafePointer<CChar>?, _ after: UnsafePointer<CChar>?, _ through: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_dispatch")
private func scenarioSchedulesDispatch(
    _ registryPath: UnsafePointer<CChar>?, _ jobStorePath: UnsafePointer<CChar>?,
    _ after: UnsafePointer<CChar>?, _ through: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_automation_get")
private func scenarioAutomationGet(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_source_wins")
private func scenarioSourceWinsCall(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ revision: UInt64, _ confirm: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_run_state")
private func scenarioRunStateGet(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_root_overlaps")
private func scenarioRootOverlapsGet(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_automation_set_paused")
private func scenarioAutomationSetPaused(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ paused: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_create")
private func scenarioSchedulesCreate(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ timezone: UnsafePointer<CChar>?,
    _ intervalHours: UInt8, _ daysMask: UInt8, _ hour: UInt8, _ minute: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_set_enabled")
private func scenarioSchedulesSetEnabled(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ scheduleID: UnsafePointer<CChar>?,
    _ enabled: UInt8
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenario_schedules_remove")
private func scenarioSchedulesRemove(
    _ path: UnsafePointer<CChar>?, _ scenarioID: UnsafePointer<CChar>?, _ scheduleID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_scenarios_update_scoped")
private func scenarioUpdate(
    _ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?,
    _ leftID: UnsafePointer<CChar>?, _ leftRoot: UnsafePointer<CChar>?,
    _ rightID: UnsafePointer<CChar>?, _ rightRoot: UnsafePointer<CChar>?,
    _ direction: UnsafePointer<CChar>?, _ scope: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_add")
private func registryAdd(_ path: UnsafePointer<CChar>?, _ adapter: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_add_smb_share")
private func registryAddSMBShare(_ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?, _ url: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_smb_endpoint")
private func registrySetSMBEndpoint(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ url: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_add_ispmanager")
private func registryAddISPmanager(_ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?, _ url: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_ispmanager_endpoint")
private func registrySetISPmanagerEndpoint(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ url: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_ispmanager_settings")
private func registryISPmanagerSettings(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_ispmanager_ssh_port")
private func registrySetISPmanagerSSHPort(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ port: UInt16) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_confirm_ispmanager_host_key")
private func registryConfirmISPmanagerHostKey(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ fingerprint: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_ensure_local_computer")
private func registryEnsureLocalComputer(_ path: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_rename")
private func registryRename(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ name: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_enabled")
private func registrySetEnabled(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ enabled: UInt8) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_finalize_disconnect")
private func registryFinalizeDisconnect(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_remove_after_job_gate")
private func registryRemoveAfterJobGate(
    _ registryPath: UnsafePointer<CChar>?, _ jobsPath: UnsafePointer<CChar>?,
    _ id: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_remove_scenario_after_job_gate")
private func registryRemoveScenarioAfterJobGate(
    _ registryPath: UnsafePointer<CChar>?, _ jobsPath: UnsafePointer<CChar>?,
    _ scenarioID: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_credential")
private func registrySetCredential(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ ref: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_status")
private func registrySetStatus(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ status: UnsafePointer<CChar>?, _ timestamp: Int64) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_set_provider_account")
private func registrySetAccount(_ path: UnsafePointer<CChar>?, _ id: UnsafePointer<CChar>?, _ account: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_registry_free_string")
private func registryFree(_ ptr: UnsafeMutablePointer<CChar>?)

struct StorageEntry: Decodable, Identifiable {
    let id: String
    let adapter: String
    let name: String
    let enabled: Bool
    let status: String
    let credential_ref: String?
    let provider_account_id: String?
    /// SMB share address (D-021) or ISPmanager panel URL (D-027). It never
    /// contains a login or a password.
    var endpoint_url: String?
}
struct ScenarioEntry: Decodable, Identifiable {
    let id: String
    let name: String
    let left_connection_id: String
    let left_root: String
    let right_connection_id: String
    let right_root: String
    let direction: String
    let enabled: Bool
    let revision: UInt64
    /// Relative paths of the selected objects. Empty means the full root.
    var selected_paths: [String] = []
    /// `paranoid` or `simple` (D-019).
    var deletion_mode: String = ScenarioDeletionMode.paranoid.rawValue
    /// D-024: retry a temporarily unavailable file every 24 h after the
    /// retry schedule. A Scenario parameter outside the revision.
    var retry_until_success: Bool = true
}

/// How a Scenario revision treats a deletion after a confirmed synchronization.
enum ScenarioDeletionMode: String, CaseIterable, Identifiable {
    /// Every deletion waits for an explicit decision.
    case paranoid
    /// The other side deletes the same unchanged object. A plan with more
    /// than 20 deletions still waits for an explicit decision.
    case simple
    var id: String { rawValue }
}

/// JSON for the `scope_json` argument of the scoped Scenario FFI.
private func scenarioScopeJSON(selectedPaths: [String], deletionMode: ScenarioDeletionMode) throws -> String {
    let object: [String: Any] = ["selected_paths": selectedPaths, "deletion_mode": deletionMode.rawValue]
    let data = try JSONSerialization.data(withJSONObject: object)
    return String(decoding: data, as: UTF8.self)
}
struct ScenarioScheduleEntry: Decodable, Identifiable {
    struct LastSlot: Decodable {
        let occurrence_key: String
        let skipped_active_run: Bool
    }

    let id: String
    let scenario_id: String
    let timezone: String
    /// `nil` for selected days at a time; 1–24 for every N hours.
    let interval_hours: UInt8?
    /// Active local days. Monday is bit 0 and Sunday is bit 6.
    let days_mask: UInt8
    let hour: UInt8
    let minute: UInt8
    let enabled: Bool
    /// The next slot as an RFC 3339 UTC instant.
    let next_at: String?
    let last_slot: LastSlot?
}
struct ScenarioScheduleOccurrence: Decodable, Identifiable, Equatable {
    let scenario_id: String
    let schedule_id: String
    let occurrence_key: String
    let due_at: String

    var id: String { scenario_id + ":" + schedule_id + ":" + occurrence_key }
}
struct ScenarioAutomationEntry: Decodable, Equatable {
    let locally_paused: Bool
    let pending_automatic_run: Bool
}
/// The active Scenario Run and the slots skipped during a run (D-025).
struct ScenarioRunState: Decodable, Equatable {
    struct Run: Decodable, Equatable {
        let id: String
        let trigger: String
        let state: String
        let waiting_reason: String?
        let created_at: Int64
        /// The Scenario whose run this run waits for (write-root lock).
        let waiting_for_scenario: String?
        /// The file batch of an executing run.
        var batch_id: String? = nil
    }
    /// The newest finished run, for the monitoring Scenario widget.
    struct LastRun: Decodable, Equatable {
        let id: String
        let trigger: String
        /// `completed`, `failed`, or `cancelled`.
        let state: String
        let error: String?
        let batch_id: String?
        let created_at: Int64
        let finished_at: Int64?
    }
    struct SkippedSlot: Decodable, Equatable {
        let schedule_id: String
        let occurrence_key: String
        let run_id: String
    }
    let active: Run?
    var last: LastRun? = nil
    let skipped: [SkippedSlot]
}
struct ScenarioSnapshotItem: Codable {
    let relative_path: String
    let object_revision: String?
    let size: UInt64
    let checksum_md5: String?
    /// A symbolic link or special file that the scan skipped. QADREN does not
    /// synchronize it; the planner blocks its path.
    var special: Bool? = nil
}
private struct ScenarioBaselineDispatchStatus: Decodable {
    let dispatched: Int
}
private struct ScenarioScheduleDispatchStatus: Decodable {
    let staged: Int
    let dispatched: Int
}
struct ScenarioPlanAction: Decodable {
    let kind: String
    let left: ScenarioSnapshotItem?
    let right: ScenarioSnapshotItem?
    let from_left: Bool?
    let on_left: Bool?
    let reason: String?
}
struct ScenarioPlanItem: Decodable, Identifiable {
    let relative_path: String
    let left: String
    let right: String
    let action: ScenarioPlanAction
    /// Byte-exact identity. Swift String equality treats NFC and NFD names as
    /// equal, but the file system can hold both as different entries.
    var id: [UInt8] { Array(relative_path.utf8) }
}

extension String {
    /// Byte-exact path comparison for Scenario rows and snapshots.
    func samePathBytes(_ other: String) -> Bool { utf8.elementsEqual(other.utf8) }
}

struct ScenarioCreateBatchReceipt: Decodable {
    let id: String
    let cancellation_requested: Bool
    /// The Worker starts no new file of a paused batch (D-024).
    var paused: Bool? = nil
    let children: [ScenarioCreateBatchChild]
}
/// Progress of one batch from database counts (D-024).
struct ScenarioBatchSummary: Decodable {
    let id: String
    let cancellation_requested: Bool
    let paused: Bool
    let total: Int
    let completed: Int
    let failed: Int
    let cancelled: Int
    let unknown: Int
    let due: Int
    let retry_waiting: Int
    let next_retry_at: Int64?
    let current_path: String?
    /// Pairs of a failure code and its count, most frequent first.
    let failure_reasons: [[CodableValue]]
    let unfinished_sample: [String]

    /// "code: count" texts of `failure_reasons`.
    var reasonTexts: [String] {
        failure_reasons.compactMap { pair in
            guard pair.count == 2, case let .text(code) = pair[0], case let .number(count) = pair[1] else { return nil }
            return "\(code): \(Int(count))"
        }
    }
}

/// A JSON string or number inside a tuple array.
enum CodableValue: Decodable {
    case text(String)
    case number(Double)
    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let text = try? container.decode(String.self) { self = .text(text) } else { self = .number(try container.decode(Double.self)) }
    }
}

struct ScenarioCreateBatchChild: Decodable {
    let relative_path: String
    let state: String
    /// Automatic retries already scheduled (D-024).
    var retries: Int? = nil
    /// Unix seconds of the next retry of a `pending` file.
    var next_retry_at: Int64? = nil
    /// Stable code of the last failure.
    var last_error: String? = nil

    /// A `pending` file that waits for its automatic retry.
    var waitsForRetry: Bool {
        state == "pending" && (next_retry_at.map { TimeInterval($0) > Date().timeIntervalSince1970 } ?? false)
    }
}
struct ScenarioConflictTask: Decodable, Identifiable {
    let id: String
    let scenario_id: String
    let scenario_revision: UInt64
    let relative_path: String
    let kind: String
    let source_is_left: Bool?
    let decision: String?
    let state: String
}
private struct NewConnection: Decodable {
    let id: String
}
private struct ScenarioJobStatus: Decodable {
    let status: String
}
private struct RegistryEnvelope<T: Decodable>: Decodable {
    let ok: Bool
    let data: T?
    let error: String?
}
enum StorageError: Error, LocalizedError {
    case operation(String)
    var errorDescription: String? {
        if case let .operation(message) = self { return message }
        return nil
    }
}
/// Russian text for stable Core and Worker error codes. An unknown code stays
/// visible as is, so the user and the logs keep the exact reason.
enum QADRENUserMessage {
    static func text(for code: String) -> String {
        switch code {
        case "scenario_roots_overlap":
            "Папки сценария пересекаются: одна совпадает с другой или находится внутри неё. Выберите папки, которые не вложены друг в друга."
        case "scenario_run_active":
            "Сценарий сейчас выполняется. Остановите запуск или дождитесь его окончания, затем удалите сценарий."
        case "scenario_has_unresolved_jobs":
            "У сценария есть незавершённые операции или операции с неизвестным результатом. QADREN сначала проверит их. Удалите сценарий после этого."
        case "scenario_revision_stale":
            "Сценарий изменён после построения плана. Вернитесь к списку сценариев и проверьте план заново."
        case "target_exists":
            "Файл назначения уже существует. QADREN не перезаписывает его без отдельного решения."
        case "local_connection_unavailable":
            "Подключение выключено или недоступно. Включите его в разделе «Подключения»."
        case "scenario_disabled":
            "Сценарий выключен. Включите его в настройках сценария."
        case "scenario_unavailable":
            "Сценарий не найден. Возможно, его удалили."
        case "stopped_by_user":
            "Запуск остановлен вручную."
        case "items_failed":
            "Часть файлов не скопирована."
        case "items_unknown":
            "Результат части файлов неизвестен: QADREN сверит их без повторной записи."
        case "run_active":
            "Сценарий уже выполняется. Второй запуск не начат: текущий запуск продолжается."
        case "plan_stale":
            "Файлы изменились после построения плана. Замена не выполнена. Проверьте план заново."
        case "plan_snapshot_required":
            "Нет метаданных плана для этого файла. Проверьте план заново."
        case "local_scenario_connection_unavailable":
            "Подключение сценария выключено или недоступно. Включите его в разделе «Подключения»."
        case "local_metadata_unavailable":
            "Не удалось прочитать папку. QADREN не открывает папки, в которых есть символические ссылки или специальные файлы, и не следует по ссылкам. Проверьте папку или выберите другую."
        case "a symbolic link or special file has this name; QADREN does not synchronize it":
            "Под этим именем находится символическая ссылка или специальный файл. QADREN такие объекты не синхронизирует, поэтому действий с этим путём нет."
        case "conflict_stale":
            "Файлы изменились после создания задачи конфликта. Обновите снимок конфликта."
        case "this_mac_required":
            "Подключение «Этот Mac» есть всегда, его нельзя удалить. Его можно переименовать или выключить."
        case "waiting_for_scenario":
            "Запуск ждёт окончания другого сценария с пересекающимися папками."
        case "smb_mounted_elsewhere":
            "Сетевая папка уже подключена в macOS в другом месте, например в Finder или Time Machine. macOS подключает одну папку только один раз. Отключите её там или откройте сценарий в QADREN и нажмите «Отключить там и подключить для QADREN»."
        case "smb_login_refused":
            "Нужен вход: сервер не принял логин или пароль сетевого диска. Проверьте их в разделе «Подключения»."
        case "smb_offline":
            "Сервер сетевого диска не отвечает. QADREN повторит подключение позже."
        case "smb_mount_failed":
            "Фоновая служба не смогла подключить сетевой диск. Откройте сценарий в QADREN, чтобы подключить его."
        case "invalid_smb_address":
            "Адрес сетевой папки должен иметь вид smb://сервер/папка. Логин и пароль в адрес не вписывайте: для них есть отдельные поля."
        case "invalid_selection":
            "Выбранный объект находится вне корневой папки сценария. Выберите объекты заново."
        case "the name differs from a selected object only by letter case or Unicode form":
            "Имя отличается от выбранного объекта только регистром букв или формой Unicode. QADREN не выполняет действий с таким путём."
        case "rule_delete_failed":
            "Удаление по правилу сценария не выполнено. Файлы не изменены. Проверьте план заново."
        case "outcome_unknown":
            "Результат операции не подтверждён. Повтор не выполнен. QADREN проверит результат автоматически."
        case "plan_unavailable", "plan_build_failed", "snapshot_store_unavailable":
            "Фоновая служба не построила план. Повторите «Проверить план»; если ошибка повторится, перезапустите QADREN."
        case "snapshot_failed", "snapshot_missing":
            "Чтение одной из сторон сценария остановилось с ошибкой. Повторите «Проверить план»."
        case "local_root_unavailable":
            "Папка сценария недоступна. Проверьте, что диск подключён и папка существует."
        case "yandex_incomplete_metadata":
            "Яндекс Диск вернул неполные метаданные. Обход продолжится автоматически."
        case "invalid_panel_url":
            "Адрес панели должен иметь вид https://сервер:порт/путь, например https://panel.example.com:1500/ispmgr. Логин и пароль в адрес не вписывайте."
        case "unsupported_scenario_endpoint":
            "Панель ISPmanager пока нельзя выбрать в сценарии. Сценарии резервного копирования с неё появятся на следующем этапе."
        case "yandex_source_only":
            "Яндекс Диск пока может быть только источником сценария «В одну сторону», а назначением — этот Mac или сетевой диск."
        case "yandex_deletions_unsupported":
            "Удаления по сценарию с Яндекс Диска пока не выполняются. Файлы на Mac не изменены."
        case "yandex_token_required", "yandex_invalid_token":
            "Нет входа в Яндекс Диск для этого подключения. Войдите в аккаунт в разделе «Подключения»."
        case "yandex_unauthorized":
            "Яндекс отклонил вход. Войдите в аккаунт заново в разделе «Подключения»."
        case "yandex_forbidden":
            "У QADREN нет доступа к этой папке Яндекс Диска."
        case "yandex_rate_limited":
            "Яндекс Диск временно ограничил запросы. Повторите позже."
        case "yandex_not_found":
            "Файл или папка на Яндекс Диске не найдены. Проверьте план заново."
        case "yandex_network_error":
            "Нет связи с Яндекс Диском. Проверьте интернет и повторите."
        case "yandex_content_mismatch":
            "Скачанный файл не совпал с данными Яндекс Диска (размер или MD5). Файл не сохранён."
        case "yandex_content_unavailable":
            "Яндекс Диск сейчас не отдаёт содержимое файла (ошибка сервера хранения). Файл не скачан и остаётся в плане. Повторите позже."
        case "yandex_snapshot_too_large":
            "В папке Яндекс Диска слишком много объектов для одного сценария (больше 10 000). Выберите папку поменьше."
        case "yandex_provider_error", "yandex_invalid_path":
            "Яндекс Диск вернул ошибку. Повторите позже."
        default:
            code
        }
    }
}

/// Yandex Disk to Mac Scenarios (D-023). The Worker needs the OAuth token of
/// the Yandex Disk source for each request. The app reads it from the
/// Keychain and sends it over the private Worker socket; nobody stores it.
enum YandexScenarioAccess {
    /// The Yandex Disk source connection of a one-way Scenario, or nil.
    static func sourceConnectionID(_ scenario: ScenarioEntry, entries: [StorageEntry]? = nil) -> String? {
        let id: String
        switch scenario.direction {
        case "left_to_right": id = scenario.left_connection_id
        case "right_to_left": id = scenario.right_connection_id
        default: return nil
        }
        let entries = entries ?? (try? StorageRegistryBridge.list()) ?? []
        return entries.first { $0.id == id }?.adapter == "yandex_disk" ? id : nil
    }

    /// Give the Worker the credentials of all enabled connections now, every
    /// minute while the app runs, and within 5 s of a Worker restart. A
    /// restart loses them; the Worker reports its start time in `status`, so
    /// a new Worker gets them before a waiting run shows "sign-in required"
    /// for long (D-024 stage A).
    static func startTokenDelivery() {
        Task.detached(priority: .utility) {
            var deliveredTo: Int64?
            var deliveredAt = Date.distantPast
            while true {
                let started = try? WorkerIPC.status().started
                if started != deliveredTo || Date().timeIntervalSince(deliveredAt) >= 60 {
                    // An incomplete delivery (a locked Keychain) repeats
                    // every minute, not every 5 s.
                    _ = WorkerCredentials.deliverAll()
                    deliveredTo = started
                    deliveredAt = Date()
                }
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    static func token(for scenario: ScenarioEntry) throws -> String? {
        guard let id = sourceConnectionID(scenario) else { return nil }
        guard let token = try YandexKeychain.read(id) else {
            throw StorageError.operation(QADRENUserMessage.text(for: "yandex_token_required"))
        }
        return token
    }
}

/// One owner for the app data directory. Registry, JobStore, and the Worker
/// socket live here. `QADREN_DATA_ROOT` moves all three to an absolute test
/// directory, so native E2E runs never touch the user's real state.
enum QADRENDataLocation {
    static let overrideVariable = "QADREN_DATA_ROOT"

    static func baseDirectory(environment: [String: String] = ProcessInfo.processInfo.environment) throws -> URL {
        let base: URL
        if let override = environment[overrideVariable], !override.isEmpty {
            guard override.hasPrefix("/") else {
                throw StorageError.operation("QADREN_DATA_ROOT must be an absolute path")
            }
            base = URL(fileURLWithPath: override, isDirectory: true)
        } else {
            base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("QADREN", isDirectory: true)
        }
        try FileManager.default.createDirectory(
            at: base, withIntermediateDirectories: true,
            attributes: [.posixPermissions: NSNumber(value: 0o700)]
        )
        return base
    }
}

enum StorageRegistryBridge {
    private static var path: String {
        get throws {
            try QADRENDataLocation.baseDirectory().appendingPathComponent("connections.sqlite3").path
        }
    }

    static func databasePath() throws -> String { try path }

    private static func unwrap<T: Decodable>(
        _ pointer: UnsafeMutablePointer<CChar>?, as: T.Type, databasePath: String? = nil
    ) throws -> T {
        guard let pointer else { throw StorageError.operation("Registry unavailable") }
        defer { registryFree(pointer) }
        let message = String(cString: pointer)
        let response = try JSONDecoder().decode(RegistryEnvelope<T>.self, from: Data(message.utf8))
        guard response.ok, let result = response.data else {
            throw StorageError.operation(QADRENUserMessage.text(for: response.error ?? "Registry request failed"))
        }
        let database = try databasePath ?? path
        try? FileManager.default.setAttributes(
            [.posixPermissions: NSNumber(value: 0o600)], ofItemAtPath: database
        )
        return result
    }

    static func list() throws -> [StorageEntry] {
        let database = try path
        return try database.withCString { try unwrap(registryList($0), as: [StorageEntry].self) }
    }

    static func listScenarios() throws -> [ScenarioEntry] {
        let database = try path
        return try database.withCString { try unwrap(scenarioList($0), as: [ScenarioEntry].self) }
    }

    static func createScenario(
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String,
        selectedPaths: [String] = [],
        deletionMode: ScenarioDeletionMode = .paranoid
    ) throws -> String {
        let database = try path
        let scope = try scenarioScopeJSON(selectedPaths: selectedPaths, deletionMode: deletionMode)
        let result: NewConnection = try database.withCString { db in
            try name.withCString { name in
                try leftID.withCString { leftID in
                    try leftRoot.withCString { leftRoot in
                        try rightID.withCString { rightID in
                            try rightRoot.withCString { rightRoot in
                                try direction.withCString { direction in
                                    try scope.withCString { scope in
                                        try unwrap(
                                            scenarioCreate(db, name, leftID, leftRoot, rightID, rightRoot, direction, scope),
                                            as: NewConnection.self
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return result.id
    }

    static func setScenarioEnabled(id: String, enabled: Bool) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(scenarioSetEnabled(db, value, enabled ? 1 : 0), as: Bool.self)
            }
        }
    }

    /// Read only metadata below one enabled Local Scenario endpoint. This does
    /// not open content, calculate a checksum, submit a Job, or change a
    /// Scenario baseline.
    static func localScenarioSnapshot(
        root: String, connectionID: String, databasePath: String? = nil
    ) throws -> [ScenarioSnapshotItem] {
        let database = try databasePath ?? path
        return try database.withCString { database in
            try connectionID.withCString { connectionID in
                try root.withCString { root in
                    try unwrap(
                        localScenarioMetadataSnapshot(database, connectionID, root),
                        as: [ScenarioSnapshotItem].self
                    )
                }
            }
        }
    }

    /// Deliver durable Scenario-copy baselines before a dry-run reads them.
    /// This has no adapter effect and never repeats a file copy.
    static func dispatchPendingScenarioCopyBaselines() throws {
        _ = try dispatchPendingScenarioCopyBaselines(
            registryPath: try path, jobsPath: try JobInventory.paths().database
        )
    }

    /// Test seam for the idempotent JobStore-to-Registry baseline hand-off.
    static func dispatchPendingScenarioCopyBaselines(
        registryPath: String, jobsPath: String
    ) throws -> Int {
        let pointer = jobsPath.withCString { jobs in
            registryPath.withCString { registry in
                dispatchScenarioCopyBaselines(jobs, registry)
            }
        }
        return try unwrap(
            pointer, as: ScenarioBaselineDispatchStatus.self, databasePath: registryPath
        ).dispatched
    }

    /// Produce a read-only plan for the exact stored Scenario revision from
    /// supplied metadata snapshots. No adapter scan or transfer occurs here.
    static func dryRunScenario(
        id: String, revision: UInt64, left: [ScenarioSnapshotItem], right: [ScenarioSnapshotItem],
        databasePath: String? = nil
    ) throws -> [ScenarioPlanItem] {
        let database = try databasePath ?? path
        let encoder = JSONEncoder()
        let leftJSON = String(decoding: try encoder.encode(left), as: UTF8.self)
        let rightJSON = String(decoding: try encoder.encode(right), as: UTF8.self)
        return try database.withCString { database in
            try id.withCString { id in
                try leftJSON.withCString { left in
                    try rightJSON.withCString { right in
                        try unwrap(
                            scenarioDryRun(database, id, revision, left, right),
                            as: [ScenarioPlanItem].self
                        )
                    }
                }
            }
        }
    }

    /// Add metadata-only conflict work to the independent Scenario queue.
    /// It neither creates a Job nor reads or changes file content.
    static func queueScenarioConflict(
        scenario: ScenarioEntry, relativePath: String, kind: String,
        left: ScenarioSnapshotItem?, right: ScenarioSnapshotItem?,
        sourceIsLeft: Bool? = nil, databasePath: String? = nil
    ) throws -> String {
        let database = try databasePath ?? path
        let source = sourceIsLeft.map { $0 ? Int32(1) : Int32(0) } ?? -1
        let encoder = JSONEncoder()
        let leftJSON = String(decoding: try encoder.encode(left), as: UTF8.self)
        let rightJSON = String(decoding: try encoder.encode(right), as: UTF8.self)
        let result: NewConnection = try database.withCString { database in
            try scenario.id.withCString { scenarioID in
                try relativePath.withCString { relativePath in
                    try kind.withCString { kind in
                        try leftJSON.withCString { leftJSON in
                            try rightJSON.withCString { rightJSON in
                                try unwrap(
                                    scenarioConflictQueue(database, scenarioID, scenario.revision, relativePath, kind, source, leftJSON, rightJSON),
                                    as: NewConnection.self
                                )
                            }
                        }
                    }
                }
            }
        }
        return result.id
    }

    /// Read the independent conflict queue for one exact Scenario revision.
    static func listScenarioConflicts(
        scenario: ScenarioEntry, databasePath: String? = nil
    ) throws -> [ScenarioConflictTask] {
        let database = try databasePath ?? path
        return try database.withCString { database in
            try scenario.id.withCString { scenarioID in
                try unwrap(
                    scenarioConflictList(database, scenarioID, scenario.revision),
                    as: [ScenarioConflictTask].self
                )
            }
        }
    }

    /// Store a user decision in the independent conflict queue. The selected
    /// source is mandatory. This method does not execute a transfer or delete.
    static func selectScenarioConflict(
        id: String, sourceIsLeft: Bool, decision: String, databasePath: String? = nil
    ) throws {
        if databasePath == nil {
            try WorkerIPC.updateRegistryControl(WorkerRegistryControlRequest(
                command: "select_conflict_decision", scenario_id: nil, locally_paused: nil,
                conflict_id: id, source_is_left: sourceIsLeft, decision: decision
            ))
            return
        }
        let database = try databasePath ?? path
        let source: Int32 = sourceIsLeft ? 1 : 0
        let _: Bool = try database.withCString { database in
            try id.withCString { id in
                try decision.withCString { decision in
                    try unwrap(
                        scenarioConflictSelect(database, id, source, decision), as: Bool.self
                    )
                }
            }
        }
    }

    static func executeScenarioConflictReplace(id: String) throws {
        _ = try WorkerIPC.executeConflict(
            WorkerConflictRequest(
                command: "execute_conflict_replace", conflict_id: id,
                job_id: "scenario-conflict-replace-" + UUID().uuidString.lowercased(),
                destructive_consent: true
            )
        )
    }

    /// Test seam for an explicitly confirmed destructive conflict replacement.
    /// Production always passes `true` only after the native confirmation alert.
    static func executeScenarioConflictReplace(
        id: String, registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let jobID = "scenario-conflict-replace-" + UUID().uuidString.lowercased()
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in
            id.withCString { id in jobID.withCString { jobID in
                runLocalScenarioConflictReplaceJob(jobs, registry, id, jobID, consent)
            }}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    static func executeScenarioConflictDelete(id: String) throws {
        _ = try WorkerIPC.executeConflict(
            WorkerConflictRequest(
                command: "execute_conflict_delete", conflict_id: id,
                job_id: "scenario-conflict-delete-" + UUID().uuidString.lowercased(),
                destructive_consent: true
            )
        )
    }

    /// Test seam for an explicitly confirmed destructive conflict deletion.
    /// Production always passes `true` only after the native confirmation alert.
    static func executeScenarioConflictDelete(
        id: String, registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let jobID = "scenario-conflict-delete-" + UUID().uuidString.lowercased()
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in
            id.withCString { id in jobID.withCString { jobID in
                runLocalScenarioConflictDeleteJob(jobs, registry, id, jobID, consent)
            }}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    /// Submit one approved non-destructive `create` proposal. The Core reads
    /// only the required source and validates the immutable Scenario revision
    /// and endpoint direction before it creates the durable copy Job.
    static func executeScenarioCreate(
        scenario: ScenarioEntry, item: ScenarioPlanItem
    ) throws {
        let request = try scenarioCreateRequest(scenario: scenario, item: item)
        _ = try WorkerIPC.executeScenarioCreate(request)
    }

    static func scenarioCreateBatch(id: String) throws -> ScenarioCreateBatchReceipt {
        let database = try path
        return try database.withCString { path in
            try id.withCString { id in
                try unwrap(scenarioCreateBatchGet(path, id), as: ScenarioCreateBatchReceipt.self, databasePath: database)
            }
        }
    }

    /// Progress of the unfinished batches of one Scenario revision (D-024).
    static func scenarioCreateBatchSummaries(scenario: ScenarioEntry) throws -> [ScenarioBatchSummary] {
        let database = try path
        return try database.withCString { path in
            try scenario.id.withCString { id in
                try unwrap(scenarioCreateBatchSummariesFFI(path, id, scenario.revision), as: [ScenarioBatchSummary].self, databasePath: database)
            }
        }
    }

    static func scenarioCreateBatchSummary(id: String) throws -> ScenarioBatchSummary {
        let database = try path
        return try database.withCString { path in
            try id.withCString { id in
                try unwrap(scenarioCreateBatchSummaryFFI(path, id), as: ScenarioBatchSummary.self, databasePath: database)
            }
        }
    }

    /// D-024: after the retry schedule, retry a temporarily unavailable file
    /// of this Scenario every 24 h until it succeeds or the user stops the
    /// batch. The revision does not change, so a running batch continues.
    static func setScenarioRetryUntilSuccess(id: String, enabled: Bool) throws {
        struct Result: Decodable { let retry_until_success: Bool }
        let database = try path
        let _: Result = try database.withCString { db in
            try id.withCString { id in
                try unwrap(scenarioSetRetryUntilSuccessFFI(db, id, enabled ? 1 : 0), as: Result.self, databasePath: database)
            }
        }
    }

    /// Pause or resume a Worker-owned batch (D-024). The current file ends.
    static func setScenarioCreateBatchPaused(id: String, paused: Bool) throws {
        struct Status: Decodable { let paused: Bool }
        let database = try path
        let _: Status = try database.withCString { path in
            try id.withCString { id in
                try unwrap(setScenarioCreateBatchPausedFFI(path, id, paused ? 1 : 0), as: Status.self, databasePath: database)
            }
        }
    }

    /// Saves a durable cancellation request in the Registry. The Worker reads it
    /// before the next file, so the request works while the Worker is busy.
    static func requestScenarioCreateBatchCancellation(id: String) throws {
        struct Status: Decodable { let status: String }
        let database = try path
        let result: Status = try database.withCString { path in
            try id.withCString { id in
                try unwrap(requestScenarioCreateBatchCancellationFFI(path, id), as: Status.self, databasePath: database)
            }
        }
        guard result.status == "cancellation_requested" else {
            throw StorageError.operation("Cancellation was not saved")
        }
    }

    static func pendingScenarioCreateBatches(
        scenario: ScenarioEntry, databasePath: String? = nil
    ) throws -> [ScenarioCreateBatchReceipt] {
        let database = try databasePath ?? path
        return try database.withCString { path in
            try scenario.id.withCString { id in
                try unwrap(
                    pendingScenarioCreateBatchesFFI(path, id, scenario.revision),
                    as: [ScenarioCreateBatchReceipt].self,
                    databasePath: database
                )
            }
        }
    }

    /// Test seam for the Rust-owned execution bridge. Production uses the
    /// app-owned Registry and JobStore paths above.
    static func executeScenarioCreate(
        scenario: ScenarioEntry, item: ScenarioPlanItem,
        registryPath: String, jobsPath: String
    ) throws {
        let request = try scenarioCreateRequest(scenario: scenario, item: item)
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in scenario.id.withCString { scenarioID in
            request.source_root.withCString { sourceRoot in request.source_connection_id.withCString { sourceConnection in request.source_relative_path.withCString { sourceRelative in
                request.destination_root.withCString { destinationRoot in request.destination_connection_id.withCString { destinationConnection in request.destination_relative_path.withCString { destinationRelative in request.job_id.withCString { jobID in
                    runLocalScenarioCopyJob(jobs, registry, scenarioID, scenario.revision, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, jobID)
                }}}}}}}}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    private static func scenarioCreateRequest(
        scenario: ScenarioEntry, item: ScenarioPlanItem
    ) throws -> WorkerScenarioCreateRequest {
        guard item.action.kind == "create", let fromLeft = item.action.from_left else {
            throw StorageError.operation("План не разрешает создание этого файла")
        }
        return WorkerScenarioCreateRequest(
            scenario_id: scenario.id,
            scenario_revision: scenario.revision,
            source_root: fromLeft ? scenario.left_root : scenario.right_root,
            source_connection_id: fromLeft ? scenario.left_connection_id : scenario.right_connection_id,
            source_relative_path: item.relative_path,
            destination_root: fromLeft ? scenario.right_root : scenario.left_root,
            destination_connection_id: fromLeft ? scenario.right_connection_id : scenario.left_connection_id,
            destination_relative_path: item.relative_path,
            job_id: "scenario-copy-" + UUID().uuidString.lowercased(),
            access_token: try YandexScenarioAccess.token(for: scenario)
        )
    }

    /// Submit one `update` proposal after explicit replacement confirmation.
    /// Rust binds the replacement to this immutable Scenario revision.
    /// Sends a confirmed Update with the plan metadata of both files. The
    /// Worker refuses it if either file changed after planning.
    static func executeScenarioUpdate(
        scenario: ScenarioEntry, item: ScenarioPlanItem,
        plannedSource: ScenarioSnapshotItem?, plannedDestination: ScenarioSnapshotItem?
    ) throws {
        guard let plannedSource, let plannedDestination else {
            throw StorageError.operation("plan_snapshot_required")
        }
        var request = try scenarioReplaceRequest(scenario: scenario, item: item, consent: true)
        request.expected_source = WorkerExpectedFileVersion(
            object_revision: plannedSource.object_revision, size: plannedSource.size
        )
        request.expected_destination = WorkerExpectedFileVersion(
            object_revision: plannedDestination.object_revision, size: plannedDestination.size
        )
        _ = try WorkerIPC.executeScenarioReplace(request)
    }

    /// Test seam for a destructive Scenario update. Production passes `true`
    /// only after the native replacement confirmation alert.
    static func executeScenarioUpdate(
        scenario: ScenarioEntry, item: ScenarioPlanItem,
        registryPath: String, jobsPath: String, consent: Bool
    ) throws {
        let request = try scenarioReplaceRequest(scenario: scenario, item: item, consent: consent)
        let pointer = jobsPath.withCString { jobs in registryPath.withCString { registry in scenario.id.withCString { scenarioID in
            request.source_root.withCString { sourceRoot in request.source_connection_id.withCString { sourceConnection in request.source_relative_path.withCString { sourceRelative in
                request.destination_root.withCString { destinationRoot in request.destination_connection_id.withCString { destinationConnection in request.destination_relative_path.withCString { destinationRelative in request.job_id.withCString { jobID in
                    runLocalScenarioReplaceJob(jobs, registry, scenarioID, scenario.revision, sourceRoot, sourceConnection, sourceRelative, destinationRoot, destinationConnection, destinationRelative, jobID, request.replacement_consent)
                }}}}}}}}
        }}
        let _: ScenarioJobStatus = try unwrap(
            pointer, as: ScenarioJobStatus.self, databasePath: registryPath
        )
    }

    /// Metadata of a Yandex Disk Scenario root for the plan (D-023). The
    /// token is read from the Keychain for this call only.
    static func yandexScenarioSnapshot(
        root: String, connectionID: String, selection: [String]
    ) throws -> [ScenarioSnapshotItem] {
        guard let token = try YandexKeychain.read(connectionID) else {
            throw StorageError.operation(QADRENUserMessage.text(for: "yandex_token_required"))
        }
        return try DiskMetadataBridge.scenarioSnapshot(
            registryPath: try path, connectionID: connectionID, root: root, selection: selection,
            accessValue: token
        )
    }

    private static func scenarioReplaceRequest(
        scenario: ScenarioEntry, item: ScenarioPlanItem, consent: Bool
    ) throws -> WorkerScenarioReplaceRequest {
        guard item.action.kind == "update", let fromLeft = item.action.from_left else {
            throw StorageError.operation("План не разрешает обновление этого файла")
        }
        return WorkerScenarioReplaceRequest(
            scenario_id: scenario.id,
            scenario_revision: scenario.revision,
            source_root: fromLeft ? scenario.left_root : scenario.right_root,
            source_connection_id: fromLeft ? scenario.left_connection_id : scenario.right_connection_id,
            source_relative_path: item.relative_path,
            destination_root: fromLeft ? scenario.right_root : scenario.left_root,
            destination_connection_id: fromLeft ? scenario.right_connection_id : scenario.left_connection_id,
            destination_relative_path: item.relative_path,
            job_id: "scenario-replace-" + UUID().uuidString.lowercased(),
            replacement_consent: consent,
            access_token: try YandexScenarioAccess.token(for: scenario)
        )
    }

    static func listScenarioSchedules(scenarioID: String) throws -> [ScenarioScheduleEntry] {
        let database = try path
        return try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(scenarioSchedulesList(db, id), as: [ScenarioScheduleEntry].self)
            }
        }
    }

    /// Read the one-time confirmation of "the source always wins" (D-025),
    /// or record it when `confirm` is true.
    static func scenarioSourceWins(scenario: ScenarioEntry, confirm: Bool) throws -> Bool {
        struct Answer: Decodable { let confirmed: Bool }
        let database = try path
        return try database.withCString { db in
            try scenario.id.withCString { id in
                try unwrap(
                    scenarioSourceWinsCall(db, id, scenario.revision, confirm ? 1 : 0), as: Answer.self
                ).confirmed
            }
        }
    }

    /// Names of enabled Scenarios whose runs cannot execute at the same time
    /// as runs of these roots (D-025 section 11).
    static func scenarioRootOverlaps(
        scenarioID: String?, leftID: String, leftRoot: String,
        rightID: String, rightRoot: String, direction: String
    ) throws -> [String] {
        let database = try path
        let values = [scenarioID ?? "", leftID, leftRoot, rightID, rightRoot, direction]
        let strings = values.map { strdup($0) }
        defer { strings.forEach { free($0) } }
        return try database.withCString { db in
            try unwrap(
                scenarioRootOverlapsGet(db, strings[0], strings[1], strings[2], strings[3], strings[4], strings[5]),
                as: [String].self
            )
        }
    }

    static func scenarioRunState(scenarioID: String) throws -> ScenarioRunState {
        let database = try path
        return try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(scenarioRunStateGet(db, id), as: ScenarioRunState.self)
            }
        }
    }

    static func scenarioAutomation(
        scenarioID: String, databasePath: String? = nil
    ) throws -> ScenarioAutomationEntry {
        let database = try databasePath ?? path
        return try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(scenarioAutomationGet(db, id), as: ScenarioAutomationEntry.self)
            }
        }
    }

    static func setScenarioLocallyPaused(
        scenarioID: String, paused: Bool, databasePath: String? = nil
    ) throws {
        if databasePath == nil {
            try WorkerIPC.updateRegistryControl(WorkerRegistryControlRequest(
                command: "set_scenario_paused", scenario_id: scenarioID, locally_paused: paused,
                conflict_id: nil, source_is_left: nil, decision: nil
            ))
            return
        }
        let database = try databasePath ?? path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { id in
                try unwrap(
                    scenarioAutomationSetPaused(db, id, paused ? 1 : 0), as: Bool.self
                )
            }
        }
    }

    /// Returns scheduler candidates only. The Core deliberately does not claim
    /// these slots or create Jobs until it has an atomic JobStore/outbox boundary.
    static func dueScenarioSchedules(
        after: Date, through: Date, databasePath: String? = nil
    ) throws -> [ScenarioScheduleOccurrence] {
        let database = try databasePath ?? path
        let formatter = ISO8601DateFormatter()
        let afterValue = formatter.string(from: after)
        let throughValue = formatter.string(from: through)
        return try database.withCString { db in
            try afterValue.withCString { lowerBound in
                try throughValue.withCString { upperBound in
                    try unwrap(
                        scenarioSchedulesDue(db, lowerBound, upperBound),
                        as: [ScenarioScheduleOccurrence].self
                    )
                }
            }
        }
    }

    /// Stage and dispatch due schedule receipts into the existing JobStore.
    /// This creates only revision-pinned queued Jobs. It does not read adapter
    /// metadata or content, execute a Scenario, or grant transfer consent.
    static func dispatchDueScenarioSchedules(
        after: Date, through: Date, databasePath: String? = nil, jobStorePath: String? = nil
    ) throws -> (staged: Int, dispatched: Int) {
        let database = try databasePath ?? path
        let jobs = try jobStorePath ?? JobInventory.paths().database
        let formatter = ISO8601DateFormatter()
        let afterValue = formatter.string(from: after)
        let throughValue = formatter.string(from: through)
        let result: ScenarioScheduleDispatchStatus = try database.withCString { database in
            try jobs.withCString { jobs in
                try afterValue.withCString { lowerBound in
                    try throughValue.withCString { upperBound in
                        try unwrap(
                            scenarioSchedulesDispatch(database, jobs, lowerBound, upperBound),
                            as: ScenarioScheduleDispatchStatus.self
                        )
                    }
                }
            }
        }
        return (result.staged, result.dispatched)
    }

    static func createScenarioSchedule(
        scenarioID: String, timezone: String, intervalHours: UInt8, daysMask: UInt8,
        hour: UInt8, minute: UInt8
    ) throws {
        let database = try path
        let _: NewConnection = try database.withCString { db in
            try scenarioID.withCString { id in
                try timezone.withCString { timezone in
                    try unwrap(
                        scenarioSchedulesCreate(db, id, timezone, intervalHours, daysMask, hour, minute),
                        as: NewConnection.self
                    )
                }
            }
        }
    }

    static func setScenarioScheduleEnabled(
        scenarioID: String, scheduleID: String, enabled: Bool
    ) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { scenario in
                try scheduleID.withCString { schedule in
                    try unwrap(
                        scenarioSchedulesSetEnabled(db, scenario, schedule, enabled ? 1 : 0),
                        as: Bool.self
                    )
                }
            }
        }
    }

    static func removeScenarioSchedule(scenarioID: String, scheduleID: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try scenarioID.withCString { scenario in
                try scheduleID.withCString { schedule in
                    try unwrap(scenarioSchedulesRemove(db, scenario, schedule), as: Bool.self)
                }
            }
        }
    }

    static func updateScenario(
        id: String,
        name: String,
        leftID: String,
        leftRoot: String,
        rightID: String,
        rightRoot: String,
        direction: String,
        selectedPaths: [String] = [],
        deletionMode: ScenarioDeletionMode = .paranoid
    ) throws {
        let database = try path
        let scope = try scenarioScopeJSON(selectedPaths: selectedPaths, deletionMode: deletionMode)
        let _: Bool = try database.withCString { db in
            try id.withCString { id in
                try name.withCString { name in
                    try leftID.withCString { leftID in
                        try leftRoot.withCString { leftRoot in
                            try rightID.withCString { rightID in
                                try rightRoot.withCString { rightRoot in
                                    try direction.withCString { direction in
                                        try scope.withCString { scope in
                                            try unwrap(
                                                scenarioUpdate(
                                                    db, id, name, leftID, leftRoot, rightID, rightRoot, direction, scope
                                                ),
                                                as: Bool.self
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    static func addYandex(name: String) throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try "yandex_disk".withCString { adapter in
                try name.withCString { label in
                    try unwrap(registryAdd(db, adapter, label), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    /// Adds one SMB share connection (D-021). Core validates the address and
    /// refuses an address with a login or a password.
    static func addSMBShare(name: String, address: String) throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try name.withCString { label in
                try address.withCString { url in
                    try unwrap(registryAddSMBShare(db, label, url), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    static func setSMBEndpoint(id: String, address: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try address.withCString { url in
                    try unwrap(registrySetSMBEndpoint(db, value, url), as: Bool.self)
                }
            }
        }
    }

    /// One ISPmanager panel connection (D-027). The URL has no credentials.
    static func addISPmanager(name: String, url: String) throws -> String {
        let database = try path
        let result: NewConnection = try database.withCString { db in
            try name.withCString { label in
                try url.withCString { value in
                    try unwrap(registryAddISPmanager(db, label, value), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    static func setISPmanagerEndpoint(id: String, url: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try url.withCString { endpoint in
                    try unwrap(registrySetISPmanagerEndpoint(db, value, endpoint), as: Bool.self)
                }
            }
        }
    }

    static func ispmanagerSettings(id: String) throws -> ISPmanagerSettings {
        let database = try path
        return try database.withCString { db in
            try id.withCString { value in
                try unwrap(registryISPmanagerSettings(db, value), as: ISPmanagerSettings.self)
            }
        }
    }

    static func setISPmanagerSSHPort(id: String, port: UInt16) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registrySetISPmanagerSSHPort(db, value, port), as: Bool.self)
            }
        }
    }

    static func confirmISPmanagerHostKey(id: String, fingerprint: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try fingerprint.withCString { key in
                    try unwrap(registryConfirmISPmanagerHostKey(db, value, key), as: Bool.self)
                }
            }
        }
    }

    /// The single "This Mac" connection (D-020): the existing record, or a new
    /// one when none exists. The product uses only this path.
    static func ensureLocalComputer(name: String = "Этот Mac", databasePath: String? = nil) throws -> String {
        let database = try databasePath ?? path
        let result: NewConnection = try database.withCString { db in
            try name.withCString { label in
                try unwrap(registryEnsureLocalComputer(db, label), as: NewConnection.self)
            }
        }
        return result.id
    }

    /// Adds a new Local Filesystem record without the single-record rule.
    /// Tests use it to model two independent local endpoints; the product
    /// calls `ensureLocalComputer`.
    static func addLocalComputer(name: String = "Этот Mac", databasePath: String? = nil) throws -> String {
        let database = try databasePath ?? path
        let result: NewConnection = try database.withCString { db in
            try "local_filesystem".withCString { adapter in
                try name.withCString { label in
                    try unwrap(registryAdd(db, adapter, label), as: NewConnection.self)
                }
            }
        }
        return result.id
    }

    /// Final local record deletion for the recoverable removal workflow. Rust
    /// rechecks that the record is disabled and has no unresolved Jobs.
    /// Removes a Scenario and its local records. Files do not change.
    static func removeScenario(id: String) throws {
        let database = try path
        let jobsDatabase = try JobInventory.paths().database
        let _: Bool = try database.withCString { db in
            try jobsDatabase.withCString { jobs in
                try id.withCString { value in
                    try unwrap(
                        registryRemoveScenarioAfterJobGate(db, jobs, value),
                        as: Bool.self,
                        databasePath: database
                    )
                }
            }
        }
    }

    static func removeAfterJobGate(
        id: String, jobsDatabasePath: String, databasePath: String? = nil
    ) throws {
        let database = try databasePath ?? path
        let _: Bool = try database.withCString { db in
            try jobsDatabasePath.withCString { jobDatabase in
                try id.withCString { value in
                    try unwrap(
                        registryRemoveAfterJobGate(db, jobDatabase, value),
                        as: Bool.self,
                        databasePath: database
                    )
                }
            }
        }
    }

    static func rename(id: String, name: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try name.withCString { newName in
                    try unwrap(registryRename(db, value, newName), as: Bool.self)
                }
            }
        }
    }

    static func setEnabled(id: String, enabled: Bool) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registrySetEnabled(db, value, enabled ? 1 : 0), as: Bool.self)
            }
        }
    }

    /// Final local Registry update after the matching Keychain item was
    /// removed. This never contacts a provider or removes remote objects.
    static func finalizeDisconnect(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registryFinalizeDisconnect(db, value), as: Bool.self)
            }
        }
    }

    static func setCredential(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try ("keychain:" + id).withCString { reference in
                    try unwrap(registrySetCredential(db, value, reference), as: Bool.self)
                }
            }
        }
    }

    static func clearCredential(id: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try unwrap(registrySetCredential(db, value, nil), as: Bool.self)
            }
        }
    }

    static func setStatus(id: String, status: String) throws {
        let database = try path
        let timestamp = Int64(Date().timeIntervalSince1970)
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try status.withCString { state in
                    try unwrap(registrySetStatus(db, value, state, timestamp), as: Bool.self)
                }
            }
        }
    }

    static func setProviderAccount(id: String, providerID: String) throws {
        let database = try path
        let _: Bool = try database.withCString { db in
            try id.withCString { value in
                try providerID.withCString { provider in
                    try unwrap(registrySetAccount(db, value, provider), as: Bool.self)
                }
            }
        }
    }
}

/// Keychain records are isolated by StorageConnection ID; no token reaches SQLite.
enum YandexKeychain {
    private static let service = "dev.qadren.yandex.oauth"

    private static func query(_ connectionID: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: connectionID
        ]
    }

    static func read(_ connectionID: String) throws -> String? {
        var q = query(connectionID)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data,
              let token = String(data: data, encoding: .utf8) else {
            throw StorageError.operation("Keychain read failed")
        }
        return token
    }

    static func store(_ token: String, for connectionID: String) throws {
        guard !token.isEmpty && !connectionID.isEmpty else {
            throw StorageError.operation("Empty credentials")
        }
        let key = query(connectionID)
        var item = key
        item[kSecValueData as String] = Data(token.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        var result = SecItemAdd(item as CFDictionary, nil)
        if result == errSecDuplicateItem {
            result = SecItemUpdate(key as CFDictionary,
                                   [kSecValueData as String: Data(token.utf8)] as CFDictionary)
        }
        guard result == errSecSuccess else {
            throw StorageError.operation("Keychain write failed")
        }
    }

    static func delete(_ connectionID: String) throws {
        let result = SecItemDelete(query(connectionID) as CFDictionary)
        guard result == errSecSuccess || result == errSecItemNotFound else {
            throw StorageError.operation("Keychain removal failed")
        }
    }
}
