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
  | 'archive'

export type MilitaryTab = 'army' | 'navy' | 'mobilization'

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
  { view: 'archive', label: 'Archive' },
]

const VIEW_IDS = new Set<string>(WORKSPACE_NAV.map((item) => item.view))
const MILITARY_TABS = new Set<string>(['army', 'navy', 'mobilization'])

export function parseHash(hash = window.location.hash): {
  view?: WorkspaceView
  militaryTab: MilitaryTab
} {
  const path = hash.replace(/^#\/?/, '').split('/')
  const view = VIEW_IDS.has(path[0]) ? (path[0] as WorkspaceView) : undefined
  const militaryTab =
    view === 'military' && MILITARY_TABS.has(path[1]) ? (path[1] as MilitaryTab) : 'army'
  return { view, militaryTab }
}

export function hashForView(view: WorkspaceView, militaryTab?: MilitaryTab): string {
  if (view === 'military' && militaryTab) return `#/military/${militaryTab}`
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
