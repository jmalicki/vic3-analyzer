import type { DefsIcons, GameIconKind } from './types'

function asStringMap(value: unknown): Record<string, string> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const entries = Object.entries(value as Record<string, unknown>).filter(
    (entry): entry is [string, string] => typeof entry[1] === 'string',
  )
  return Object.fromEntries(entries)
}

/** Normalize wasm JSON (nested or flat goods) into [`DefsIcons`]. */
export function parseDefsIcons(raw: unknown): DefsIcons {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const obj = raw as Record<string, unknown>
  const extra = asStringMap(obj.extra) ?? asStringMap(obj.$extra)
  const nestedGoods = asStringMap(obj.goods)
  const goods =
    nestedGoods ??
    asStringMap(
      Object.fromEntries(
        Object.entries(obj).filter(
          ([key, value]) =>
            key !== 'extra' && key !== '$extra' && key !== 'goods' && typeof value === 'string',
        ),
      ),
    )
  return { goods, extra }
}

const SCRIPT_PREFIXES = [
  'pm_',
  'building_',
  'combat_unit_type_',
  'ship_type_',
  'mobilization_option_',
  'silhouette_',
] as const

const ID_ALIASES: Record<string, string[]> = {
  army: ['army_01', 'battalions'],
  navy: ['fleet_01', 'fleet'],
  fleet: ['fleet_01'],
  starvation: ['starving', 'famine'],
  market: ['goods_shortage'],
  market_access: ['world_market_access', 'market_isolated', 'market_over_capacity'],
  qualification: ['literacy'],
  unemployment: ['population'],
}

/** Script ids, texture stems, and a few vanilla filename aliases. */
export function iconLookupKeys(id: string): string[] {
  const keys: string[] = []
  const push = (key: string) => {
    if (key && !keys.includes(key)) keys.push(key)
  }
  push(id)
  for (const alias of ID_ALIASES[id] ?? []) push(alias)
  let stripped = id
  for (const prefix of SCRIPT_PREFIXES) {
    if (stripped.startsWith(prefix)) stripped = stripped.slice(prefix.length)
  }
  if (stripped !== id) {
    push(stripped)
    push(`silhouette_${stripped}`)
  }
  return keys
}

function lookupIcon(kind: GameIconKind, id: string, icons?: DefsIcons | null): string | undefined {
  if (!icons) return undefined
  for (const key of iconLookupKeys(id)) {
    const hit =
      icons.extra?.[`${kind}:${key}`] ??
      icons.extra?.[`generic:${key}`] ??
      icons.extra?.[key] ??
      ((kind === 'good' || kind === 'alert')
        ? (icons.goods?.[key] ??
          icons.extra?.[`good:${key}`] ??
          (typeof icons[key] === 'string' ? icons[key] : undefined))
        : undefined)
    if (hit) return hit
  }
  if (kind === 'military' && /combat_unit|infantry|artillery|cavalry|battalion|^army$/i.test(id)) {
    return icons.extra?.['military:battalions'] ?? icons.extra?.['generic:battalions']
  }
  return undefined
}

export function GameIcon({
  kind,
  id,
  icons,
}: {
  kind: GameIconKind
  id: string
  icons?: DefsIcons | null
}) {
  const src = lookupIcon(kind, id, icons)
  if (!src) return null
  return <img className="good-icon" src={src} alt="" width={24} height={24} />
}
