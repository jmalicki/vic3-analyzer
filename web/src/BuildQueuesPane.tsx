/**
 * Buildings → Queues: government / private construction lists like the game.
 *
 * Data is the full queue from `loaded_constructions`, not the single planning
 * head `queued_building`.
 */
import { useState } from 'react'
import { GameIcon } from './GameIcon'
import type { ConstructionOrderSnapshot, ConstructionsSnapshot, DefsIcons } from './types'

export type ConstructionQueueTab = 'government' | 'private'

function displayBuilding(order: ConstructionOrderSnapshot): string {
  return (
    order.building_name?.trim() ||
    order.building.replace(/^building_/, '').replace(/_/g, ' ')
  )
}

function OrderCard({ order, icons }: { order: ConstructionOrderSnapshot; icons?: DefsIcons }) {
  const facts: string[] = []
  if (order.state_label) facts.push(order.state_label)
  else if (order.state_id != null) facts.push(`State ${order.state_id}`)
  if (order.remaining != null) facts.push(`${order.remaining.toLocaleString()} remaining`)
  return (
    <li className="military-card">
      <div className="military-heading">
        <GameIcon kind="building" id={order.building} icons={icons} />
        <strong>{displayBuilding(order)}</strong>
      </div>
      {facts.length > 0 && <p className="military-meta">{facts.join(' · ')}</p>}
    </li>
  )
}

export function BuildQueuesPane({
  snapshot,
  icons,
}: {
  snapshot: ConstructionsSnapshot
  icons?: DefsIcons
}) {
  const [queueTab, setQueueTab] = useState<ConstructionQueueTab>('government')
  const orders = queueTab === 'government' ? snapshot.government : snapshot.private
  const emptyMessage =
    queueTab === 'government'
      ? 'No government constructions in the queue.'
      : 'No private constructions in the queue.'

  return (
    <div className="military-pane">
      <div className="state-tabs" role="tablist" aria-label="Construction queue">
        {(['government', 'private'] as const).map((tab) => (
          <button
            type="button"
            key={tab}
            role="tab"
            aria-selected={queueTab === tab}
            onClick={() => setQueueTab(tab)}
          >
            {tab === 'government' ? 'Government' : 'Private'}
          </button>
        ))}
      </div>
      {orders.length === 0 ? (
        <p>{emptyMessage}</p>
      ) : (
        <ul className="military-list">
          {orders.map((order) => (
            <OrderCard key={`${order.queue}-${order.id}`} order={order} icons={icons} />
          ))}
        </ul>
      )}
    </div>
  )
}
