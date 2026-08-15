import { useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type FormEvent } from 'react'
import whatIfSchemaJson from '../../schema/what-if.json'
import './App.css'
import {
  compareAnalyses,
  listAnalyses,
  parseAnalysis,
  saveAnalysis,
  serializeAnalysis,
} from './archive'
import { FieldHelp } from './FieldHelp'
import { SchemaForm } from './SchemaForm'
import {
  canUseRememberedSavePicker,
  pickSaveWithRememberedFolder,
  victoria3SavePaths,
} from './savePicker'
import type {
  AnalysisKind,
  AnalysisRecord,
  AnalysisResult,
  GapAtom,
  GapsResult,
  JsonSchema,
  PlanAction,
  PlanResult,
  PricesResult,
  SaveSummary,
} from './types'
import { loadWasm, parseSchema, runGaps, type WasmApi } from './wasm'

function bundledDefsUrl(): string {
  const base = import.meta.env.BASE_URL || '/'
  const prefix = base.endsWith('/') ? base : `${base}/`
  return `${prefix}defs.postcard`
}

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

function kindLabel(kind: AnalysisKind): string {
  switch (kind) {
    case 'what_if':
      return 'What-if'
    case 'gaps':
      return 'Gaps'
    case 'plan':
      return 'Plan'
    default:
      return 'Prices'
  }
}

