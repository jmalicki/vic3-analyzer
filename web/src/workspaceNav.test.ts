import { describe, expect, it } from 'vitest'
import { hashForView, parseHash } from './workspaceNav'

describe('workspaceNav', () => {
  it('parses known pane hashes including nested price and military paths', () => {
    expect(parseHash('#/states').view).toBe('states')
    expect(parseHash('#/states/12').view).toBe('states')
    expect(parseHash('#/prices/good/iron').view).toBe('prices')
    expect(parseHash('#/prices/state/1').view).toBe('prices')
    expect(parseHash('#/prices/building/4').view).toBe('prices')
    expect(parseHash('#/buildings/building/9').view).toBe('buildings')
    expect(parseHash('#/military/navy')).toEqual({ view: 'military', militaryTab: 'navy' })
    expect(parseHash('#/military/mobilization').militaryTab).toBe('mobilization')
    expect(parseHash('#/unknown').view).toBeUndefined()
    expect(parseHash('').view).toBeUndefined()
  })

  it('builds pane hashes', () => {
    expect(hashForView('gaps')).toBe('#/gaps')
    expect(hashForView('what-if')).toBe('#/what-if')
    expect(hashForView('military')).toBe('#/military')
    expect(hashForView('military', 'army')).toBe('#/military/army')
  })
})
