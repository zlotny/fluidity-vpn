import AppKit
import CoreLocation
import CoreWLAN

@MainActor
final class WiFiWatcher: NSObject, ObservableObject {
    static let shared = WiFiWatcher()

    @Published private(set) var ssid: String?
    @Published private(set) var authorization: CLAuthorizationStatus = .notDetermined
    var onChange: ((String?) -> Void)?

    private let client = CWWiFiClient.shared()
    private let location = CLLocationManager()
    private let events = EventBridge()

    var canReadNetworkName: Bool { authorization == .authorizedAlways || ssid != nil }

    func start() {
        events.owner = self
        client.delegate = events
        try? client.startMonitoringEvent(with: .ssidDidChange)
        try? client.startMonitoringEvent(with: .linkDidChange)
        location.delegate = events
        authorization = location.authorizationStatus
        refresh()
    }

    func requestAccess() {
        if location.authorizationStatus == .notDetermined {
            location.requestWhenInUseAuthorization()
        } else if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices") {
            NSWorkspace.shared.open(url)
        }
    }

    fileprivate func refresh() {
        authorization = location.authorizationStatus
        let current = client.interface()?.ssid()
        guard current != ssid else { return }
        ssid = current
        onChange?(current)
    }
}

private final class EventBridge: NSObject, CWEventDelegate, CLLocationManagerDelegate, @unchecked Sendable {
    weak var owner: WiFiWatcher?

    private func notify() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            MainActor.assumeIsolated { self?.owner?.refresh() }
        }
    }

    func ssidDidChangeForWiFiInterface(withName interfaceName: String) { notify() }
    func linkDidChangeForWiFiInterface(withName interfaceName: String) { notify() }
    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) { notify() }
}
