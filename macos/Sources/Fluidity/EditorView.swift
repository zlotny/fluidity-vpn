import AppKit
import SwiftUI

struct EditorView: View {
    let id: String?
    @State var model: EditorModel
    let close: () -> Void
    @State private var tab = 0
    @State private var problems: [String] = []
    @EnvironmentObject var manager: ConnectionManager

    init(id: String?, model: EditorModel, close: @escaping () -> Void) {
        self.id = id
        _model = State(initialValue: model)
        self.close = close
    }

    var body: some View {
        VStack(spacing: 0) {
            TabView(selection: $tab) {
                GeneralTab(model: $model).tabItem { Text("General") }.tag(0)
                AuthenticationTab(model: $model).tabItem { Text("Authentication") }.tag(1)
                OptionsTab(model: $model).tabItem { Text("Options") }.tag(2)
                NetworkingTab(model: $model).tabItem { Text("Networking") }.tag(3)
                ProxyTab(model: $model).tabItem { Text("Proxy") }.tag(4)
                AdvancedTab(model: $model).tabItem { Text("Advanced") }.tag(5)
            }
            .padding([.horizontal, .top])
            if !problems.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(problems, id: \.self) { Label($0, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal)
            }
            HStack {
                if let id, manager.isActive(id) {
                    Text("Changes apply the next time you connect.").font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel", action: close).keyboardShortcut(.cancelAction)
                Button("Save", action: save).keyboardShortcut(.defaultAction)
            }
            .padding()
        }
        .frame(minWidth: 640, minHeight: 520)
    }

    private func save() {
        problems = (try? Core.call("editor.validate", model)) ?? []
        guard problems.isEmpty else { return }
        do {
            let _: String = try Core.call("editor.save", EditorSave(id: id, model: model))
            manager.reload()
            close()
        } catch {
            problems = error.localizedDescription.components(separatedBy: "\n")
        }
    }
}

struct FilePicker: View {
    let title: String
    @Binding var field: FileField

    var body: some View {
        LabeledContent(title) {
            HStack {
                Text(field.displayName)
                    .foregroundStyle(field.isSet ? .primary : .secondary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                Button("Select…", action: choose)
                if field.isSet {
                    Button { field = FileField() } label: { Image(systemName: "xmark.circle.fill") }.buttonStyle(.borderless)
                }
            }
        }
    }

    private func choose() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.message = "Select the \(title.lowercased())."
        guard panel.runModal() == .OK, let url = panel.url, let data = try? Data(contentsOf: url) else { return }
        field = FileField(name: url.lastPathComponent, data: data.base64EncodedString())
    }
}

struct ScriptPicker: View {
    let title: String
    @Binding var path: String

    var body: some View {
        LabeledContent(title) {
            HStack {
                Text(path.isEmpty ? "None" : URL(fileURLWithPath: path).lastPathComponent)
                    .foregroundStyle(path.isEmpty ? .secondary : .primary)
                    .lineLimit(1).truncationMode(.middle)
                Spacer()
                Button("Select…") {
                    let panel = NSOpenPanel()
                    panel.canChooseFiles = true
                    if panel.runModal() == .OK, let url = panel.url { path = url.path }
                }
                if !path.isEmpty {
                    Button { path = "" } label: { Image(systemName: "xmark.circle.fill") }.buttonStyle(.borderless)
                }
            }
        }
    }
}

private func optionalNumber<T: FixedWidthInteger>(_ binding: Binding<T?>) -> Binding<String> {
    Binding(get: { binding.wrappedValue.map { String($0) } ?? "" }, set: { binding.wrappedValue = T($0.trimmingCharacters(in: .whitespaces)) })
}

private func lines(_ binding: Binding<[String]>) -> Binding<String> {
    Binding(get: { binding.wrappedValue.joined(separator: ", ") }, set: {
        binding.wrappedValue = $0.split(whereSeparator: { $0 == "," || $0 == " " || $0.isNewline }).map(String.init)
    })
}

struct GeneralTab: View {
    @Binding var model: EditorModel
    @State private var selection: Remote.ID?

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $model.name)
            }
            Section("Servers") {
                Table($model.remotes, selection: $selection) {
                    TableColumn("Address") { $r in TextField("vpn.example.com", text: $r.host).labelsHidden() }
                    TableColumn("Port") { $r in
                        TextField("1194", value: $r.port, format: .number.grouping(.never)).labelsHidden()
                    }.width(70)
                    TableColumn("Protocol") { $r in
                        Picker("", selection: $r.transport) {
                            Text("UDP").tag("udp")
                            Text("TCP").tag("tcp")
                        }.labelsHidden()
                    }.width(80)
                    TableColumn("IP") { $r in
                        Picker("", selection: $r.ipVersion) {
                            Text("Auto").tag("automatic")
                            Text("IPv4").tag("ipv4")
                            Text("IPv6").tag("ipv6")
                        }.labelsHidden()
                    }.width(80)
                }
                .frame(minHeight: 110)
                HStack {
                    Button { model.remotes.append(Remote()) } label: { Image(systemName: "plus") }
                    Button { model.remotes.removeAll { $0.id == selection } } label: { Image(systemName: "minus") }
                        .disabled(selection == nil || model.remotes.count < 2)
                    Spacer()
                    Toggle("Try servers in random order", isOn: $model.remoteRandom)
                }
                TextField("Server timeout (seconds)", text: optionalNumber($model.serverTimeout), prompt: Text("120"))
            }
            Section {
                Picker("Device", selection: $model.device) {
                    Text("TUN (routed)").tag("tun")
                    Text("TAP (bridged)").tag("tap")
                }
                Toggle("Use DHCP for the TAP device", isOn: $model.dhcp).disabled(model.device != "tap")
                Toggle("Enable IPv6", isOn: $model.ipv6)
                Toggle("Reconnect automatically", isOn: $model.autoReconnect)
                Toggle("Connect when Fluidity opens", isOn: $model.connectOnLaunch)
            }
        }
        .formStyle(.grouped)
    }
}

