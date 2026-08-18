import { GameIcon } from './GameIcon'
import type {
  DefsIcons,
  MilitaryFormationSnapshot,
  MilitarySnapshot,
  MilitaryUnitSnapshot,
  MobilizationSnapshot,
} from './types'
import type { MilitaryTab } from './workspaceNav'

function formationName(formation: MilitaryFormationSnapshot, fallback: string): string {
  return formation.name?.trim() || `${fallback} ${formation.id}`
}

function unitName(unit: MilitaryUnitSnapshot, index: number): string {
  return unit.name?.trim() || unit.type?.trim() || `Unit ${unit.id ?? index + 1}`
}

function mobilizationName(entry: MobilizationSnapshot): string {
  return entry.name?.trim() || entry.type?.trim() || `Mobilization ${entry.id}`
}

function FormationCard({
  formation,
  fallbackName,
  icons,
}: {
  formation: MilitaryFormationSnapshot
  fallbackName: string
  icons?: DefsIcons
}) {
  const iconId = formation.type || fallbackName.toLowerCase()
  const facts: string[] = []
  if (formation.current_manpower != null) {
    facts.push(`${formation.current_manpower.toLocaleString()} manpower`)
  }
  if (formation.organization != null) {
    facts.push(`${formation.organization.toLocaleString()} organization`)
  }
  if (formation.units.length > 0) {
    facts.push(`${formation.units.length.toLocaleString()} ${formation.units.length === 1 ? 'unit' : 'units'}`)
  }
  return (
    <li className="military-card">
      <div className="military-heading">
        <GameIcon kind="military" id={iconId} icons={icons} />
        <strong>{formationName(formation, fallbackName)}</strong>
      </div>
      {facts.length > 0 && <p className="military-meta">{facts.join(' · ')}</p>}
      {formation.units.length > 0 && (
        <ul className="military-units">
          {formation.units.map((unit, index) => (
            <li key={unit.id ?? `${formation.id}-${index}`}>
              {unit.type && <GameIcon kind="military" id={unit.type} icons={icons} />}
              <span>{unitName(unit, index)}</span>
              {unit.manpower != null && (
                <span className="military-unit-manpower">{unit.manpower.toLocaleString()}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </li>
  )
}

function FormationList({
  formations,
  emptyMessage,
  fallbackName,
  icons,
}: {
  formations: MilitaryFormationSnapshot[]
  emptyMessage: string
  fallbackName: string
  icons?: DefsIcons
}) {
  if (formations.length === 0) {
    return <p>{emptyMessage}</p>
  }
  return (
    <ul className="military-list">
      {formations.map((formation) => (
        <FormationCard
          key={formation.id}
          formation={formation}
          fallbackName={fallbackName}
          icons={icons}
        />
      ))}
    </ul>
  )
}

function MobilizationList({
  entries,
  icons,
}: {
  entries: MobilizationSnapshot[]
  icons?: DefsIcons
}) {
  if (entries.length === 0) {
    return <p>None recorded</p>
  }
  return (
    <ul className="military-list">
      {entries.map((entry) => (
        <li key={entry.id} className="military-card">
          <div className="military-heading">
            {entry.type && <GameIcon kind="military" id={entry.type} icons={icons} />}
            <strong>{mobilizationName(entry)}</strong>
          </div>
          {entry.type && entry.name && <p className="military-meta">{entry.type}</p>}
        </li>
      ))}
    </ul>
  )
}

export function MilitaryPane({
  snapshot,
  tab,
  icons,
}: {
  snapshot: MilitarySnapshot
  tab: MilitaryTab
  icons?: DefsIcons
}) {
  return (
    <div className="military-pane">
      {tab === 'army' && (
        <FormationList
          formations={snapshot.armies}
          emptyMessage="No armies recorded in this save."
          fallbackName="Army"
          icons={icons}
        />
      )}
      {tab === 'navy' && (
        <FormationList
          formations={snapshot.navies}
          emptyMessage="No navies recorded in this save."
          fallbackName="Navy"
          icons={icons}
        />
      )}
      {tab === 'mobilization' && <MobilizationList entries={snapshot.mobilization} icons={icons} />}
      {snapshot.limitations.map((line) => (
        <p key={line} className="model-info">
          {line}
        </p>
      ))}
    </div>
  )
}
