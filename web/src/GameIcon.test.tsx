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

  it('aliases script ids onto texture stems and generic icons', () => {
    const icons = parseDefsIcons({
      goods: { electricity: 'data:image/png;base64,ELEC' },
      extra: {
        'pm:pm_bakery': 'data:image/png;base64,BAKE',
        'military:silhouette_frigate': 'data:image/png;base64,SHIP',
        'military:chocolate': 'data:image/png;base64,CHOC',
        'generic:battalions': 'data:image/png;base64,BAT',
        'generic:population': 'data:image/png;base64,POP',
        'alert:starving': 'data:image/png;base64,STARVE',
      },
    })
    const { container, rerender } = render(<GameIcon kind="pm" id="pm_bakery" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,BAKE',
    )

    rerender(<GameIcon kind="military" id="ship_type_frigate" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,SHIP',
    )

    rerender(<GameIcon kind="military" id="mobilization_option_chocolate" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,CHOC',
    )

    rerender(<GameIcon kind="military" id="combat_unit_type_line_infantry" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,BAT',
    )

    rerender(<GameIcon kind="alert" id="electricity" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,ELEC',
    )

    rerender(<GameIcon kind="alert" id="starvation" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,STARVE',
    )

    rerender(<GameIcon kind="alert" id="population" icons={icons} />)
    expect(container.querySelector('img.good-icon')).toHaveAttribute(
      'src',
      'data:image/png;base64,POP',
    )
  })
})