struct AuthenticationTab: View {
    @Binding var model: EditorModel

    var body: some View {
        Form {
            Section {
                Picker("Type", selection: $model.authType) {
                    Text("SSL/TLS client").tag("tls")
                    Text("PKCS#12 file").tag("pkcs12")
                    Text("PKCS#11 token").tag("pkcs11")
                    Text("Keychain identity").tag("system_identity")
                    Text("Static key").tag("static_key")
                    Text("None").tag("none")
                }
                Toggle("Require username and password", isOn: $model.userPass)
            }
            Section("Certificates") {
                switch model.authType {
                case "tls":
                    FilePicker(title: "CA", field: $model.ca)
                    FilePicker(title: "Certificate", field: $model.cert)
                    FilePicker(title: "Key", field: $model.key)
                case "pkcs12":
                    FilePicker(title: "CA", field: $model.ca)
                    FilePicker(title: "PKCS#12", field: $model.pkcs12)
                case "pkcs11":
                    FilePicker(title: "CA", field: $model.ca)
                    TextField("Provider library", text: $model.pkcs11Providers)
                    TextField("Token ID", text: $model.pkcs11Id)
                    Toggle("Choose the token ID when connecting", isOn: $model.pkcs11Prompt)
                case "system_identity":
                    FilePicker(title: "CA", field: $model.ca)
                    IdentityPicker(identity: $model.systemIdentity)
                case "static_key":
                    FilePicker(title: "Static key", field: $model.staticKey)
                    directionPicker($model.staticKeyDirection)
                default:
                    Text("The connection uses no certificates.").foregroundStyle(.secondary)
                }
            }
            if model.authType != "static_key" {
                Section("TLS control channel") {
                    Picker("Protection", selection: $model.extraType) {
                        Text("None").tag("none")
                        Text("TLS Auth").tag("tls_auth")
                        Text("TLS Crypt").tag("tls_crypt")
                        Text("TLS Crypt v2").tag("tls_crypt_v2")
                    }
                    if model.extraType != "none" {
                        FilePicker(title: "Key", field: $model.extraKey)
                        if model.extraType == "tls_auth" { directionPicker($model.extraDirection) }
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    private func directionPicker(_ binding: Binding<UInt8?>) -> some View {
        Picker("Direction", selection: binding) {
            Text("None").tag(UInt8?.none)
            Text("0").tag(UInt8?.some(0))
            Text("1").tag(UInt8?.some(1))
        }
    }
}

struct IdentityPicker: View {
    @Binding var identity: String
    @State private var identities: [String] = []

    var body: some View {
        Picker("Identity", selection: $identity) {
            Text("Choose…").tag("")
            ForEach(identities, id: \.self) { Text($0).tag($0) }
            if !identity.isEmpty && !identities.contains(identity) { Text(identity).tag(identity) }
        }
        .task { identities = Keychain.identityNames() }
    }
}

struct OptionsTab: View {
    @Binding var model: EditorModel

    var body: some View {
        Form {
            Section("Keepalive") {
                TextField("Ping interval (seconds)", text: optionalNumber($model.ping))
                TextField("Restart after (seconds)", text: optionalNumber($model.pingRestart))
            }
            Section("Persistence") {
                Toggle("Persist TUN device", isOn: $model.persistTun)
                Toggle("Persist key", isOn: $model.persistKey)
                Toggle("Persist local IP", isOn: $model.persistLocalIp)
                Toggle("Persist remote IP", isOn: $model.persistRemoteIp)
            }
            Section("Behavior") {
                Toggle("Require server certificate usage (remote-cert-tls)", isOn: $model.remoteCertTls)
                Toggle("Do not bind to a local port", isOn: $model.nobind)
                Toggle("Pull options from the server", isOn: $model.pull)
                Picker("Compression", selection: $model.compression) {
                    Text("Off").tag("off")
                    Text("None (framing only)").tag("none")
                    Text("LZO").tag("lzo")
                    Text("LZ4").tag("lz4")
                    Text("LZ4 v2").tag("lz4_v2")
                    Text("Stub").tag("stub")
                    Text("Stub v2").tag("stub_v2")
                    Text("Adaptive LZO").tag("adaptive")
                }
                Picker("Compatibility", selection: $model.compatibility) {
                    Text("Current").tag("")
                    ForEach(["2.6", "2.5", "2.4", "2.3"], id: \.self) { Text("OpenVPN \($0)").tag($0) }
                }
            }
        }
        .formStyle(.grouped)
    }
}

struct NetworkingTab: View {
    @Binding var model: EditorModel
    @State private var selection: Route.ID?

    var body: some View {
        Form {
            Section("Traffic") {
                Picker("Send traffic over VPN", selection: $model.redirect) {
                    Text("As the server sets").tag("automatic")
                    Text("All traffic").tag("all")
                    Text("All IPv4 traffic").tag("ipv4")
                    Text("All IPv6 traffic").tag("ipv6")
                }
                TextField("Default gateway", text: $model.defaultGateway)
            }
            Section("Routes") {
                Table($model.routes, selection: $selection) {
                    TableColumn("Network") { $r in TextField("10.0.0.0", text: $r.network).labelsHidden() }
                    TableColumn("Mask / Prefix") { $r in TextField("255.0.0.0", text: $r.mask).labelsHidden() }
                    TableColumn("Gateway") { $r in TextField("vpn_gateway", text: $r.gateway).labelsHidden() }
                    TableColumn("Metric") { $r in TextField("", text: $r.metric).labelsHidden() }.width(50)
                    TableColumn("IPv6") { $r in Toggle("", isOn: $r.ipv6).labelsHidden() }.width(40)
                }
                .frame(minHeight: 90)
                HStack {
                    Button { model.routes.append(Route()) } label: { Image(systemName: "plus") }
                    Button { model.routes.removeAll { $0.id == selection } } label: { Image(systemName: "minus") }.disabled(selection == nil)
                }
            }
            Section("DNS") {
                Picker("Mode", selection: $model.dnsMode) {
                    Text("Automatic").tag("automatic")
                    Text("Full (all queries)").tag("full")
                    Text("Split (listed domains)").tag("split")
                    Text("Off").tag("off")
                }
                TextField("Servers", text: lines($model.dnsServers), prompt: Text("10.0.0.1, 10.0.0.2"))
                TextField("Domains", text: lines($model.dnsDomains), prompt: Text("corp.example.com"))
                Toggle("Ignore DNS settings from the server", isOn: $model.ignorePushedDns)
            }
            Section("Link") {
                TextField("Traffic shaping (bytes/s)", text: optionalNumber($model.shaper))
                TextField("Fragment size", text: optionalNumber($model.fragment))
                TextField("TUN MTU", text: optionalNumber($model.tunMtu))
                TextField("Disconnect when inactive (seconds)", text: optionalNumber($model.inactive))
            }
        }
        .formStyle(.grouped)
    }
}

struct ProxyTab: View {
    @Binding var model: EditorModel

    var body: some View {
        Form {
            Section("Proxy") {
                Picker("Type", selection: $model.proxyType) {
                    Text("None").tag("none")
                    Text("HTTP").tag("http")
                    Text("SOCKS").tag("socks")
                    Text("System proxy").tag("system")
                }
                if model.proxyType == "http" || model.proxyType == "socks" {
                    TextField("Address", text: $model.proxyHost)
                    TextField("Port", text: optionalNumber($model.proxyPort))
                }
                if model.proxyType == "http" {
                    Picker("Authentication", selection: $model.proxyAuth) {
                        Text("None").tag("none")
                        Text("Basic").tag("basic")
                        Text("NTLM").tag("ntlm")
                        Text("Automatic").tag("automatic")
                    }
                }
            }
            Section("Obfuscation") {
                Picker("Method", selection: $model.obfuscation) {
                    Text("None").tag("")
                    Text("obfs2").tag("obfs2")
                    Text("obfs3").tag("obfs3")
                    Text("obfs4").tag("obfs4")
                    Text("ScrambleSuit").tag("scramblesuit")
                }
                if model.obfuscation == "obfs4" || model.obfuscation == "scramblesuit" {
                    TextField("Key or certificate", text: $model.obfuscationKey)
                }
            }
        }
        .formStyle(.grouped)
    }
}

struct AdvancedTab: View {
    @Binding var model: EditorModel

    var body: some View {
        Form {
            Section("Scripts") {
                ScriptPicker(title: "Before connect", path: $model.beforeConnectScript)
                ScriptPicker(title: "Connected", path: $model.connectedScript)
                ScriptPicker(title: "Disconnected", path: $model.disconnectedScript)
            }
            Section("Additional OpenVPN commands") {
                TextEditor(text: $model.extraCommands)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 160)
                Text("One command per line. Lines that start with #fluidity set Fluidity options.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
    }
}
