/** OS family used for Victoria 3 save-path hints. */
export type Platform = 'windows' | 'macos' | 'linux' | 'unknown'

export type SavePathHint = {
  platform: Platform
  /** Short lead-in before the path, e.g. "Usual local folder". */
  label: string
  /** Typical local Paradox documents path for this platform. */
  local: string
  /** Typical Steam Cloud cache path when applicable. */
  steamCloud?: string
  /** Browser caveat shown under the path. */
  summary: string
}

const SAVE_PICKER_ID = 'vic3-analyzer-save'

type OpenFilePickerWindow = Window & {
  showOpenFilePicker?: (options?: {
    id?: string
    multiple?: boolean
    excludeAcceptAllOption?: boolean
    startIn?: 'desktop' | 'documents' | 'downloads' | 'music' | 'pictures' | 'videos'
    types?: Array<{
      description?: string
      accept: Record<string, string[]>
    }>
  }) => Promise<Array<{ getFile: () => Promise<File> }>>
}

/** Detect the OS family from a user-agent string (or navigator). */
export function detectPlatform(userAgent = globalThis.navigator?.userAgent ?? ''): Platform {
  const ua = userAgent.toLowerCase()
  if (ua.includes('win')) return 'windows'
  if (ua.includes('mac')) return 'macos'
  if (ua.includes('linux') || ua.includes('x11')) return 'linux'
  return 'unknown'
}

/** Return the usual Victoria 3 save locations for a platform. */
export function victoria3SavePaths(platform: Platform = detectPlatform()): SavePathHint {
  const remembered =
    'Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.'
  switch (platform) {
    case 'windows':
      return {
        platform,
        label: 'Usual local folder',
        local: 'Documents\\Paradox Interactive\\Victoria 3\\save games',
        steamCloud: 'Steam\\userdata\\<id>\\529340\\remote\\save games',
        summary: remembered,
      }
    case 'macos':
      return {
        platform,
        label: 'Usual local folder',
        local: '~/Documents/Paradox Interactive/Victoria 3/save games',
        steamCloud: '~/Library/Application Support/Steam/userdata/<id>/529340/remote/save games',
        summary: remembered,
      }
    case 'linux':
      return {
        platform,
        label: 'Usual local folder',
        local: '~/.local/share/Paradox Interactive/Victoria 3/save games',
        steamCloud: '~/.steam/steam/userdata/<id>/529340/remote/save games',
        summary: remembered,
      }
    default:
      return {
        platform,
        label: 'Usual local folder',
        local: 'Paradox Interactive/Victoria 3/save games',
        summary:
          'Look in your Documents (or Linux game-data) folder. Browsers cannot open that path automatically.',
      }
  }
}

/** True when the File System Access API open picker is available. */
export function canUseRememberedSavePicker(
  win: OpenFilePickerWindow = globalThis.window as OpenFilePickerWindow,
): boolean {
  return typeof win?.showOpenFilePicker === 'function'
}

/**
 * Open a Chromium file picker that can remember the last approved folder.
 * Returns `undefined` when unsupported or the user cancels.
 */
export async function pickSaveWithRememberedFolder(
  win: OpenFilePickerWindow = globalThis.window as OpenFilePickerWindow,
): Promise<File | undefined> {
  if (!canUseRememberedSavePicker(win) || !win.showOpenFilePicker) return undefined
  try {
    const [handle] = await win.showOpenFilePicker({
      id: SAVE_PICKER_ID,
      multiple: false,
      excludeAcceptAllOption: true,
      startIn: 'documents',
      types: [
        {
          description: 'Victoria 3 save',
          accept: { 'application/octet-stream': ['.v3'] },
        },
      ],
    })
    return await handle.getFile()
  } catch (reason) {
    if (reason instanceof DOMException && reason.name === 'AbortError') return undefined
    throw reason
  }
}

export type GamePathHint = {
  platform: Platform
  /** Short lead-in before the path, e.g. "Usual Steam folder". */
  label: string
  /** Typical Steam `game` path for this platform. */
  local: string
  /** Browser caveat shown under the path. */
  summary: string
}

/**
 * Return the usual Steam Victoria 3 `game` locations for a platform.
 *
 * These paths are shown, never fed to a picker API: Chromium's File System
 * Access blocklist marks `~/Library` (macOS) and `Program Files` (Windows) as
 * block-all-children, so `showDirectoryPicker` cannot open a Steam install
 * there. The plain `webkitdirectory` input has no such restriction, so the
 * hint tells users how to reach the path in the native dialog.
 */
export function victoria3GamePaths(
  platform: Platform = detectPlatform(),
): GamePathHint {
  switch (platform) {
    case 'windows':
      return {
        platform,
        label: 'Usual Steam folder',
        local: 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Victoria 3\\game',
        summary:
          'Chrome cannot open that path for you. In the folder dialog, paste the path into the address bar.',
      }
    case 'macos':
      return {
        platform,
        label: 'Usual Steam folder',
        local: '~/Library/Application Support/Steam/steamapps/common/Victoria 3/game',
        summary:
          'Finder hides ~/Library, and Chrome blocks it in the newer folder API. In the folder dialog press Cmd+Shift+G, then paste the path.',
      }
    case 'linux':
      return {
        platform,
        label: 'Usual Steam folder',
        local: '~/.local/share/Steam/steamapps/common/Victoria 3/game',
        summary:
          'Also try ~/.steam/steam/steamapps/…. In the folder dialog press Ctrl+L, then paste the path.',
      }
    default:
      return {
        platform,
        label: 'Usual Steam folder',
        local: 'Steam/steamapps/common/Victoria 3/game',
        summary: 'Look under your Steam library, then paste the path in the folder dialog.',
      }
  }
}

export { SAVE_PICKER_ID }
