import { useEffect, useState, type DragEvent, type FormEvent } from 'react'
import whatIfSchemaJson from '../../schema/what-if.json'
import './App.css'
import { listAnalyses, saveAnalysis } from './archive'
import { SchemaForm } from './SchemaForm'
import type {
  AnalysisKind,
  AnalysisRecord,
  JsonSchema,
  PlanAction,
  PlanResult,
  PricesResult,
  SaveSummary,
} from './types'
import { loadWasm, parseSchema, type WasmApi } from './wasm'

interface Props {
  wasmApi?: WasmApi | Promise<WasmApi>
}

const fallbackSchema = whatIfSchemaJson as JsonSchema

function initialValue(schema: JsonSchema): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(schema.properties ?? {}).map(([name, field]) => {
      if (field.type === 'object' || field.properties) return [name, initialValue(field)]
      if (field.default !== undefined) return [name, field.default]
      if (field.type === 'integer' || field.type === 'number') return [name, field.minimum ?? 0]
      return [name, '']
    }),
  )
}

async function bytes(file?: File): Promise<Uint8Array | undefined> {
  return file ? new Uint8Array(await file.arrayBuffer()) : undefined
}

async function fingerprint(data: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', data.slice().buffer)
  return [...new Uint8Array(digest)].map((part) => part.toString(16).padStart(2, '0')).join('')
}

