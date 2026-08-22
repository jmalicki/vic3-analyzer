/**
 * Pixel-compare generated screenshots to committed goldens in docs/assets/.
 *
 * Policy (a): if a golden PNG is missing, skip that file (success).
 * If a golden exists, fail when the generated image drifts beyond threshold.
 *
 * Expects generators to have written into DOCS_SCREENSHOTS_OUT (or docs/assets).
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import pixelmatch from 'pixelmatch'
import { PNG } from 'pngjs'
import { ALL_SHOTS, assetsDir, outDir } from './lib/paths.mjs'

const THRESHOLD = Number(process.env.DOCS_SCREENSHOTS_THRESHOLD || 0.1)
const MAX_DIFF_RATIO = Number(process.env.DOCS_SCREENSHOTS_MAX_DIFF_RATIO || 0.02)

function compareOne(name, generatedDir, goldenDir, diffDir) {
  const goldenPath = join(goldenDir, name)
  const actualPath = join(generatedDir, name)

  if (!existsSync(goldenPath)) {
    console.log(`skip ${name}: no committed golden`)
    return { status: 'skipped' }
  }
  if (!existsSync(actualPath)) {
    console.error(`fail ${name}: golden exists but generator did not produce ${actualPath}`)
    return { status: 'missing' }
  }

  // When generating into docs/assets itself, skip byte-identical self-compare noise
  // by still running pixelmatch (same file → 0 diff).
  const imgA = PNG.sync.read(readFileSync(goldenPath))
  const imgB = PNG.sync.read(readFileSync(actualPath))
  if (imgA.width !== imgB.width || imgA.height !== imgB.height) {
    console.error(
      `fail ${name}: size ${imgB.width}x${imgB.height} vs golden ${imgA.width}x${imgA.height}`,
    )
    return { status: 'drift' }
  }

  const diff = new PNG({ width: imgA.width, height: imgA.height })
  const mismatched = pixelmatch(imgA.data, imgB.data, diff.data, imgA.width, imgA.height, {
    threshold: THRESHOLD,
  })
  const ratio = mismatched / (imgA.width * imgA.height)
  if (ratio > MAX_DIFF_RATIO) {
    mkdirSync(diffDir, { recursive: true })
    const diffPath = join(diffDir, name.replace(/\.png$/, '-diff.png'))
    writeFileSync(diffPath, PNG.sync.write(diff))
    console.error(
      `fail ${name}: ${(ratio * 100).toFixed(3)}% pixels differ (max ${(MAX_DIFF_RATIO * 100).toFixed(2)}%) → ${diffPath}`,
    )
    return { status: 'drift' }
  }
  console.log(`ok ${name}: ${(ratio * 100).toFixed(4)}% diff`)
  return { status: 'ok' }
}

function main() {
  const generated = outDir()
  const golden = assetsDir
  const diffDir = join(generated, '_diff')

  // When OUT === assets and no goldens yet, every file is "skip" after generate — fine.
  // When OUT === assets and goldens were just overwritten by generate, compare is tautological.
  // CI should set DOCS_SCREENSHOTS_OUT to a scratch directory so compare hits committed goldens.
  if (generated === golden && process.env.CI === 'true') {
    console.warn(
      'warning: DOCS_SCREENSHOTS_OUT unset on CI; compare may be a no-op against overwritten assets',
    )
  }

  let failed = 0
  let compared = 0
  let skipped = 0

  for (const name of ALL_SHOTS) {
    const result = compareOne(name, generated, golden, diffDir)
    if (result.status === 'skipped') skipped += 1
    else if (result.status === 'ok') compared += 1
    else failed += 1
  }

  console.log(
    `compare done: ${compared} ok, ${skipped} skipped (no golden), ${failed} failed`,
  )
  if (failed > 0) process.exit(1)
}

main()
