import { useEffect, useMemo, useState } from 'react'

export type GoalKind = 'research' | 'gdp' | 'good-price' | 'declare-war' | 'advanced'

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
  const [technology, setTechnology] = useState('nitroglycerin')
  const [gdp, setGdp] = useState(100_000_000)
  const [good, setGood] = useState(goods[0] ?? 'grain')
  const [relation, setRelation] = useState('<=')
  const [price, setPrice] = useState(20)
  const [country, setCountry] = useState('FRA')
  const [state, setState] = useState('alsace')

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
        return `declare-war(tag=${cleanId(country)}, wargoal=conquer_state, state=${cleanId(state)})`
      case 'advanced':
        return value
    }
  }, [country, gdp, good, kind, price, relation, state, technology, value])

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
          <option value="declare-war">Prepare to declare war</option>
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
        <div className="goal-row">
          <label>
            Target country tag
            <input
              aria-label={`${idPrefix} target country`}
              value={country}
              maxLength={3}
              onChange={(event) => setCountry(event.target.value.toUpperCase())}
            />
          </label>
          <label>
            Target state
            <input
              aria-label={`${idPrefix} target state`}
              value={state}
              onChange={(event) => setState(event.target.value)}
            />
          </label>
        </div>
      )}
      {kind === 'advanced' && (
        <label>
          Goal DSL
          <textarea
            aria-label={`${idPrefix} advanced goal`}
            value={value}
            onChange={(event) => onChange(event.target.value)}
            rows={3}
          />
        </label>
      )}

      <output className="goal-preview">Goal: {builtGoal}</output>
    </fieldset>
  )
}
