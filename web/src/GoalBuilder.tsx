import { useEffect, useMemo, useState } from 'react'
import { Modal } from './Modal'

export type GoalKind = 'research' | 'gdp' | 'good-price' | 'declare-war' | 'colonize' | 'advanced'

interface Props {
  goods: string[]
  value: string
  onChange: (goal: string) => void
  idPrefix: string
  initialKind?: GoalKind
}

function cleanId(value: string): string {
  return value.trim().replaceAll(' ', '_')
}

export function GoalBuilder({ goods, value, onChange, idPrefix, initialKind = 'research' }: Props) {
  const [kind, setKind] = useState<GoalKind>(initialKind)
  const [showDslHelp, setShowDslHelp] = useState(false)
  const [technology, setTechnology] = useState('nitroglycerin')
  const [gdp, setGdp] = useState(100_000_000)
  const [good, setGood] = useState(goods[0] ?? 'grain')
  const [relation, setRelation] = useState('<=')
  const [price, setPrice] = useState(20)
  const [state, setState] = useState('alsace')
  const [region, setRegion] = useState('region_congo')

  useEffect(() => {
    if (goods.length > 0 && !goods.includes(good)) setGood(goods[0])
  }, [good, goods])

  const builtGoal = useMemo(() => {
    switch (kind) {
      case 'research':
        return `research(tech=${cleanId(technology)})`
      case 'gdp':
        return `gdp ${'>='} ${gdp}`
      case 'good-price':
        return `good_price(${cleanId(good)}) ${relation} ${price}`
      case 'declare-war':
        // tag= / wargoal= are accepted by the parser for forward compatibility but
        // ignored by compile today — do not collect or emit them from the UI.
        return `declare-war(state=${cleanId(state)})`
      case 'colonize':
        return `colonize(region=${cleanId(region)})`
      case 'advanced':
        return value
    }
  }, [gdp, good, kind, price, region, relation, state, technology, value])

  useEffect(() => {
    if (kind !== 'advanced') onChange(builtGoal)
  }, [builtGoal, kind, onChange])

  return (
    <fieldset className="goal-builder">
      <legend>Goal</legend>
      <label>
        Goal type
        <select
          aria-label={`${idPrefix} goal type`}
          value={kind}
          onChange={(event) => setKind(event.target.value as GoalKind)}
        >
          <option value="research">Research technology</option>
          <option value="gdp">Reach GDP</option>
          <option value="good-price">Reach a goods price</option>
          <option value="declare-war">War readiness (gaps)</option>
          <option value="colonize">Colonize region</option>
          <option value="advanced">Advanced DSL</option>
        </select>
      </label>

      {kind === 'research' && (
        <label>
          Technology
          <input
            aria-label={`${idPrefix} technology`}
            value={technology}
            onChange={(event) => setTechnology(event.target.value)}
            placeholder="nitroglycerin"
          />
        </label>
      )}
      {kind === 'gdp' && (
        <label>
          Target GDP
          <input
            aria-label={`${idPrefix} target GDP`}
            type="number"
            min="0"
            value={gdp}
            onChange={(event) => setGdp(Number(event.target.value))}
          />
        </label>
      )}
      {kind === 'good-price' && (
        <div className="goal-row">
          <label>
            Good
            <select
              aria-label={`${idPrefix} good`}
              value={good}
              onChange={(event) => setGood(event.target.value)}
            >
              {(goods.length > 0 ? goods : [good]).map((id) => (
                <option value={id} key={id}>
                  {id.replaceAll('_', ' ')}
                </option>
              ))}
            </select>
          </label>
          <label>
            Comparison
            <select
              aria-label={`${idPrefix} comparison`}
              value={relation}
              onChange={(event) => setRelation(event.target.value)}
            >
              <option value="<=">At most</option>
              <option value=">=">At least</option>
              <option value="<">Below</option>
              <option value=">">Above</option>
            </select>
          </label>
          <label>
            Price
            <input
              aria-label={`${idPrefix} price`}
              type="number"
              step="0.01"
              value={price}
              onChange={(event) => setPrice(Number(event.target.value))}
            />
          </label>
        </div>
      )}
      {kind === 'declare-war' && (
        <label>
          Target state
          <input
            aria-label={`${idPrefix} target state`}
            value={state}
            onChange={(event) => setState(event.target.value)}
          />
          <span className="field-hint">
            Compiles to interest, army, munitions-price, and solvent. Army PP closes
            via staffed barracks when the economy context is present.
          </span>
        </label>
      )}
      {kind === 'colonize' && (
        <label>
          Target region
          <input
            aria-label={`${idPrefix} target region`}
            value={region}
            onChange={(event) => setRegion(event.target.value)}
          />
          <span className="field-hint">
            Compiles to colonization tech, colonial law, quinine, interest, army/navy
            PP, and solvent. Navy needs staffed shipyards and naval administrations.
          </span>
        </label>
      )}
      {kind === 'advanced' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
          <label>
            Goal DSL
            <textarea
              aria-label={`${idPrefix} advanced goal`}
              value={value}
              onChange={(event) => onChange(event.target.value)}
              rows={3}
            />
          </label>
          <button type="button" onClick={() => setShowDslHelp(true)} style={{ alignSelf: 'flex-start' }}>View DSL Reference</button>
        </div>
      )}

      {showDslHelp && (
        <Modal title="Goal DSL Reference" onClose={() => setShowDslHelp(false)}>
          <div style={{ padding: '1rem', maxHeight: '60vh', overflowY: 'auto' }}>
            <p>Goals are boolean expressions over your country's state. You can use <code>&&</code> (AND), <code>||</code> (OR), and <code>not</code>.</p>
            <h4>Atomic Predicates</h4>
            <ul>
              <li><code>has_tech(id)</code>: E.g. <code>has_tech(nitroglycerin)</code></li>
              <li><code>has_law(id)</code>: E.g. <code>has_law(law_homesteading)</code></li>
              <li><code>good_price(id) &lt;= num</code>: E.g. <code>good_price(ammunition) &lt;= 40</code></li>
              <li><code>army_power_projection &gt;= num</code></li>
              <li><code>navy_power_projection &gt;= num</code></li>
              <li><code>weekly_balance &gt;= num</code></li>
              <li><code>solvent</code>: True when credit_headroom &gt; 0</li>
              <li><code>interest_in(state=id)</code></li>
            </ul>
            <h4>Compound Goals</h4>
            <ul>
              <li><code>declare-war(state=id)</code>: Expands to interest, army PP, munitions price, and solvent.</li>
              <li><code>colonize(region=id)</code>: Expands to tech, laws, interest, army/navy PP, and solvent.</li>
              <li><code>gdp &gt;= num</code>: Evaluates solved building output value.</li>
            </ul>
          </div>
        </Modal>
      )}

      <output className="goal-preview">Goal: {builtGoal}</output>
    </fieldset>
  )
}
