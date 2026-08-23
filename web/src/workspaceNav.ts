export type WorkspaceView =
  | 'prices'
  | 'states'
  | 'pops'
  | 'alerts'
  | 'military'
  | 'buildings'
  | 'what-if'
  | 'timeline'
  | 'gaps'
  | 'query'
  | 'archive'

export type MilitaryTab = 'army' | 'navy' | 'mobilization'

/** Buildings workspace sub-view. `building/{id}` is detail, not a tab. */
export type BuildingsTab = 'overview' | 'queues'

export const WORKSPACE_NAV: readonly { view: WorkspaceView; label: string }[] = [
  { view: 'prices', label: 'Prices' },
  { view: 'states', label: 'States' },
  { view: 'pops', label: 'Pops' },
  { view: 'alerts', label: 'Alerts' },
  { view: 'military', label: 'Military' },
  { view: 'buildings', label: 'Buildings' },
  { view: 'what-if', label: 'What-if' },
  { view: 'timeline', label: 'Timeline' },
  { view: 'gaps', label: 'Goal gaps' },
  { view: 'query', label: 'Query' },
  { view: 'archive', label: 'Archive' },
]

const VIEW_IDS = new Set<string>(WORKSPACE_NAV.map((item) => item.view))
const MILITARY_TABS = new Set<string>(['army', 'navy', 'mobilization'])
const BUILDINGS_TABS = new Set<string>(['overview', 'queues'])

export function parseHash(hash = window.location.hash): {
  view?: WorkspaceView
  militaryTab: MilitaryTab
  buildingsTab: BuildingsTab
} {
  const path = hash.replace(/^#\/?/, '').split('/')
  const view = VIEW_IDS.has(path[0]) ? (path[0] as WorkspaceView) : undefined
  const militaryTab =
    view === 'military' && MILITARY_TABS.has(path[1]) ? (path[1] as MilitaryTab) : 'army'
  // `#/buildings/building/{id}` is a detail route — do not treat `building` as a tab.
  let buildingsTab: BuildingsTab = 'overview'
  if (view === 'buildings' && path[1] && BUILDINGS_TABS.has(path[1])) {
    buildingsTab = path[1] as BuildingsTab
  }
  return { view, militaryTab, buildingsTab }
}

export function hashForView(
  view: WorkspaceView,
  militaryTab?: MilitaryTab,
  buildingsTab?: BuildingsTab,
): string {
  if (view === 'military' && militaryTab) return `#/military/${militaryTab}`
  if (view === 'buildings' && buildingsTab && buildingsTab !== 'overview') {
    return `#/buildings/${buildingsTab}`
  }
  return `#/${view}`
}

export function hashForState(id: number): string {
  return `#/states/${id}`
}

export function parseStateId(hash = window.location.hash): number | undefined {
  const path = hash.replace(/^#\/?/, '').split('/')
  if (path[0] !== 'states' || !path[1] || !Number.isFinite(Number(path[1]))) return undefined
  return Number(path[1])
}

export function hashForGood(id: string): string {
  return `#/prices/good/${encodeURIComponent(id)}`
}

export function hashForBuilding(id: number, source: 'prices' | 'buildings' = 'buildings'): string {
  return source === 'prices' ? `#/prices/building/${id}` : `#/buildings/building/${id}`
}

export function parseBuildingId(hash = window.location.hash): number | undefined {
  const path = hash.replace(/^#\/?/, '').split('/')
  if (path[0] !== 'buildings' || path[1] !== 'building' || !path[2] || !Number.isFinite(Number(path[2]))) {
    return undefined
  }
  return Number(path[2])
}
