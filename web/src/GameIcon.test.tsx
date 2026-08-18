import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { GameIcon, parseDefsIcons } from './GameIcon'

describe('GameIcon', () => {
  it('renders an img when the icon is present', () => {
    const icons = parseDefsIcons({
      goods: { grain: 'data:image/png;base64,GRAIN' },
      extra: { 'building:building_rye_farm': 'data:image/png;base64,FARM' },
    })
    const { container, rerender } = render(<GameIcon kind="good" id="grain" icons={icons} />)
    const good = container.querySelector('img.good-icon')
    expect(good).toHaveAttribute('src', 'data:image/png;base64,GRAIN')
    expect(good).toHaveAttribute('alt', '')

    rerender(<GameIcon kind="building" id="building_rye_farm" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,FARM',
    )
  })

  it('renders nothing when the icon is absent', () => {
    const { container } = render(<GameIcon kind="good" id="wood" icons={{ goods: {}, extra: {} }} />)
    expect(container.querySelector('img.good-icon')).toBeNull()
  })

  it('looks up a flattened goods map from older wasm JSON', () => {
    const icons = parseDefsIcons({ grain: 'data:image/png;base64,FLAT' })
    const { container } = render(<GameIcon kind="good" id="grain" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,FLAT',
    )
  })
})
