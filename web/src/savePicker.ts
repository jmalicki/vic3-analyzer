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

export { SAVE_PICKER_ID }
