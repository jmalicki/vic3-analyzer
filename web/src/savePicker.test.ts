import { describe, expect, it, vi } from 'vitest'
import {
  canUseRememberedSavePicker,
  detectPlatform,
  pickSaveWithRememberedFolder,
  victoria3GameCommonPaths,
  victoria3SavePaths,
  type Platform,
} from './savePicker'

describe('savePicker paths', () => {
  it.each([
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'windows' as Platform],
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)', 'macos' as Platform],
    ['Mozilla/5.0 (X11; Linux x86_64)', 'linux' as Platform],
  ])('detects %s as %s', (ua, platform) => {
    expect(detectPlatform(ua)).toBe(platform)
  })

  it('returns the Windows documents save path', () => {
    const hint = victoria3SavePaths('windows')
    expect(hint.label).toBe('Usual local folder')
    expect(hint.local).toContain('Documents\\Paradox Interactive\\Victoria 3\\save games')
    expect(hint.summary).toContain('cannot open that path automatically')
    expect(hint.summary).not.toContain(hint.local)
  })

  it('returns the macOS documents save path', () => {
    const hint = victoria3SavePaths('macos')
    expect(hint.local).toBe('~/Documents/Paradox Interactive/Victoria 3/save games')
  })

  it('returns the Linux XDG save path', () => {
    const hint = victoria3SavePaths('linux')
    expect(hint.local).toBe('~/.local/share/Paradox Interactive/Victoria 3/save games')
  })
})

describe('remembered save picker', () => {
  it('reports support only when showOpenFilePicker exists', () => {
    expect(canUseRememberedSavePicker({} as Window)).toBe(false)
    expect(
      canUseRememberedSavePicker({
        showOpenFilePicker: async () => [],
      } as unknown as Window),
    ).toBe(true)
  })

  it('returns the selected file and swallows AbortError', async () => {
    const file = new File(['x'], 'a.v3')
    const win = {
      showOpenFilePicker: vi.fn(async () => [{ getFile: async () => file }]),
    } as unknown as Window
    await expect(pickSaveWithRememberedFolder(win)).resolves.toBe(file)

    const cancelled = {
      showOpenFilePicker: vi.fn(async () => {
        throw new DOMException('cancelled', 'AbortError')
      }),
    } as unknown as Window
    await expect(pickSaveWithRememberedFolder(cancelled)).resolves.toBeUndefined()
  })
})

describe('game common paths', () => {
  it('returns the Windows Steam game path', () => {
    const hint = victoria3GameCommonPaths('windows')
    expect(hint.label).toBe('Usual Steam folder')
    expect(hint.local).toContain('steamapps\\common\\Victoria 3\\game')
    expect(hint.local).not.toMatch(/\\game\\common$/)
    expect(hint.summary).toContain('address bar')
    expect(hint.summary).not.toContain(hint.local)
  })

  it('tells macOS users how to reach the blocked ~/Library path', () => {
    const hint = victoria3GameCommonPaths('macos')
    expect(hint.local).toBe(
      '~/Library/Application Support/Steam/steamapps/common/Victoria 3/game',
    )
    expect(hint.summary).toContain('Cmd+Shift+G')
  })

  it('returns the Linux Steam game path', () => {
    const hint = victoria3GameCommonPaths('linux')
    expect(hint.local).toBe(
      '~/.local/share/Steam/steamapps/common/Victoria 3/game',
    )
    expect(hint.summary).toContain('Ctrl+L')
  })
})
