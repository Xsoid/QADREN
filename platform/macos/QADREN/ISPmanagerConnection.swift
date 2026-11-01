import Foundation

// ISPmanager 6 panel connection (D-027, proposed): a read-only backup
// source. This stage signs in and lists capabilities; no Scenario uses it yet.

@_silgen_name("qadren_ispmanager_check")
private func ispmanagerCheck(
    _ url: UnsafePointer<CChar>?, _ login: UnsafePointer<CChar>?, _ password: UnsafePointer<CChar>?,
    _ sshPort: UInt16, _ hostKey: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?
@_silgen_name("qadren_ispmanager_free_string")
private func ispmanagerFree(_ pointer: UnsafeMutablePointer<CChar>?)

let ISPmanagerKeychain = LoginKeychain(service: "dev.qadren.ispmanager.credentials")

/// The panel URL form that Core stores: `https://host[:port][/path]`. Core
/// removes a fragment, such as `#/` in the address that a browser shows.
enum ISPmanagerAddress {
    static func isValid(_ text: String) -> Bool {
        guard let components = URLComponents(string: text.trimmingCharacters(in: .whitespaces)) else {
            return false
        }
        return components.scheme == "https" && !(components.host ?? "").isEmpty
            && components.user == nil && components.password == nil
            && components.query == nil
    }
}

struct ISPmanagerFunctionReport: Codable, Equatable {
    let function: String
    /// `items`, `empty`, `denied`, `missing`, or `error`.
    let result: String
    let error: String?
}

struct ISPmanagerCategoryReport: Codable, Equatable, Identifiable {
    var id: String { category }
    let category: String
    /// `verified_metadata_only`, `permission_denied`, `unsupported`, or
    /// `unverified`.
    let state: String
    let functions: [ISPmanagerFunctionReport]
}

/// SFTP settings of one connection (D-027 section 7a). The fingerprint is
/// not a secret; the Registry keeps it.
struct ISPmanagerSettings: Decodable, Equatable {
    var ssh_port: UInt16
    var host_key_sha256: String?
    /// The report of the last connection check, kept by the Registry.
    var last_check: ISPmanagerCheckReport?
    var last_check_at: Int64?

    static let defaults = ISPmanagerSettings(ssh_port: 22, host_key_sha256: nil)
}

/// The file transport that a check confirmed: `sftp`, or `panel_api` with
/// the reason why SFTP is not used.
struct ISPmanagerFileTransport: Codable, Equatable {
    let kind: String
    let reason: String
    let fingerprint: String?
    let detail: String?

    var isSFTP: Bool { kind == "sftp" }

    func message(port: UInt16) -> String {
        let fallback = " Файлы будут браться через API панели."
        let detail = detail.map { " (\($0))" } ?? ""
        switch reason {
        case "ready":
            return "Файлы: SFTP, порт \(port). Ключ сервера подтверждён, вход по SSH выполнен, команды на сервере не запускаются."
        case "host_key_unconfirmed":
            return "SSH-сервер на порту \(port) показал ключ \(fingerprint ?? "?"). Сверьте отпечаток с ключом сервера и подтвердите его. До подтверждения QADREN не отправляет логин и пароль по SSH." + fallback
        case "host_key_changed":
            return "Ключ SSH-сервера изменился: теперь \(fingerprint ?? "?"). SFTP остановлен. Если ключ сменили вы, подтвердите новый; иначе это может быть подмена сервера." + fallback
        case "host_certificate_unsupported":
            return "SSH-сервер использует сертификат хоста, QADREN его пока не принимает." + fallback
        case "auth_refused":
            return "SSH-сервер не принял логин и пароль: возможно, у аккаунта нет SSH-доступа или для SSH другой пароль." + fallback
        case "unreachable":
            return "SSH-сервер не отвечает на порту \(port)\(detail)." + fallback
        default:
            return "Вход по SSH выполнен, но SFTP недоступен\(detail)." + fallback
        }
    }
}

/// The result of one sign-in and capability check. `status` is a Registry
/// status. The report holds no values from the panel, only result classes.
struct ISPmanagerCheckReport: Codable, Equatable {
    let status: String
    let error: String?
    let categories: [ISPmanagerCategoryReport]
    let file_transport: ISPmanagerFileTransport?

    var message: String {
        switch status {
        case "connected":
            "Вход выполнен. Ниже — что этот аккаунт может прочитать на панели. На сервере ничего не изменено."
        case "requires_auth":
            "Панель не приняла логин или пароль. Проверьте их и повторите."
        case "offline":
            "Панель не отвечает или её сертификат не проходит проверку. QADREN не принимает недоверенные сертификаты." + detail
        default:
            "Не удалось проверить панель." + detail
        }
    }

    private var detail: String { error.map { " (\($0))" } ?? "" }
}

private struct ISPmanagerCheckResult: Decodable {
    let ok: Bool
    let data: ISPmanagerCheckReport?
    let error: String?
}

enum ISPmanagerCheck {
    /// Signs in and reads the lists of every category, off the main thread.
    /// The password goes to Core in memory only.
    static func run(
        url: String, credentials: LoginCredentials, settings: ISPmanagerSettings
    ) async -> ISPmanagerCheckReport {
        await Task.detached(priority: .userInitiated) {
            let pointer = url.withCString { url in
                credentials.login.withCString { login in
                    credentials.password.withCString { password in
                        (settings.host_key_sha256 ?? "").withCString { hostKey in
                            ispmanagerCheck(url, login, password, settings.ssh_port, hostKey)
                        }
                    }
                }
            }
            let failure = ISPmanagerCheckReport(
                status: "error", error: "no response from Core", categories: [], file_transport: nil
            )
            guard let pointer else { return failure }
            defer { ispmanagerFree(pointer) }
            guard let result = try? JSONDecoder().decode(
                ISPmanagerCheckResult.self, from: Data(String(cString: pointer).utf8)
            ), result.ok, let report = result.data else { return failure }
            return report
        }.value
    }
}

/// Russian names of the D-027 resource categories, in the adapter contract order.
enum ISPmanagerCategory {
    static func title(_ key: String) -> String {
        switch key {
        case "files": "Файлы и папки"
        case "websites": "Сайты и WWW-домены"
        case "databases": "Базы данных"
        case "mail_domains": "Почтовые домены"
        case "mailboxes": "Почтовые ящики"
        case "cron": "Планировщик (Cron)"
        case "dns": "DNS"
        case "ssl": "SSL-сертификаты"
        case "ftp": "FTP"
        case "accounts": "Пользователи панели"
        case "ssh": "SSH"
        case "web_server": "Веб-серверы и службы"
        case "php": "PHP"
        case "mail_service": "Почтовый сервер"
        case "network": "Сеть и безопасность"
        case "docker": "Docker"
        case "panel_settings": "Настройки панели и бэкапов"
        case "logs": "Журналы и статистика"
        default: key
        }
    }

    static func state(_ report: ISPmanagerCategoryReport) -> (kind: QDStatusKind, text: String) {
        switch report.state {
        case "verified_metadata_only":
            (.ready, "Список доступен; экспорт ещё не проверен")
        case "permission_denied":
            (.needsAction, "Нет прав у этого аккаунта")
        case "unsupported":
            (.offline, "Нет на этой панели")
        default:
            if !report.functions.isEmpty {
                (.unknown, "Не проверено")
            } else if report.category == "ssh" {
                // The API has no SSH key function; the keys are files (D-027).
                (.unknown, "В API нет функции; ключи — через файлы ~/.ssh")
            } else {
                (.unknown, "В API нет функции; только через файлы конфигурации")
            }
        }
    }
}