function App({ wasmApi }: Props) {
  const [api, setApi] = useState<WasmApi>()
  const [saveFile, setSaveFile] = useState<File>()
  const [tokensFile, setTokensFile] = useState<File>()
  const [defsFile, setDefsFile] = useState<File>()
  const [bundledDefsFile, setBundledDefsFile] = useState<File>()
  const [bundledDefsStatus, setBundledDefsStatus] = useState<'loading' | 'ready' | 'missing'>(
    'loading',
  )
  const [summary, setSummary] = useState<SaveSummary>()
  const [result, setResult] = useState<PricesResult>()
  const [gapsResult, setGapsResult] = useState<GapsResult>()
  const [planResult, setPlanResult] = useState<PlanResult>()
  const [goal, setGoal] = useState('research(tech=nitroglycerin)')
  const [label, setLabel] = useState('')
  const [schema, setSchema] = useState<JsonSchema>(fallbackSchema)
  const [whatIfOpts, setWhatIfOpts] = useState<Record<string, unknown>>(() =>
    initialValue(fallbackSchema),
  )
  const [records, setRecords] = useState<AnalysisRecord[]>([])
  const [selectedRecordIds, setSelectedRecordIds] = useState<string[]>([])
  const [archiveNote, setArchiveNote] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const saveInputRef = useRef<HTMLInputElement>(null)
  const savePaths = useMemo(() => victoria3SavePaths(), [])
  const rememberedPicker = canUseRememberedSavePicker()
  const effectiveDefs = defsFile ?? bundledDefsFile

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
    let cancelled = false
    setBundledDefsStatus('loading')
    void fetch(bundledDefsUrl())
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const buffer = await response.arrayBuffer()
        if (cancelled) return
        setBundledDefsFile(new File([buffer], 'defs.postcard'))
        setBundledDefsStatus('ready')
      })
      .catch(() => {
        if (!cancelled) {
          setBundledDefsFile(undefined)
          setBundledDefsStatus('missing')
        }
      })
    return () => {
      cancelled = true
    }
  }, [])

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
    analysisResult: AnalysisResult,
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
    if (!api || !saveFile || !effectiveDefs) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes, defsBytes] = await Promise.all([
        bytes(saveFile),
        bytes(tokensFile),
        bytes(effectiveDefs),
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

  const chooseSave = async () => {
    if (rememberedPicker) {
      try {
        const file = await pickSaveWithRememberedFolder()
        if (file) setSaveFile(file)
        return
      } catch (reason) {
        setError(reason instanceof Error ? reason.message : String(reason))
        return
      }
    }
    saveInputRef.current?.click()
  }

  const submitWhatIf = (event: FormEvent) => {
    event.preventDefault()
    void run('what_if')
  }

  const submitGaps = async (event: FormEvent) => {
    event.preventDefault()
    if (!api || !saveFile || !effectiveDefs || !goal.trim()) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes, defsBytes] = await Promise.all([
        bytes(saveFile),
        bytes(tokensFile),
        bytes(effectiveDefs),
      ])
      const json = await runGaps(api, saveBytes!, tokenBytes, defsBytes!, goal.trim())
      const nextResult = JSON.parse(json) as GapsResult
      setGapsResult(nextResult)
      await archiveResult('gaps', { goal: goal.trim() }, nextResult, saveBytes!, tokenBytes)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const submitPlan = (event: FormEvent) => {
    event.preventDefault()
    if (!api || !saveFile || !effectiveDefs) return
    setBusy(true)
    setError(undefined)
    void Promise.all([bytes(saveFile), bytes(tokensFile), bytes(effectiveDefs)])
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

  const formatGap = (atom: GapAtom) => (typeof atom === 'string' ? atom : JSON.stringify(atom))

  const groupedRecords = useMemo(() => {
    const groups = new Map<string, AnalysisRecord[]>()
    for (const record of records) {
      const group = groups.get(record.fingerprint) ?? []
      group.push(record)
      groups.set(record.fingerprint, group)
    }
    return [...groups.entries()]
  }, [records])

  const selectedRecords = selectedRecordIds
    .map((id) => records.find((record) => record.id === id))
    .filter((record): record is AnalysisRecord => Boolean(record))
  const comparison =
    selectedRecords.length === 2 ? compareAnalyses(selectedRecords[0], selectedRecords[1]) : undefined

  const toggleComparison = (id: string) => {
    setSelectedRecordIds((current) =>
      current.includes(id)
        ? current.filter((selected) => selected !== id)
        : current.length < 2
          ? [...current, id]
          : [current[1], id],
    )
  }

  const importRecord = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    if (!file) return
    try {
      const record = parseAnalysis(await file.text())
      await saveAnalysis(record)
      setRecords(await listAnalyses())
      setArchiveNote(`Imported ${record.label ?? record.id}.`)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      event.target.value = ''
    }
  }

  const exportRecord = (record: AnalysisRecord) => {
    const url = URL.createObjectURL(
      new Blob([serializeAnalysis(record)], { type: 'application/json' }),
    )
    const link = document.createElement('a')
    link.href = url
    link.download = `${record.id}.json`
    link.click()
    URL.revokeObjectURL(url)
  }

  const reopenRecord = (record: AnalysisRecord) => {
    if (!record.blob) {
      setArchiveNote(`Re-drop ${record.filename ?? 'the save'}; its fingerprint must match.`)
      return
    }
    setSaveFile(
      new File([record.blob.save.slice().buffer as ArrayBuffer], record.filename ?? 'archive.v3'),
    )
    setTokensFile(
      record.blob.tokens
        ? new File([record.blob.tokens.slice().buffer as ArrayBuffer], 'tokens.txt')
        : undefined,
    )
    setArchiveNote(`Reopened ${record.filename ?? record.id} from the local archive.`)
  }

  const ready = Boolean(api && saveFile && effectiveDefs)

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
          <button type="button" className="file-button" onClick={() => void chooseSave()}>
            Choose save
          </button>
          <input
            ref={saveInputRef}
            aria-label="Save file"
            type="file"
            accept=".v3"
            className="visually-hidden"
            onChange={(event) => setSaveFile(event.target.files?.[0])}
          />
          {saveFile && <output>{saveFile.name}</output>}
          <p className="path-hint">{savePaths.summary}</p>
          <code className="path-hint-path">{savePaths.local}</code>
        </div>
        <div className="support-files">
          <div className="field-with-help">
            <label>
              <span className="field-label-row">
                Token map (binary saves only)
                <FieldHelp label="About token maps">
                  <p>
                    Binary (Ironman) saves store field names as numbers. A token map is a text file
                    that translates those numbers back into names so this tool can read the save.
                  </p>
                  <p>
                    Plaintext saves do not need a token map. Token maps stay on your machine and are
                    never uploaded; this project does not redistribute Paradox tokens.
                  </p>
                </FieldHelp>
              </span>
              <input
                type="file"
                aria-label="Tokens file"
                onChange={(e) => setTokensFile(e.target.files?.[0])}
              />
            </label>
          </div>
          <div className="field-with-help">
            <label>
              <span className="field-label-row">
                Definitions blob
                <FieldHelp label="About definitions">
                  <p>
                    Definitions are a postcard-encoded snapshot of goods, needs, and production
                    methods for a game patch. Analysis uses that blob instead of reading a Victoria
                    3 install in the browser.
                  </p>
                  <p>
                    The demo ships a fixture blob for local experiments. Prefer a blob built for
                    your save&apos;s patch when analyzing a real campaign.
                  </p>
                </FieldHelp>
              </span>
              <input
                type="file"
                aria-label="Choose definitions blob"
                onChange={(e) => setDefsFile(e.target.files?.[0])}
              />
            </label>
            <small>
              {defsFile
                ? `Using your file: ${defsFile.name}`
                : bundledDefsStatus === 'ready'
                  ? 'Using the bundled demo definitions blob.'
                  : bundledDefsStatus === 'loading'
                    ? 'Loading bundled demo definitions…'
                    : 'Bundled demo definitions are unavailable; choose a postcard blob.'}
            </small>
          </div>
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

      <section aria-labelledby="gaps-form-heading">
        <form className="gaps-form" onSubmit={(event) => void submitGaps(event)}>
          <h2 id="gaps-form-heading">Goal gaps</h2>
          <label>
            Goal
            <input
              aria-label="Gaps goal"
              value={goal}
              onChange={(event) => setGoal(event.target.value)}
              placeholder="research(tech=nitroglycerin)"
            />
          </label>
          <button disabled={!ready || busy || !goal.trim()} type="submit">
            Run gaps
          </button>
        </form>
      </section>

      {gapsResult && (
        <section aria-labelledby="gaps-heading">
          <div className="result-heading">
            <h2 id="gaps-heading">Goal gaps</h2>
            <strong>Satisfied: {gapsResult.satisfied ? 'Yes' : 'No'}</strong>
          </div>
          {gapsResult.gaps.length === 0 ? (
            <p>No unsatisfied atoms.</p>
          ) : (
            <ul className="gap-list">
              {gapsResult.gaps.map((atom, index) => (
                <li key={`${formatGap(atom)}-${index}`}>
                  <code>{formatGap(atom)}</code>
                </li>
              ))}
            </ul>
          )}
          <div className="limitations">
            <h3>Model limitations</h3>
            <ul>
              {gapsResult.limitations.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        </section>
      )}

      {result && (
        <section aria-labelledby="prices-heading">
          <div className="result-heading">
            <h2 id="prices-heading">Goods prices</h2>
            <span>
              {result.status} · residual {result.residual.toPrecision(4)}
            </span>
          </div>
          <div className="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Good</th>
                  <th>Base</th>
                  <th>Price</th>
                  <th>Buy</th>
                  <th>Sell</th>
                </tr>
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
            <ul>
              {result.limitations.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
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
            <ul>
              {planResult.limitations.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        </section>
      )}

      <section aria-labelledby="archive-heading">
        <div className="result-heading">
          <h2 id="archive-heading">Past saves</h2>
          <label className="file-button">
            Import record
            <input
              aria-label="Import AnalysisRecord"
              type="file"
              accept=".json,application/json"
              onChange={(event) => void importRecord(event)}
            />
          </label>
        </div>
        <p>Select two analyses to compare stored results without running the solver again.</p>
        {archiveNote && <p role="status">{archiveNote}</p>}
        {records.length === 0 ? (
          <p>No archived analyses yet.</p>
        ) : (
          <div className="archive-groups">
            {groupedRecords.map(([fingerprintValue, group]) => (
              <section className="archive-group" key={fingerprintValue}>
                <h3>
                  {group[0].country ?? 'Unknown country'} · {group[0].date ?? 'Unknown date'}
                </h3>
                <small title={fingerprintValue}>Save {fingerprintValue.slice(0, 12)}</small>
                <ul className="archive-list">
                  {group.map((record) => (
                    <li key={record.id}>
                      <label>
                        <input
                          type="checkbox"
                          aria-label={`Compare ${record.label ?? record.id}`}
                          checked={selectedRecordIds.includes(record.id)}
                          onChange={() => toggleComparison(record.id)}
                        />
                        <strong>
                          {kindLabel(record.kind)}
                          {record.label ? ` · ${record.label}` : ''}
                        </strong>
                      </label>
                      <time dateTime={record.created_at}>
                        {new Date(record.created_at).toLocaleString()}
                      </time>
                      <div className="archive-buttons">
                        <button type="button" onClick={() => reopenRecord(record)}>
                          {record.blob ? 'Reopen save' : 'Re-drop save'}
                        </button>
                        <button type="button" onClick={() => exportRecord(record)}>
                          Export
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </section>

      {comparison && (
        <section aria-labelledby="compare-heading">
          <div className="result-heading">
            <h2 id="compare-heading">Archive comparison</h2>
            <strong>
              {comparison.same_fingerprint ? 'Alternative plans' : 'Campaign progression'}
            </strong>
          </div>
          {comparison.day_cost_delta !== undefined && (
            <p>
              Day cost delta:{' '}
              <strong>
                {comparison.day_cost_delta > 0 ? '+' : ''}
                {comparison.day_cost_delta}
              </strong>
            </p>
          )}
          {comparison.actions && (
            <>
              <h3>Action changes</h3>
              <ul>
                {comparison.actions.map((change, index) => (
                  <li key={index}>
                    {change.left ? `${actionLabel(change.left.action)} → ` : 'Added: '}
                    {change.right ? actionLabel(change.right.action) : 'removed'}
                  </li>
                ))}
              </ul>
            </>
          )}
          {comparison.prices && (
            <>
              <h3>Price changes</h3>
              <ul>
                {comparison.prices.map((change) => (
                  <li key={change.good}>
                    {change.good}: {change.delta > 0 ? '+' : ''}
                    {change.delta.toFixed(2)}
                  </li>
                ))}
              </ul>
            </>
          )}
          {comparison.gaps && (
            <>
              <h3>Gap changes</h3>
              <ul>
                {comparison.gaps.map((change, index) => (
                  <li key={`${formatGap(change.atom)}-${index}`}>
                    {formatGap(change.atom)} · {change.status.replaceAll('_', ' ')}
                  </li>
                ))}
              </ul>
            </>
          )}
          {!comparison.actions?.length &&
            !comparison.prices?.length &&
            !comparison.gaps?.length &&
            comparison.day_cost_delta === undefined && <p>No comparable stored-result changes.</p>}
        </section>
      )}
    </main>
  )
}

export default App
