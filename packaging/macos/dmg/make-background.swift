// Writes background.svg for the macOS DMG window (see README.md here): a white field that darkens
// slightly towards the bottom, a dotted arrow from Prism.app to Applications, and the drag-to-install
// line in Korean and English. The text is set in Pretendard (SIL OFL 1.1) and written as outlines,
// so rendering the SVG needs no fonts.
//
// macOS only, with Pretendard installed (https://github.com/orioncactus/pretendard):
//     swift packaging/macos/dmg/make-background.swift
// Then run generate.py, which renders the SVG to background.tiff and writes dmg-layout.DS_Store.
//
// Keep the layout in sync with generate.py: the icons sit at (170, 190) and (490, 190).
import AppKit
import CoreText

let width: CGFloat = 660, height: CGFloat = 400
let arrowY: CGFloat = 190
let out = CommandLine.arguments.count > 1
  ? CommandLine.arguments[1]
  : URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("background.svg").path

func fail(_ message: String) -> Never {
  FileHandle.standardError.write(Data("error: \(message)\n".utf8))
  exit(1)
}

func num(_ v: CGFloat) -> String {
  let s = String(format: "%.2f", Double(v))
  // "12.50" -> "12.5", "12.00" -> "12"
  let trimmed = s.replacingOccurrences(of: #"\.?0+$"#, with: "", options: .regularExpression)
  return trimmed == "-0" ? "0" : trimmed
}

/// One line of text as SVG path data, centred on `centerX` with its baseline at `baseline`
/// (SVG coordinates: y grows downwards).
func outline(_ string: String, font name: String, size: CGFloat, centerX: CGFloat, baseline: CGFloat) -> String {
  let font = CTFontCreateWithName(name as CFString, size, nil)
  guard (CTFontCopyPostScriptName(font) as String) == name else {
    fail("font \(name) is not installed (got \(CTFontCopyPostScriptName(font)))")
  }
  let attributed = NSAttributedString(string: string, attributes: [.font: font])
  let line = CTLineCreateWithAttributedString(attributed)
  let lineWidth = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
  let originX = centerX - lineWidth / 2
  var d = ""
  for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
    let attrs = CTRunGetAttributes(run) as NSDictionary
    guard let value = attrs[kCTFontAttributeName], CFGetTypeID(value as CFTypeRef) == CTFontGetTypeID() else { continue }
    // Checked just above: the value is a CTFont.
    let runFont = value as! CTFont
    let runName = CTFontCopyPostScriptName(runFont) as String
    // A fallback font here would put another typeface's outlines in the file.
    guard runName == name else { fail("\"\(string)\" needs a glyph \(name) lacks (fell back to \(runName))") }
    let count = CTRunGetGlyphCount(run)
    var glyphs = [CGGlyph](repeating: 0, count: count)
    var positions = [CGPoint](repeating: .zero, count: count)
    CTRunGetGlyphs(run, CFRange(location: 0, length: count), &glyphs)
    CTRunGetPositions(run, CFRange(location: 0, length: count), &positions)
    for (glyph, position) in zip(glyphs, positions) {
      guard let path = CTFontCreatePathForGlyph(runFont, glyph, nil) else { continue } // a space
      // Glyph space has y up; SVG has y down.
      let x = originX + position.x, y = baseline - position.y
      path.applyWithBlock { element in
        let p = element.pointee.points
        func pt(_ i: Int) -> String { "\(num(x + p[i].x)) \(num(y - p[i].y))" }
        switch element.pointee.type {
        case .moveToPoint: d += "M \(pt(0)) "
        case .addLineToPoint: d += "L \(pt(0)) "
        case .addQuadCurveToPoint: d += "Q \(pt(0)) \(pt(1)) "
        case .addCurveToPoint: d += "C \(pt(0)) \(pt(1)) \(pt(2)) "
        case .closeSubpath: d += "Z "
        @unknown default: break
        }
      }
    }
  }
  return d.trimmingCharacters(in: .whitespaces)
}

let korean = "프리즘을 Applications 폴더로 끌어다 놓으세요"
let english = "Drag Prism to the Applications folder to install"
// Twelve dots from 268 to 378, then the head.
let dots = stride(from: CGFloat(268), through: 378, by: 10)
  .map { "        <circle cx=\"\(num($0))\" cy=\"\(num(arrowY))\" r=\"1.5\"/>" }
  .joined(separator: "\n")

let svg = """
<svg width="\(num(width))" height="\(num(height))" xmlns="http://www.w3.org/2000/svg">
    <title>Prism — NK Studio</title>
    <desc>\(english).</desc>
    <defs>
        <linearGradient id="field" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#ffffff"/>
            <stop offset="1" stop-color="#eeeef1"/>
        </linearGradient>
    </defs>
    <path fill="url(#field)" d="M 0 0 L \(num(width)) 0 L \(num(width)) \(num(height)) L 0 \(num(height)) Z"/>
    <g fill="#9999a2">
\(dots)
    </g>
    <path fill="none" stroke="#9999a2" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" d="M 378 \(num(arrowY - 15)) L 394 \(num(arrowY)) L 378 \(num(arrowY + 15))"/>
    <path fill="#3a3a40" d="\(outline(korean, font: "Pretendard-Medium", size: 15, centerX: width / 2, baseline: 347))"/>
    <path fill="#8e8e96" d="\(outline(english, font: "Pretendard-Regular", size: 12, centerX: width / 2, baseline: 368))"/>
</svg>

"""
do {
  try svg.write(toFile: out, atomically: true, encoding: .utf8)
} catch {
  fail("couldn't write \(out): \(error.localizedDescription)")
}
print(out)
