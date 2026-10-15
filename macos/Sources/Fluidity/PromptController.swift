import AppKit
import SwiftUI

@MainActor
final class PromptController {
    typealias Completion = (Reply, Bool) -> Void

    private var panels: [String: NSPanel] = [:]

    func present(
        id: String, title: String, prompt: Prompt, saved: SavedCredentials,
        splitter: StaticChallengePassword? = nil, completion: @escaping Completion
    ) {
        dismiss(id)
        if case .webAuth(let url) = prompt {
            if let url = URL(string: url) { NSWorkspace.shared.open(url) }
            return
        }
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 420, height: 200),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        panel.title = title
        panel.level = .floating
        panel.isReleasedWhenClosed = false
        let finish: Completion = { [weak self, weak panel] reply, remember in
            completion(reply, remember)
            panel?.orderOut(nil)
            self?.panels[id] = nil
        }
        let view = PromptView(
            title: title, prompt: prompt,
            username: saved.username ?? "", splitter: splitter, finish: finish)
        panel.contentViewController = NSHostingController(rootView: view)
        panel.center()
        panels[id] = panel
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
    }

    func dismiss(_ id: String) {
        panels.removeValue(forKey: id)?.orderOut(nil)
    }

    func dismissAll() {
        for id in Array(panels.keys) { dismiss(id) }
    }
}

struct PromptView: View {
    let title: String
    let prompt: Prompt
    @State var username: String
    let splitter: StaticChallengePassword?
    let finish: PromptController.Completion

    @State private var password = ""
    @State private var response = ""
    @State private var remember = false

    init(title: String, prompt: Prompt, username: String, splitter: StaticChallengePassword?, finish: @escaping PromptController.Completion) {
        self.title = title
        self.prompt = prompt
        _username = State(initialValue: username)
        self.splitter = splitter
        self.finish = finish
    }

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            Image(nsImage: NSApp.applicationIconImage)
                .resizable()
                .frame(width: 56, height: 56)
            VStack(alignment: .leading, spacing: 10) {
                Text(heading).font(.headline)
                if let detail { Text(detail).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                fields
                HStack {
                    if showsRemember { Toggle("Save in Keychain", isOn: $remember) }
                    Spacer()
                    Button("Cancel") { finish(.cancel, false) }.keyboardShortcut(.cancelAction)
                    if case .needOk = prompt {
                        Button("Decline") { reply(ok: false) }
                    }
                    Button(confirmLabel) { reply(ok: true) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!canSubmit)
                }
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private var heading: String {
        switch prompt {
        case .credentials(_, _, let retry): return retry ? "Authentication failed. Try again." : "Enter your credentials"
        case .password(let realm): return realm == "Private Key" ? "Enter the private key passphrase" : "Enter the \(realm) password"
        case .challenge: return "Additional authentication is required"
        case .needOk: return "Confirmation required"
        case .needStr: return "Input required"
        case .webAuth: return "Open the browser to authenticate"
        case .choice: return "Choose a certificate"
        case .certificate, .sign: return "Using keychain identity"
        }
    }

    private var detail: String? {
        switch prompt {
        case .challenge(_, let text, _): return text
        case .needOk(_, let message), .needStr(_, let message): return message
        case .choice(_, _, let options) where options.isEmpty: return "No tokens were found. Connect your token and try again."
        case .credentials(_, let sc, _):
            if let splitter, sc != nil, !splitter.text.isEmpty { return "Enter your password, then \(splitter.text), separated by \"\(splitter.delimiter)\"." }
            return nil
        default: return nil
        }
    }

    private var confirmLabel: String {
        if case .needOk = prompt { return "Accept" }
        return "OK"
    }

    private var showsRemember: Bool {
        switch prompt {
        case .credentials, .password: return true
        default: return false
        }
    }

    private var staticChallenge: StaticChallengePrompt? {
        if case .credentials(_, let sc, _) = prompt, splitter == nil || splitter?.delimiter.isEmpty == true { return sc }
        return nil
    }

    private var canSubmit: Bool {
        switch prompt {
        case .credentials: return !username.isEmpty
        case .needOk: return true
        case .choice(_, _, let options): return !options.isEmpty
        default: return true
        }
    }

    @ViewBuilder private var fields: some View {
        Form {
            switch prompt {
            case .credentials:
                TextField("Username", text: $username)
                SecureField("Password", text: $password)
                if let sc = staticChallenge {
                    if sc.echo { TextField(sc.text, text: $response) } else { SecureField(sc.text, text: $response) }
                }
            case .password:
                SecureField("Password", text: $password)
            case .challenge(_, _, let echo):
                if echo { TextField("Response", text: $response) } else { SecureField("Response", text: $response) }
            case .needStr:
                TextField("Value", text: $response)
            case .choice(_, _, let options):
                Picker("Certificate", selection: $response) {
                    ForEach(options, id: \.value) { Text($0.label).tag($0.value) }
                }
                .onAppear { if response.isEmpty { response = options.first?.value ?? "" } }
            default:
                EmptyView()
            }
        }
        .formStyle(.columns)
    }

    private func reply(ok: Bool) {
        switch prompt {
        case .credentials(let realm, let sc, _):
            var secret = password
            var challenge: String? = sc == nil ? nil : response
            if sc != nil, let splitter, !splitter.delimiter.isEmpty, let range = password.range(of: splitter.delimiter, options: .backwards) {
                secret = String(password[..<range.lowerBound])
                challenge = String(password[range.upperBound...])
            }
            finish(.credentials(realm: realm, username: username, password: secret, challengeResponse: challenge), remember)
        case .password(let realm):
            finish(.password(realm: realm, password: password), remember)
        case .challenge(let realm, _, _):
            finish(.password(realm: realm, password: response), false)
        case .needOk(let name, _):
            finish(.needOk(name: name, ok: ok), false)
        case .needStr(let name, _), .choice(let name, _, _):
            finish(.needStr(name: name, value: response), false)
        case .webAuth, .certificate, .sign:
            finish(.cancel, false)
        }
    }
}
