import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SettingsPane } from './SettingsPane'
import { invokeTauri } from './tauriApi'

vi.mock('./tauriApi', () => ({
  invokeTauri: vi.fn(),
}))

const invoke = vi.mocked(invokeTauri)

const sampleConfig = {
  game_dir: '/games/Victoria 3/game',
  defs_blob: null,
  save_dirs: ['/saves'],
  tokens_path: null,
  auto_detect: true,
  config_path: '/app/config.toml',
}

describe('SettingsPane', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockImplementation(async (cmd) => {
      if (cmd === 'get_config') return sampleConfig
      if (cmd === 'save_config') return { ...sampleConfig, game_dir: '/custom/game' }
      if (cmd === 'reset_config') return { ...sampleConfig, game_dir: null, auto_detect: true }
      throw new Error(`unexpected ${cmd}`)
    })
  })

  afterEach(() => {
    cleanup()
  })

  it('loads config into the form', async () => {
    render(<SettingsPane />)
    await waitFor(() => {
      expect(screen.getByLabelText(/Game folder/i)).toHaveValue('/games/Victoria 3/game')
    })
    expect(screen.getByLabelText(/Save folders/i)).toHaveValue('/saves')
    expect(screen.getByText('/app/config.toml')).toBeInTheDocument()
  })

  it('saves config and reports status', async () => {
    const user = userEvent.setup()
    const onConfigChange = vi.fn()
    render(<SettingsPane onConfigChange={onConfigChange} />)
    await waitFor(() => expect(screen.getByLabelText(/Game folder/i)).toBeEnabled())

    await user.clear(screen.getByLabelText(/Game folder/i))
    await user.type(screen.getByLabelText(/Game folder/i), '/custom/game')
    await user.click(screen.getByRole('button', { name: /Save config/i }))

    await waitFor(() => {
      expect(screen.getByText('Saved')).toBeInTheDocument()
    })
    expect(invoke).toHaveBeenCalledWith(
      'save_config',
      expect.objectContaining({
        config: expect.objectContaining({ game_dir: '/custom/game' }),
      }),
    )
    expect(onConfigChange).toHaveBeenCalled()
  })
})