function newId(): string {
  return crypto.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function actionLabel(action: PlanAction): string {
  if ('QueueTech' in action) return `Queue technology: ${action.QueueTech.tech}`
  return `Wait ${action.WaitForEvent.days} days for ${action.WaitForEvent.event.TechCompleted.tech}`
}

function App({ wasmApi }: Props) {
  const [api, setApi] = useState<WasmApi>()
  const [saveFile, setSaveFile] = useState<File>()
  const [tokensFile, setTokensFile] = useState<File>()
  const [defsFile, setDefsFile] = useState<File>()
  const [summary, setSummary] = useState<SaveSummary>()
  const [result, setResult] = useState<PricesResult>()
  const [planResult, setPlanResult] = useState<PlanResult>()
  const [goal, setGoal] = useState('research(tech=nitroglycerin)')
  const [label, setLabel] = useState('')
  const [schema, setSchema] = useState<JsonSchema>(fallbackSchema)
  const [whatIfOpts, setWhatIfOpts] = useState<Record<string, unknown>>(() =>
    initialValue(fallbackSchema),
  )
  const [records, setRecords] = useState<AnalysisRecord[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()

  useEffect(() => {
    void listAnalyses().then(setRecords)
    void Promise.resolve(wasmApi ?? loadWasm())
      .then((loaded) => {
        setApi(loaded)
        const loadedSchema = parseSchema(loaded.what_if_schema())
        setSchema(loadedSchema)
        setWhatIfOpts(initialValue(loadedSchema))
      })
      .catch(() => {
        if (wasmApi) setError('Could not load the analysis engine.')
      })
  }, [wasmApi])

  useEffect(() => {
    if (!api || !saveFile) {
      setSummary(undefined)
      return
    }
    let cancelled = false
    void Promise.all([bytes(saveFile), bytes(tokensFile)])
      .then(async ([saveBytes, tokenBytes]) => {
        const json = await api.parse_save(saveBytes!, tokenBytes)
        if (!cancelled) setSummary(JSON.parse(json) as SaveSummary)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [api, saveFile, tokensFile])

  const archiveResult = async (
    kind: AnalysisKind,
    opts: Record<string, unknown>,
    analysisResult: PricesResult | PlanResult,
    saveBytes: Uint8Array,
    tokenBytes?: Uint8Array,
    recordLabel?: string,
  ) => {
    const record: AnalysisRecord = {
      id: newId(),
      created_at: new Date().toISOString(),
      label: recordLabel || undefined,
      kind,
      fingerprint: await fingerprint(saveBytes),
      date: summary?.date,
      country: summary?.tag,
      filename: saveFile?.name,
      opts,
      result: analysisResult,
      limitations: analysisResult.limitations,
      blob: { save: saveBytes, tokens: tokenBytes },
    }
    await saveAnalysis(record)
    setRecords(await listAnalyses())
  }

  const run = async (kind: 'prices' | 'what_if') => {
    if (!api || !saveFile || !defsFile) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes, defsBytes] = await Promise.all([
        bytes(saveFile),
        bytes(tokensFile),
        bytes(defsFile),
      ])
      const json =
        kind === 'prices'
          ? await api.prices(saveBytes!, tokenBytes, defsBytes!, '{}')
          : await api.what_if(
              saveBytes!,
              tokenBytes,
              defsBytes!,
              '{}',
              JSON.stringify(whatIfOpts),
            )
      const nextResult = JSON.parse(json) as PricesResult
      setResult(nextResult)
      await archiveResult(kind, kind === 'prices' ? {} : whatIfOpts, nextResult, saveBytes!, tokenBytes)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const files = [...event.dataTransfer.files]
    const save = files.find((file) => file.name.endsWith('.v3')) ?? files[0]
    const tokens = files.find((file) => file !== save)
    setSaveFile(save)
    if (tokens) setTokensFile(tokens)
  }

  const submitWhatIf = (event: FormEvent) => {
    event.preventDefault()
    void run('what_if')
  }

  const submitPlan = (event: FormEvent) => {
    event.preventDefault()
    if (!api || !saveFile || !defsFile) return
    setBusy(true)
    setError(undefined)
    void Promise.all([bytes(saveFile), bytes(tokensFile), bytes(defsFile)])
      .then(async ([saveBytes, tokenBytes, defsBytes]) => {
        const opts = { goal, max_days: 3650, label: label || null }
        const json = await api.plan(
          saveBytes!,
          tokenBytes,
          defsBytes!,
          '{}',
          JSON.stringify(opts),
        )
        const nextResult = JSON.parse(json) as PlanResult
        setPlanResult(nextResult)
        await archiveResult('plan', opts, nextResult, saveBytes!, tokenBytes, label)
      })
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason))
      })
      .finally(() => setBusy(false))
  }

  const ready = Boolean(api && saveFile && defsFile)

  return (
    <main>
      <header>
        <p className="eyebrow">LOCAL ECONOMY WORKBENCH</p>
        <h1>vic3-analyzer</h1>
        <p>Inspect market prices without uploading your campaign.</p>
      </header>

      <section className="inputs" aria-label="Analysis files">
        <div className="drop-zone" onDragOver={(event) => event.preventDefault()} onDrop={handleDrop}>
          <strong>Drop your .v3 save</strong>
          <span>Optionally drop its token map at the same time</span>
          <label className="file-button">
            Choose save
            <input
              aria-label="Save file"
              type="file"
              accept=".v3"
              onChange={(event) => setSaveFile(event.target.files?.[0])}
            />
          </label>
          {saveFile && <output>{saveFile.name}</output>}
        </div>
        <div className="support-files">
          <label>
            Token map (binary saves only)
            <input type="file" aria-label="Tokens file" onChange={(e) => setTokensFile(e.target.files?.[0])} />
          </label>
          <label>
            Prebuilt definitions blob
            <input type="file" aria-label="Definitions blob" onChange={(e) => setDefsFile(e.target.files?.[0])} />
          </label>
          <small>Definitions must be an offline postcard blob for the save's game patch.</small>
        </div>
      </section>

      {summary && (
        <section className="save-summary" aria-label="Save summary">
          <span>{summary.tag ?? 'Unknown country'}</span>
          <span>{summary.date ?? 'Unknown date'}</span>
          <span>Victoria 3 {summary.version}</span>
        </section>
      )}

      {error && <p role="alert">{error}</p>}

      <section className="actions">
        <button disabled={!ready || busy} onClick={() => void run('prices')}>
          Analyze prices
        </button>
        <form onSubmit={submitWhatIf}>
          <h2>What-if scenario</h2>
          <SchemaForm schema={schema} value={whatIfOpts} onChange={setWhatIfOpts} />
          <button disabled={!ready || busy} type="submit">
            Run what-if
          </button>
        </form>
        <form onSubmit={submitPlan}>
          <h2>Plan timeline</h2>
          <label>
            Goal
            <input aria-label="Plan goal" value={goal} onChange={(event) => setGoal(event.target.value)} />
          </label>
          <label>
            Label
            <input aria-label="Plan label" value={label} onChange={(event) => setLabel(event.target.value)} />
          </label>
          <button disabled={!ready || busy || !goal.trim()} type="submit">
            Run plan
          </button>
        </form>
      </section>

      {result && (
        <section aria-labelledby="prices-heading">
          <div className="result-heading">
            <h2 id="prices-heading">Goods prices</h2>
            <span>{result.status} · residual {result.residual.toPrecision(4)}</span>
          </div>
          <div className="table-scroll">
            <table>
              <thead>
                <tr><th>Good</th><th>Base</th><th>Price</th><th>Buy</th><th>Sell</th></tr>
              </thead>
              <tbody>
                {result.goods.map((good) => (
                  <tr key={good.id}>
                    <th>{good.id}</th>
                    <td>{good.base.toFixed(2)}</td>
                    <td>{good.price.toFixed(2)}</td>
                    <td>{good.buy.toFixed(2)}</td>
                    <td>{good.sell.toFixed(2)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="limitations">
            <h3>Model limitations</h3>
            <ul>{result.limitations.map((item) => <li key={item}>{item}</li>)}</ul>
          </div>
        </section>
      )}

      {planResult && (
        <section aria-labelledby="plan-heading">
          <div className="result-heading">
            <h2 id="plan-heading">Plan timeline</h2>
            <span>{planResult.day_cost} total days</span>
          </div>
          <ol className="timeline">
            {planResult.actions.map((step, index) => (
              <li key={`${step.day}-${index}`}>
                <strong>Day {step.day}</strong>
                <span>{actionLabel(step.action)}</span>
              </li>
            ))}
          </ol>
          <div className="limitations">
            <h3>Model limitations</h3>
            <ul>{planResult.limitations.map((item) => <li key={item}>{item}</li>)}</ul>
          </div>
        </section>
      )}

      <section aria-labelledby="archive-heading">
        <h2 id="archive-heading">Past runs</h2>
        {records.length === 0 ? (
          <p>No archived analyses yet.</p>
        ) : (
          <ul className="archive-list">
            {records.map((record) => (
              <li key={record.id}>
                <strong>
                  {record.kind === 'what_if' ? 'What-if' : record.kind === 'plan' ? 'Plan' : 'Prices'}
                  {record.label ? ` · ${record.label}` : ''}
                </strong>
                <span>{record.country ?? '—'} · {record.date ?? '—'}</span>
                <time dateTime={record.created_at}>{new Date(record.created_at).toLocaleString()}</time>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  )
}

export default App
