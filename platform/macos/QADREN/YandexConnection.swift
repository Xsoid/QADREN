import AppKit
import AuthenticationServices
import CryptoKit
import Foundation
import Security
import SwiftUI

// macOS-only OAuth boundary. Never persist access tokens in UserDefaults or SQLite.
@MainActor
final class YandexConnection: NSObject, ObservableObject, ASWebAuthenticationPresentationContextProviding {
    @Published var clientID = ""
    @Published private(set) var status = "Not connected"
    @Published private(set) var connected = false
    @Published private(set) var busy = false

    private var session: ASWebAuthenticationSession?
    private let service = "dev.qadren.yandex.oauth"
    private let account = "default"
    // Must match the exact registered redirect URI at Yandex OAuth.
    private let redirect = "qadren://oauth/callback"

    override init() {
        super.init()
        connected = (try? readToken()) != nil
        status = connected ? "Token stored in Keychain" : "Not connected"
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        NSApp.keyWindow ?? NSApp.windows.first ?? ASPresentationAnchor()
    }

    func connect() {
        let id = clientID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty, id.rangeOfCharacter(from: .whitespacesAndNewlines) == nil else {
            status = "Enter registered Yandex OAuth Client ID"
            return
        }
        guard !busy else { return }
        busy = true

        let verifier = Self.randomURLSafe(32)
        let digest = SHA256.hash(data: Data(verifier.utf8))
        let challenge = Data(digest).base64URLEncodedString()
        let state = Self.randomURLSafe(32)
        var url = URLComponents(string: "https://oauth.yandex.ru/authorize")!
        url.queryItems = [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: id),
            URLQueryItem(name: "redirect_uri", value: redirect),
            URLQueryItem(name: "scope", value: "cloud_api:disk.read"),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256")
        ]
        guard let authorizationURL = url.url else {
            status = "Invalid authorization URL"
            busy = false
            return
        }
        let auth = ASWebAuthenticationSession(
            url: authorizationURL,
            callbackURLScheme: "qadren"
        ) { [weak self] callback, error in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.session = nil
                guard error == nil, let callback, let components = URLComponents(url: callback, resolvingAgainstBaseURL: false),
                      components.scheme == "qadren", components.host == "oauth",
                      components.path == "/callback" else {
                    self.status = "Authentication cancelled or callback unavailable"
                    self.busy = false
                    return
                }
                let params = Dictionary(components.queryItems?.compactMap { item in
                    item.value.map { (item.name, $0) }
                } ?? [], uniquingKeysWith: { first, _ in first })
                guard params["state"] == state, let code = params["code"] else {
                    self.status = "OAuth state or code mismatch"
                    self.busy = false
                    return
                }
                await self.exchange(code: code, verifier: verifier, clientID: id)
            }
        }
        auth.presentationContextProvider = self
        auth.prefersEphemeralWebBrowserSession = false
        session = auth
        if !auth.start() {
            session = nil
            status = "Failed to start browser authorization"
            busy = false
        } else {
            status = "Waiting for Yandex authorization"
        }
    }

    private func exchange(code: String, verifier: String, clientID: String) async {
        var request = URLRequest(url: URL(string: "https://oauth.yandex.ru/token")!)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        var fields = URLComponents()
        fields.queryItems = [
            URLQueryItem(name: "grant_type", value: "authorization_code"),
            URLQueryItem(name: "code", value: code),
            URLQueryItem(name: "client_id", value: clientID),
            URLQueryItem(name: "code_verifier", value: verifier)
        ]
        request.httpBody = fields.percentEncodedQuery?.data(using: .utf8)
        defer { busy = false }
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                status = "Yandex token exchange failed"
                return
            }
            let token = try JSONDecoder().decode(YandexToken.self, from: data)
            guard !token.access_token.isEmpty else {
                status = "Empty access token"
                return
            }
            try storeToken(token.access_token)
            connected = true
            status = "Connected; access token stored in Keychain"
        } catch {
            // Do not print token, code, full response body or OAuth callback.
            status = "Authorization failed or Keychain unavailable"
        }
    }

    func disconnect() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        let result = SecItemDelete(query as CFDictionary)
        guard result == errSecSuccess || result == errSecItemNotFound else {
            status = "Could not remove credentials from Keychain"
            return
        }
        connected = false
        status = "Disconnected locally (Yandex token not revoked)"
    }

    private func readToken() throws -> String? {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var item: CFTypeRef?
        let code = SecItemCopyMatching(query as CFDictionary, &item)
        if code == errSecItemNotFound { return nil }
        guard code == errSecSuccess, let data = item as? Data,
              let token = String(data: data, encoding: .utf8) else {
            throw OAuthError.keychain
        }
        query.removeAll()
        return token
    }

    private func storeToken(_ token: String) throws {
        let item: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: Data(token.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        ]
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        let status = SecItemCopyMatching(query.merging([kSecReturnAttributes as String: true]) { _, newer in newer } as CFDictionary, nil)
        let result: OSStatus
        if status == errSecItemNotFound {
            result = SecItemAdd(item as CFDictionary, nil)
        } else if status == errSecSuccess {
            result = SecItemUpdate(query as CFDictionary, [kSecValueData as String: Data(token.utf8)] as CFDictionary)
        } else {
            result = status
        }
        guard result == errSecSuccess else { throw OAuthError.keychain }
    }

    private static func randomURLSafe(_ count: Int) -> String {
        var data = [UInt8](repeating: 0, count: count)
        let result = SecRandomCopyBytes(kSecRandomDefault, count, &data)
        precondition(result == errSecSuccess, "Secure random generation unavailable")
        return Data(data).base64URLEncodedString()
    }
}

private struct YandexToken: Decodable {
    let access_token: String
}

private enum OAuthError: Error {
    case keychain
}

private extension Data {
    func base64URLEncodedString() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
