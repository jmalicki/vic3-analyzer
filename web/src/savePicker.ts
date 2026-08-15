/** OS family used for Victoria 3 save-path hints. */
export type Platform = 'windows' | 'macos' | 'linux' | 'unknown'

export type SavePathHint = {
  platform: Platform
  /** Typical local Paradox documents path for this platform. */
  local: string
  /** Typical Steam Cloud cache path when applicable. */
  steamCloud?: string
  /** Short copy shown under the save picker. */
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
  switch (platform) {
    case 'windows':
      return {
        platform,
        local: 'Documents\\Paradox Interactive\\Victoria 3\\save games',
        steamCloud: 'Steam\\userdata\\<id>\\529340\\remote\\save games',
        summary:
          'Usual local folder: Documents\\Paradox Interactive\\Victoria 3\\save games. Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    case 'macos':
      return {
        platform,
        local: '~/Documents/Paradox Interactive/Victoria 3/save games',
        steamCloud: '~/Library/Application Support/Steam/userdata/<id>/529340/remote/save games',
        summary:
          'Usual local folder: ~/Documents/Paradox Interactive/Victoria 3/save games. Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    case 'linux':
      return {
        platform,
        local: '~/.local/share/Paradox Interactive/Victoria 3/save games',
        steamCloud: '~/.steam/steam/userdata/<id>/529340/remote/save games',
        summary:
          'Usual local folder: ~/.local/share/Paradox Interactive/Victoria 3/save games. Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    default:
      return {
        platform,
        local: 'Paradox Interactive/Victoria 3/save games',
        summary:
          'Look under Paradox Interactive/Victoria 3/save games in your Documents (or Linux game-data) folder. Browsers cannot open that path automatically.',
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

export type GameCommonPathHint = {
  platform: Platform
  /** Typical Steam `game/common` path for this platform. */
  local: string
  /** Short copy shown under the definitions folder picker. */
  summary: string
}

const GAME_COMMON_PICKER_ID = 'vic3-analyzer-game-common'

type DirectoryPickerWindow = Window & {
  showDirectoryPicker?: (options?: {
    id?: string
    mode?: 'read' | 'readwrite'
    startIn?: 'desktop' | 'documents' | 'downloads' | 'music' | 'pictures' | 'videos'
  }) => Promise<FileSystemDirectoryHandle>
}

/** Return the usual Steam Victoria 3 `game/common` locations for a platform. */
export function victoria3GameCommonPaths(
  platform: Platform = detectPlatform(),
): GameCommonPathHint {
  switch (platform) {
    case 'windows':
      return {
        platform,
        local:
          'C:\\Program Files (x86)\\Steam\\steamapps\\common\\Victoria 3\\game\\common',
        summary:
          'Usual Steam folder: C:\\Program Files (x86)\\Steam\\steamapps\\common\\Victoria 3\\game\\common. Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    case 'macos':
      return {
        platform,
        local:
          '~/Library/Application Support/Steam/steamapps/common/Victoria 3/game/common',
        summary:
          'Usual Steam folder: ~/Library/Application Support/Steam/steamapps/common/Victoria 3/game/common. Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    case 'linux':
      return {
        platform,
        local: '~/.local/share/Steam/steamapps/common/Victoria 3/game/common',
        summary:
          'Usual Steam folder: ~/.local/share/Steam/steamapps/common/Victoria 3/game/common (or ~/.steam/steam/steamapps/…). Browsers cannot open that path automatically; Chromium may remember the folder after you choose it once.',
      }
    default:
      return {
        platform,
        local: 'Steam/steamapps/common/Victoria 3/game/common',
        summary:
          'Look under Steam/steamapps/common/Victoria 3/game/common in your Steam library. Browsers cannot open that path automatically.',
      }
  }
}

/** True when the File System Access API directory picker is available. */
export function canUseRememberedDirectoryPicker(
  win: DirectoryPickerWindow = globalThis.window as DirectoryPickerWindow,
): boolean {
  return typeof win?.showDirectoryPicker === 'function'
}

/**
 * Open a Chromium directory picker that can remember the last approved folder.
 * Returns collected definition files, or `undefined` when unsupported/cancelled.
 */
export async function pickGameCommonWithRememberedFolder(
  win: DirectoryPickerWindow = globalThis.window as DirectoryPickerWindow,
): Promise<Array<{ path: string; file: File }> | undefined> {
  if (!canUseRememberedDirectoryPicker(win) || !win.showDirectoryPicker) {
    return undefined
  }
  try {
    const root = await win.showDirectoryPicker({
      id: GAME_COMMON_PICKER_ID,
      mode: 'read',
    })
    return await collectDirectoryFiles(root)
  } catch (reason) {
    if (reason instanceof DOMException && reason.name === 'AbortError') {
      return undefined
    }
    throw reason
  }
}

async function collectDirectoryFiles(
  handle: FileSystemDirectoryHandle,
  prefix = '',
): Promise<Array<{ path: string; file: File }>> {
  const out: Array<{ path: string; file: File }> = []
  for await (const [name, entry] of handle.entries()) {
    const path = prefix ? `${prefix}/${name}` : name
    if (entry.kind === 'directory') {
      out.push(...(await collectDirectoryFiles(entry, path)))
    } else if (entry.kind === 'file') {
      out.push({ path, file: await entry.getFile() })
    }
  }
  return out
}

export { SAVE_PICKER_ID, GAME_COMMON_PICKER_ID }
