// Draws an orange "DEV" band across the bottom of a square icon picture, for Moss Dev's icon.
// Used by scripts/dev-icon.sh:  swift scripts/dev-icon.swift <in.png> <out.png>
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

let args = CommandLine.arguments
guard args.count == 3 else {
  FileHandle.standardError.write("usage: swift scripts/dev-icon.swift <in.png> <out.png>\n".data(using: .utf8)!)
  exit(2)
}
guard let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
  let picture = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
  FileHandle.standardError.write("can't read \(args[1])\n".data(using: .utf8)!)
  exit(1)
}

let (w, h) = (CGFloat(picture.width), CGFloat(picture.height))
let context = CGContext(
  data: nil, width: picture.width, height: picture.height, bitsPerComponent: 8, bytesPerRow: 0,
  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
context.draw(picture, in: CGRect(x: 0, y: 0, width: w, height: h))

// A band low on the tile, under the moss, across the whole width (the icon's shape rounds its ends).
let band = CGRect(x: 0, y: h * 0.095, width: w, height: h * 0.15)
context.setFillColor(CGColor(srgbRed: 0.91, green: 0.36, blue: 0.05, alpha: 1))
context.fill(band)

let font = CTFontCreateUIFontForLanguage(.emphasizedSystem, h * 0.105, nil)!
let text = NSAttributedString(
  string: "DEV",
  attributes: [
    NSAttributedString.Key(kCTFontAttributeName as String): font,
    NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(gray: 1, alpha: 1),
    NSAttributedString.Key(kCTKernAttributeName as String): h * 0.02,
  ])
let line = CTLineCreateWithAttributedString(text)
let glyphs = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
context.textPosition = CGPoint(x: (w - glyphs.width) / 2 - glyphs.minX, y: band.midY - glyphs.height / 2 - glyphs.minY)
CTLineDraw(line, context)

let out = CGImageDestinationCreateWithURL(URL(fileURLWithPath: args[2]) as CFURL, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(out, context.makeImage()!, nil)
guard CGImageDestinationFinalize(out) else { exit(1) }
