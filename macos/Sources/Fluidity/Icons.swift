import AppKit

enum IconSet: String, CaseIterable, Identifiable {
    case droplet = "default"
    case lock
    case shield
    case dot
    case colored

    var id: String { rawValue }

    var title: String {
        switch self {
        case .droplet: return "Droplet"
        case .lock: return "Lock"
        case .shield: return "Shield"
        case .dot: return "Dot"
        case .colored: return "Droplet (color)"
        }
    }
}

enum StatusIcon {
    static let frameCount = 8
    static let size = NSSize(width: 18, height: 18)

    static func image(set: IconSet, state: SessionState, frame: Int = 0) -> NSImage {
        let image = NSImage(size: size, flipped: false) { rect in
            draw(set: set, state: state, frame: frame, in: rect)
            return true
        }
        image.isTemplate = set != .colored
        image.accessibilityDescription = "Fluidity: \(state.label)"
        return image
    }

    private static func fillLevel(_ state: SessionState, _ frame: Int) -> CGFloat {
        switch state {
        case .connected: return 1
        case .disconnected, .error: return 0
        default:
            let phase = Double(frame % frameCount) / Double(frameCount)
            return CGFloat(0.15 + 0.7 * (0.5 - 0.5 * cos(phase * 2 * .pi)))
        }
    }

    private static func draw(set: IconSet, state: SessionState, frame: Int, in rect: NSRect) {
        let level = fillLevel(state, frame)
        switch set {
        case .droplet, .colored:
            let path = droplet(in: rect.insetBy(dx: 3, dy: 1))
            stroke(path, fill: level, color: set == .colored ? color(for: state) : .black)
        case .shield:
            let path = shield(in: rect.insetBy(dx: 2.5, dy: 1.5))
            stroke(path, fill: level, color: .black)
        case .lock:
            lock(in: rect, open: state != .connected, level: level)
        case .dot:
            let ring = NSBezierPath(ovalIn: rect.insetBy(dx: 3.5, dy: 3.5))
            stroke(ring, fill: level, color: .black)
        }
        if state == .error {
            NSColor.black.setFill()
            let badge = NSBezierPath(ovalIn: NSRect(x: rect.maxX - 6, y: rect.minY, width: 6, height: 6))
            badge.fill()
        }
    }

    private static func color(for state: SessionState) -> NSColor {
        switch state {
        case .connected: return .systemGreen
        case .error: return .systemRed
        case .disconnected: return .secondaryLabelColor
        default: return .systemOrange
        }
    }

    private static func stroke(_ path: NSBezierPath, fill level: CGFloat, color: NSColor) {
        color.setStroke()
        path.lineWidth = 1.4
        path.stroke()
        guard level > 0 else { return }
        NSGraphicsContext.saveGraphicsState()
        path.addClip()
        let bounds = path.bounds
        color.setFill()
        NSRect(x: bounds.minX, y: bounds.minY, width: bounds.width, height: bounds.height * level).fill()
        NSGraphicsContext.restoreGraphicsState()
    }

    private static func droplet(in r: NSRect) -> NSBezierPath {
        let p = NSBezierPath()
        let top = NSPoint(x: r.midX, y: r.maxY)
        let radius = r.width / 2
        let center = NSPoint(x: r.midX, y: r.minY + radius)
        p.move(to: top)
        p.curve(
            to: NSPoint(x: r.maxX, y: center.y),
            controlPoint1: NSPoint(x: r.midX + radius * 0.35, y: r.maxY - r.height * 0.25),
            controlPoint2: NSPoint(x: r.maxX, y: center.y + radius * 0.6))
        p.appendArc(withCenter: center, radius: radius, startAngle: 0, endAngle: 180, clockwise: true)
        p.curve(
            to: top,
            controlPoint1: NSPoint(x: r.minX, y: center.y + radius * 0.6),
            controlPoint2: NSPoint(x: r.midX - radius * 0.35, y: r.maxY - r.height * 0.25))
        p.close()
        return p
    }

