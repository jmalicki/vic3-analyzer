/**
 * Capture the frontmost Vic3 Analyzer macOS window (including title-bar chrome).
 * Requires Screen Recording permission for the parent process.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

const SWIFT = `
import Cocoa

let disallowedOwners = ["cursor", "electron", "code", "terminal", "iterm", "antigravity", "safari", "chrome"]
let allowedOwners = ["vic3-analyzer", "vic3_analyzer", "victoria 3 analyzer"]
let allowedTitles = ["victoria 3 analyzer & planner", "victoria 3 analyzer", "vic3 analyzer", "vic3-analyzer"]

let opts = CGWindowListOption(arrayLiteral: .optionOnScreenOnly, .excludeDesktopElements)
guard let info = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] else {
  fputs("no window list\\n", stderr)
  exit(1)
}

var best: (area: CGFloat, id: Int)?
for w in info {
  let layer = w[kCGWindowLayer as String] as? Int ?? -1
  if layer != 0 { continue }
  let owner = (w[kCGWindowOwnerName as String] as? String) ?? ""
  let title = (w[kCGWindowName as String] as? String) ?? ""
  let ownerLower = owner.lowercased()
  let titleLower = title.lowercased()

  if disallowedOwners.contains(where: { ownerLower.contains($0) }) { continue }

  let ownerMatch = allowedOwners.contains(where: { ownerLower.contains($0) })
  let titleMatch = allowedTitles.contains(where: { titleLower.contains($0) })
  guard ownerMatch || titleMatch else { continue }

  let bounds = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
  let width = bounds["Width"] as? CGFloat ?? 0
  let height = bounds["Height"] as? CGFloat ?? 0
  let area = width * height
  if area < 10_000 { continue }
  guard let wid = w[kCGWindowNumber as String] as? Int else { continue }
  if best == nil || area > best!.area {
    best = (area, wid)
  }
}

guard let hit = best else {
  fputs("no Vic3 Analyzer window found\\n", stderr)
  exit(1)
}
print(hit.id)
`

const helperBin = join(tmpdir(), 'vic3-docs-find-window')

function ensureHelper() {
  if (existsSync(helperBin)) return helperBin
  const src = join(tmpdir(), 'vic3-docs-find-window.swift')
  writeFileSync(src, SWIFT)
  execFileSync('swiftc', ['-O', '-o', helperBin, src], { stdio: 'inherit' })
  return helperBin
}

function findWindowId() {
  return execFileSync(ensureHelper(), [], { encoding: 'utf8' }).trim()
}

/**
 * @param {string} outPath
 */
export function captureMacWindow(outPath) {
  mkdirSync(dirname(outPath), { recursive: true })
  const wid = findWindowId()
  execFileSync('screencapture', ['-x', '-l', wid, outPath], { stdio: 'inherit' })
  return outPath
}
