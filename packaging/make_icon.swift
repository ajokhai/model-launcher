import Cocoa

// Draws Model Launcher's icon: a white play mark with a spark on a warm orange squircle.
// Usage: swift make_icon.swift out.png
let size: CGFloat = 1024
let img = NSImage(size: NSSize(width: size, height: size))
img.lockFocus()
let rect = NSRect(x: 100, y: 100, width: 824, height: 824)
let squircle = NSBezierPath(roundedRect: rect, xRadius: 185, yRadius: 185)
NSGraphicsContext.current?.saveGraphicsState()
let shadow = NSShadow()
shadow.shadowColor = NSColor.black.withAlphaComponent(0.25)
shadow.shadowOffset = NSSize(width: 0, height: -12)
shadow.shadowBlurRadius = 28
shadow.set()
NSColor.white.setFill()
squircle.fill()
NSGraphicsContext.current?.restoreGraphicsState()
NSGradient(colors: [NSColor(red: 1.00, green: 0.74, blue: 0.38, alpha: 1),
                    NSColor(red: 0.95, green: 0.38, blue: 0.18, alpha: 1)])!.draw(in: squircle, angle: -65)

// play mark with rounded corners
let tri = NSBezierPath()
tri.move(to: NSPoint(x: 395, y: 330))
tri.line(to: NSPoint(x: 395, y: 694))
tri.line(to: NSPoint(x: 710, y: 512))
tri.close()
tri.lineJoinStyle = .round
tri.lineWidth = 70
NSColor.white.setStroke()
NSColor.white.setFill()
tri.stroke()
tri.fill()

// spark
func spark(_ c: NSPoint, _ r: CGFloat) {
    let p = NSBezierPath()
    p.move(to: NSPoint(x: c.x, y: c.y + r))
    p.curve(to: NSPoint(x: c.x + r, y: c.y), controlPoint1: NSPoint(x: c.x + r * 0.15, y: c.y + r * 0.15), controlPoint2: NSPoint(x: c.x + r * 0.15, y: c.y + r * 0.15))
    p.curve(to: NSPoint(x: c.x, y: c.y - r), controlPoint1: NSPoint(x: c.x + r * 0.15, y: c.y - r * 0.15), controlPoint2: NSPoint(x: c.x + r * 0.15, y: c.y - r * 0.15))
    p.curve(to: NSPoint(x: c.x - r, y: c.y), controlPoint1: NSPoint(x: c.x - r * 0.15, y: c.y - r * 0.15), controlPoint2: NSPoint(x: c.x - r * 0.15, y: c.y - r * 0.15))
    p.curve(to: NSPoint(x: c.x, y: c.y + r), controlPoint1: NSPoint(x: c.x - r * 0.15, y: c.y + r * 0.15), controlPoint2: NSPoint(x: c.x - r * 0.15, y: c.y + r * 0.15))
    NSColor.white.withAlphaComponent(0.92).setFill()
    p.fill()
}
spark(NSPoint(x: 735, y: 735), 70)
spark(NSPoint(x: 655, y: 800), 32)
img.unlockFocus()
let rep = NSBitmapImageRep(data: img.tiffRepresentation!)!
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
