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

function lookupIcon(kind: GameIconKind, id: string, icons?: DefsIcons | null): string | undefined {
  if (!icons) return undefined
  if (kind === 'good') {
    return (
      icons.goods?.[id] ??
      icons.extra?.[`good:${id}`] ??
      icons.extra?.[id] ??
      (typeof icons[id] === 'string' ? icons[id] : undefined)
    )
  }
  return icons.extra?.[`${kind}:${id}`] ?? icons.extra?.[id]
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