    private static func shield(in r: NSRect) -> NSBezierPath {
        let p = NSBezierPath()
        p.move(to: NSPoint(x: r.midX, y: r.maxY))
        p.curve(
            to: NSPoint(x: r.maxX, y: r.maxY - r.height * 0.2),
            controlPoint1: NSPoint(x: r.midX + r.width * 0.25, y: r.maxY - r.height * 0.08),
            controlPoint2: NSPoint(x: r.maxX - r.width * 0.1, y: r.maxY - r.height * 0.15))
        p.curve(
            to: NSPoint(x: r.midX, y: r.minY),
            controlPoint1: NSPoint(x: r.maxX, y: r.midY - r.height * 0.1),
            controlPoint2: NSPoint(x: r.midX + r.width * 0.3, y: r.minY + r.height * 0.15))
        p.curve(
            to: NSPoint(x: r.minX, y: r.maxY - r.height * 0.2),
            controlPoint1: NSPoint(x: r.midX - r.width * 0.3, y: r.minY + r.height * 0.15),
            controlPoint2: NSPoint(x: r.minX, y: r.midY - r.height * 0.1))
        p.curve(
            to: NSPoint(x: r.midX, y: r.maxY),
            controlPoint1: NSPoint(x: r.minX + r.width * 0.1, y: r.maxY - r.height * 0.15),
            controlPoint2: NSPoint(x: r.midX - r.width * 0.25, y: r.maxY - r.height * 0.08))
        p.close()
        return p
    }

    private static func lock(in rect: NSRect, open: Bool, level: CGFloat) {
        let body = NSBezierPath(roundedRect: NSRect(x: rect.minX + 3, y: rect.minY + 2, width: 12, height: 8.5), xRadius: 1.5, yRadius: 1.5)
        stroke(body, fill: max(level, 0), color: .black)
        let shackle = NSBezierPath()
        let left: CGFloat = rect.minX + 5.5
        let right: CGFloat = rect.minX + 12.5
        let base: CGFloat = rect.minY + 10.5
        shackle.move(to: NSPoint(x: left, y: base))
        shackle.line(to: NSPoint(x: left, y: base + 2.5))
        shackle.appendArc(withCenter: NSPoint(x: (left + right) / 2, y: base + 2.5), radius: (right - left) / 2, startAngle: 180, endAngle: 0, clockwise: true)
        shackle.line(to: NSPoint(x: right, y: open ? base + 1.5 : base))
        if open {
            let t = NSAffineTransform()
            t.translateX(by: 3.5, yBy: 1)
            shackle.transform(using: t as AffineTransform)
        }
        NSColor.black.setStroke()
        shackle.lineWidth = 1.6
        shackle.stroke()
    }

    static func appIcon(size: CGFloat = 512) -> NSImage {
        NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            let inset = rect.insetBy(dx: size * 0.08, dy: size * 0.08)
            let bg = NSBezierPath(roundedRect: inset, xRadius: size * 0.2, yRadius: size * 0.2)
            NSGradient(colors: [
                NSColor(calibratedRed: 0.10, green: 0.45, blue: 0.95, alpha: 1),
                NSColor(calibratedRed: 0.05, green: 0.75, blue: 0.85, alpha: 1),
            ])?.draw(in: bg, angle: 90)
            let drop = droplet(in: inset.insetBy(dx: inset.width * 0.28, dy: inset.height * 0.18))
            NSColor.white.withAlphaComponent(0.95).setFill()
            drop.fill()
            let shine = NSBezierPath(ovalIn: NSRect(
                x: inset.midX - inset.width * 0.12, y: inset.minY + inset.height * 0.3,
                width: inset.width * 0.08, height: inset.height * 0.16))
            NSColor(calibratedRed: 0.10, green: 0.55, blue: 0.95, alpha: 0.5).setFill()
            shine.fill()
            return true
        }
    }

    static func writeIconset(to dir: URL) -> Bool {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        for base in [16, 32, 128, 256, 512] {
            for scale in [1, 2] {
                let pixels = base * scale
                guard let rep = NSBitmapImageRep(
                    bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
                    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                    bytesPerRow: 0, bitsPerPixel: 0)
                else { return false }
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
                appIcon(size: CGFloat(pixels)).draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels))
                NSGraphicsContext.restoreGraphicsState()
                let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
                guard let png = rep.representation(using: .png, properties: [:]),
                      (try? png.write(to: dir.appendingPathComponent(name))) != nil
                else { return false }
            }
        }
        return true
    }
}
