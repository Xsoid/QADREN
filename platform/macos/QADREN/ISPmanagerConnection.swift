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

@_silgen_name("qadren_ispmanager_databases")
private func ispmanagerDatabases(
    _ url: UnsafePointer<CChar>?, _ login: UnsafePointer<CChar>?, _ password: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("qadren_ispmanager_database_tables")
private func ispmanagerDatabaseTables(
    _ url: UnsafePointer<CChar>?, _ login: UnsafePointer<CChar>?, _ password: UnsafePointer<CChar>?,
    _ sshPort: UInt16, _ hostKey: UnsafePointer<CChar>?, _ database: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("qadren_ispmanager_sftp_list")
private func ispmanagerSFTPList(
    _ url: UnsafePointer<CChar>?, _ login: UnsafePointer<CChar>?, _ password: UnsafePointer<CChar>?,
    _ sshPort: UInt16, _ hostKey: UnsafePointer<CChar>?, _ path: UnsafePointer<CChar>?
) -> UnsafeMutablePointer<CChar>?

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
            return "Файлы: SFTP, порт \(port). Ключ сервера подтверждён, вход по SSH выполнен. На сервере запускаются только команды чтения: md5sum и проверка версий утилит дампа."
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

/// What the panel and the SSH server tell about databases (D-027 section
/// 7b): field names, server type and version, tool versions, and whether
/// the panel returns a password. It holds no password and no names.
struct ISPmanagerDatabaseProbe: Codable, Equatable {
    struct ServerField: Codable, Equatable {
        let field: String
        let value: String
    }
    struct ToolVersion: Codable, Equatable {
        let tool: String
        let version: String?
    }
    let db_fields: [String]
    let db_key_field: String?
    let server_info: [ServerField]
    let users_result: String?
    let user_fields: [String]
    let user_key_field: String?
    let user_form_result: String?
    let user_form_fields: [String]
    let password_readable: Bool?
    let password_masked: Bool?
    let mysql_login: String?
    let tools: [ToolVersion]
    let error: String?

    var lines: [String] {
        if db_fields.isEmpty {
            return [error.map { "Список баз не прочитан (\($0))." } ?? "Баз данных у аккаунта нет."]
        }
        var result: [String] = []
        let server = server_info.map { "\($0.field): \($0.value)" }.joined(separator: ", ")
        result.append("Базы: поля списка — \(db_fields.joined(separator: ", "))." + (server.isEmpty ? "" : " Сервер: \(server)."))
        switch users_result {
        case "items": result.append("Пользователи базы читаются: \(user_fields.joined(separator: ", ")).")
        case "empty": result.append("У первой базы нет пользователей.")
        case .some(let other): result.append("Пользователи базы не читаются: \(Self.label(other)).")
        case nil: break
        }
        switch (user_form_result, password_readable) {
        case ("items", true?) where password_masked == true:
            result.append("Форма пользователя базы отдаёт только маску вместо пароля.")
        case ("items", true?): result.append("Панель отдаёт пароль пользователя базы.")
        case ("items", _): result.append("Форма пользователя базы читается, но поле пароля пустое: панель пароль не отдаёт. Поля: \(user_form_fields.joined(separator: ", ")).")
        case (.some(let other), _): result.append("Форма пользователя базы не читается: \(Self.label(other)).")
        default: break
        }
        if !tools.isEmpty {
            let found = tools.compactMap { tool in tool.version.map { "\(tool.tool): \($0)" } }
            result.append(found.isEmpty ? "Утилит дампа по SSH не найдено." : "По SSH: " + found.joined(separator: "; ") + ".")
        }
        switch mysql_login {
        case "ok": result.append("Вход в MySQL с данными из панели работает: mysqldump прочитал структуру первой базы. Данные не читались, база не изменялась.")
        case "refused": result.append("MySQL не принял пользователя или пароль из панели.")
        case "error": result.append("Проверочный вход в MySQL по SSH не удался.")
        default: break
        }
        return result
    }

    private static func label(_ result: String) -> String {
        switch result {
        case "denied": "нет прав"
        case "missing": "функции нет на панели"
        default: "ошибка"
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
    var databases: ISPmanagerDatabaseProbe? = nil

    /// The Registry status. Backups read files through SFTP with the
    /// confirmed host key, so an unconfirmed or changed key needs the user
    /// even when the panel sign-in works.
    var effectiveStatus: String {
        let keyProblem = ["host_key_unconfirmed", "host_key_changed"].contains(file_transport?.reason ?? "")
        return status == "connected" && keyProblem ? "requires_auth" : status
    }

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
            (.unsupported, "Нет на этой панели")
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

/// One entry of an SFTP folder listing. A link is not followed.
struct ISPmanagerDirEntry: Decodable, Identifiable, Equatable {
    var id: String { name }
    let name: String
    /// `dir`, `file`, `symlink`, or `other`.
    let kind: String
    let size: UInt64?
    let modified: UInt32?
    let mode: UInt32?
}

struct ISPmanagerDirListing: Decodable, Equatable {
    let path: String
    let parent: String?
    let home: String
    let entries: [ISPmanagerDirEntry]
    let truncated: Bool
}

private struct ISPmanagerListResult: Decodable {
    let ok: Bool
    let data: ISPmanagerDirListing?
    let error: String?
    let transport: ISPmanagerFileTransport?
}

enum ISPmanagerSFTP {
    enum Failure: Error, Equatable {
        case transport(ISPmanagerFileTransport)
        case message(String)
    }

    /// Lists one folder off the main thread. `path` nil is the home folder.
    static func list(
        url: String, credentials: LoginCredentials, settings: ISPmanagerSettings, path: String?
    ) async -> Result<ISPmanagerDirListing, Failure> {
        guard let hostKey = settings.host_key_sha256 else {
            return .failure(.message("Ключ SSH-сервера не подтверждён. Подтвердите его на карточке подключения в разделе «Подключения»."))
        }
        return await Task.detached(priority: .userInitiated) {
            let pointer = url.withCString { url in
                credentials.login.withCString { login in
                    credentials.password.withCString { password in
                        hostKey.withCString { key in
                            (path ?? "").withCString { path in
                                ispmanagerSFTPList(url, login, password, settings.ssh_port, key, path)
                            }
                        }
                    }
                }
            }
            guard let pointer else { return .failure(.message("Нет ответа от Core.")) }
            defer { ispmanagerFree(pointer) }
            guard let result = try? JSONDecoder().decode(
                ISPmanagerListResult.self, from: Data(String(cString: pointer).utf8)
            ) else { return .failure(.message("Неверный ответ Core.")) }
            if let listing = result.data, result.ok { return .success(listing) }
            if let transport = result.transport { return .failure(.transport(transport)) }
            return .failure(.message(result.error ?? "Не удалось прочитать папку."))
        }.value
    }
}

/// The MySQL databases of the account for the Scenario editor (D-027
/// section 7b). Core reads only the `db` list of the panel.
enum ISPmanagerDatabases {
    private struct Result: Decodable {
        let ok: Bool
        let data: [String]?
        let error: String?
    }

    static func list(url: String, credentials: LoginCredentials) async -> Swift.Result<[String], ISPmanagerSFTP.Failure> {
        await Task.detached(priority: .userInitiated) {
            let pointer = url.withCString { url in
                credentials.login.withCString { login in
                    credentials.password.withCString { password in
                        ispmanagerDatabases(url, login, password)
                    }
                }
            }
            guard let pointer else { return .failure(.message("Нет ответа от Core.")) }
            defer { ispmanagerFree(pointer) }
            guard let result = try? JSONDecoder().decode(
                Result.self, from: Data(String(cString: pointer).utf8)
            ) else { return .failure(.message("Неверный ответ Core.")) }
            if let names = result.data, result.ok { return .success(names) }
            return .failure(.message("Не удалось прочитать список баз панели (\(result.error ?? "ошибка"))."))
        }.value
    }
    private struct TablesResult: Decodable {
        let ok: Bool
        let data: [ISPmanagerTable]?
        let error: String?
        let transport: ISPmanagerFileTransport?
    }

    /// The base tables of one database with their estimated sizes (D-027
    /// section 7b.10). Core reads the access facts from the panel and runs
    /// one read-only `information_schema` query over SSH.
    static func tables(
        url: String, credentials: LoginCredentials, settings: ISPmanagerSettings, database: String
    ) async -> Swift.Result<[ISPmanagerTable], ISPmanagerSFTP.Failure> {
        guard let hostKey = settings.host_key_sha256 else {
            return .failure(.message("Ключ SSH-сервера не подтверждён. Подтвердите его на карточке подключения в разделе «Подключения»."))
        }
        return await Task.detached(priority: .userInitiated) {
            let pointer = url.withCString { url in
                credentials.login.withCString { login in
                    credentials.password.withCString { password in
                        hostKey.withCString { key in
                            database.withCString { database in
                                ispmanagerDatabaseTables(url, login, password, settings.ssh_port, key, database)
                            }
                        }
                    }
                }
            }
            guard let pointer else { return .failure(.message("Нет ответа от Core.")) }
            defer { ispmanagerFree(pointer) }
            guard let result = try? JSONDecoder().decode(
                TablesResult.self, from: Data(String(cString: pointer).utf8)
            ) else { return .failure(.message("Неверный ответ Core.")) }
            if let tables = result.data, result.ok { return .success(tables) }
            if let transport = result.transport { return .failure(.transport(transport)) }
            return .failure(.message(QADRENUserMessage.text(for: result.error ?? "ispmanager_database_tables_failed")))
        }.value
    }
}

/// One base table of a MySQL database. `bytes` and `rows` are estimates.
struct ISPmanagerTable: Decodable, Identifiable, Equatable {
    var id: String { name }
    let name: String
    let bytes: UInt64
    let rows: UInt64?
}
