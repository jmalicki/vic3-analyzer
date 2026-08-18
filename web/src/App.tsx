import { useEffect, useMemo, useRef, useState, type ChangeEvent, type DragEvent, type FormEvent } from 'react'
import './App.css'
import {
  compareAnalyses,
  listAnalyses,
  parseAnalysis,
  saveAnalysis,
  serializeAnalysis,
} from './archive'
import { DefsBuilder } from './DefsBuilder'
import { clearStoredDefs, loadStoredDefs, storeDefs } from './defsStore'
import { clearStoredSave, loadStoredSave, persistErrorMessage, storeSave, storeSaveAnalysis } from './saveStore'
import { AlertsPane } from './AlertsPane'
import { MilitaryPane } from './MilitaryPane'
import { FieldHelp } from './FieldHelp'
import { GoalBuilder } from './GoalBuilder'
import { parseDefsIcons } from './GameIcon'
import { Modal } from './Modal'
import { PLAN_TEMPLATES, planTemplate } from './planTemplates'
import { PriceExplorer } from './PriceExplorer'
import { StatesPane } from './StatesPane'
import { ProgressBar } from './ProgressBar'
import {
  canUseRememberedSavePicker,
  pickSaveWithRememberedFolder,
  victoria3SavePaths,
} from './savePicker'
import type {
  AnalysisKind,
  AnalysisRecord,
  AnalysisResult,
  AlertsResult,
  DefsIcons,
  MilitarySnapshot,
  DefsSummary,
  GapAtom,
  GapsResult,
  PlanAction,
  PlanResult,
  PricesResult,
  SaveSummary,
} from './types'
import type { WasmApi } from './wasm'
import { loadWasmApi } from './wasmClient'
import {
  hashForView,
  parseHash,
  WORKSPACE_NAV,
  type MilitaryTab,
  type WorkspaceView,
} from './workspaceNav'

/**
 * Demo definitions exist for local development and tests only. The fixture is
 * generated outside `public/`, so a production build ships nothing and prices
 * can only come from definitions the user supplies.
 */
function demoDefsUrl(): string | undefined {
  if (!import.meta.env.DEV) return undefined
  const base = import.meta.env.BASE_URL || '/'
  const prefix = base.endsWith('/') ? base : `${base}/`
  return `${prefix}fixtures/defs.postcard`
}

interface Props {
  wasmApi?: WasmApi | Promise<WasmApi>
}

const MODEL_DOCS =
  'https://github.com/jmalicki/vic3-analyzer/blob/main/docs/prices.md#limitations-must-appear-in-rustdoc-cli-json-ui'

function ModelInfo({ status }: { status?: PricesResult['status'] }) {
  return (
    <p className="model-info">
      {status && status !== 'converged' && <strong>Solver status: {status}. </strong>}
      Results use a simplified economy model.{' '}
      <a href={MODEL_DOCS} target="_blank" rel="noreferrer noopener">
        Method and limitations
      </a>
    </p>
  )
}

interface PlanTemplatePickerProps {
  idPrefix: string
  value: string
  onChange: (id: string) => void
}

function PlanTemplatePicker({ idPrefix, value, onChange }: PlanTemplatePickerProps) {
  const selected = planTemplate(value)
  return (
    <div className="plan-template-picker">
      <label>
        Default plan
        <select
          aria-label={`${idPrefix} default plan`}
          value={value}
          onChange={(event) => onChange(event.target.value)}
        >
          <option value="">Custom goal</option>
          {PLAN_TEMPLATES.map((template) => (
            <option key={template.id} value={template.id} disabled={!template.goal}>
              {template.title}{template.goal ? '' : ' (coming soon)'}
            </option>
          ))}
        </select>
      </label>
      {selected && (
        <p className="template-description">
          {selected.description}
          {!selected.goal && ' This preset cannot be run yet.'}
        </p>
      )}
    </div>
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
  if ('QueueBuildingLevel' in action) {
    return `Queue building level: ${action.QueueBuildingLevel.building}`
  }
  const { days, event } = action.WaitForEvent
  if ('TechCompleted' in event) {
    return `Wait ${days} days for ${event.TechCompleted.tech}`
  }
  return `Wait ${days} days for ${event.BuildingCompleted.building}`
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
  const [saveRestored, setSaveRestored] = useState(false)
  const [defsFile, setDefsFile] = useState<File>()
  const [defsRestored, setDefsRestored] = useState(false)
  const [demoDefsFile, setDemoDefsFile] = useState<File>()
  const [demoDefsStatus, setDemoDefsStatus] = useState<'loading' | 'ready' | 'absent'>(
    demoDefsUrl() ? 'loading' : 'absent',
  )
  const [summary, setSummary] = useState<SaveSummary>()
  const [defsSummary, setDefsSummary] = useState<DefsSummary>()
  const [goodIcons, setGoodIcons] = useState<DefsIcons>({})
  const [result, setResult] = useState<PricesResult>()
  const [gapsResult, setGapsResult] = useState<GapsResult>()
  const [planResult, setPlanResult] = useState<PlanResult>()
  const [alertsResult, setAlertsResult] = useState<AlertsResult>()
  const [militaryResult, setMilitaryResult] = useState<MilitarySnapshot>()
  const [goal, setGoal] = useState('research(tech=nitroglycerin)')
  const [label, setLabel] = useState('')
  const [selectedTemplateId, setSelectedTemplateId] = useState('')
  const [whatIfOpts, setWhatIfOpts] = useState<Record<string, unknown>>({
    building: '',
    extra_levels: 1,
  })
  const [activeView, setActiveView] = useState<WorkspaceView>(() => parseHash().view ?? 'prices')
  const [militaryTab, setMilitaryTab] = useState<MilitaryTab>(() => parseHash().militaryTab)
  const [records, setRecords] = useState<AnalysisRecord[]>([])
  const [selectedRecordIds, setSelectedRecordIds] = useState<string[]>([])
  const [archiveNote, setArchiveNote] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [builderOpen, setBuilderOpen] = useState(false)
  const [builderBusy, setBuilderBusy] = useState(false)
  const [analysisReady, setAnalysisReady] = useState(false)
  const [error, setError] = useState<string>()
  const saveInputRef = useRef<HTMLInputElement>(null)
  const savePaths = useMemo(() => victoria3SavePaths(), [])
  const rememberedPicker = canUseRememberedSavePicker()
  const effectiveDefs = defsFile ?? demoDefsFile
  const selectedTemplate = planTemplate(selectedTemplateId)

  const applyPlanTemplate = (id: string) => {
    setSelectedTemplateId(id)
    const template = planTemplate(id)
    if (!template?.goal) return
    setGoal(template.goal)
    setLabel(template.label)
  }

  const persistSave = (save: File, tokens?: File) => {
    void storeSave(save, tokens).catch((error: unknown) => {
      setError(persistErrorMessage(error))
    })
  }

  /** Keep the chosen save (and optional token map) across reloads. */
  const applySaveFile = (file?: File, tokens: File | null = null) => {
    setSaveFile(file)
    setTokensFile(file ? tokens ?? undefined : undefined)
    setSaveRestored(false)
    setResult(undefined)
    setGapsResult(undefined)
    setPlanResult(undefined)
    setSummary(undefined)
    setAnalysisReady(false)
    if (!file) {
      void clearStoredSave().catch(() => {})
      return
    }
    persistSave(file, tokens ?? undefined)
  }

  /** Keep the chosen blob across reloads. */
  const applyDefsFile = (file?: File) => {
    setDefsFile(file)
    setDefsRestored(false)
    setResult(undefined)
    setGapsResult(undefined)
    setPlanResult(undefined)
    setAnalysisReady(false)
    void (file ? storeDefs(file) : clearStoredDefs()).catch(() => {
      setError('Definitions could not be saved in this browser; they last until reload.')
    })
  }

  useEffect(() => {
    let cancelled = false
    void loadStoredSave()
      .then((stored) => {
        if (!stored || cancelled) return
        setSaveFile(stored.save)
        setTokensFile(stored.tokens)
        setSaveRestored(true)
        if (stored.summary) setSummary(stored.summary)
        if (stored.prices) setResult(stored.prices)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    void listAnalyses().then(setRecords)
    void Promise.resolve(wasmApi ?? loadWasmApi())
      .then((loaded) => {
        setApi(loaded)
      })
      .catch(() => {
        if (wasmApi) setError('Could not load the analysis engine.')
      })
  }, [wasmApi])

  useEffect(() => {
    let cancelled = false
    void loadStoredDefs()
      .then((file) => {
        if (!file || cancelled) return
        setDefsFile(file)
        setDefsRestored(true)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const url = demoDefsUrl()
    if (!url) return
    let cancelled = false
    setDemoDefsStatus('loading')
    void fetch(url)
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        const buffer = await response.arrayBuffer()
        if (cancelled) return
        setDemoDefsFile(new File([buffer], 'defs.postcard'))
        setDemoDefsStatus('ready')
      })
      .catch(() => {
        if (!cancelled) {
          setDemoDefsFile(undefined)
          setDemoDefsStatus('absent')
        }
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!api || !saveFile) {
      if (!saveFile && api) void api.clear_analysis()
      return
    }
    let cancelled = false
    const inputs = effectiveDefs
      ? Promise.all([bytes(saveFile), bytes(tokensFile), bytes(effectiveDefs)])
      : Promise.all([bytes(saveFile), bytes(tokensFile)])
    void inputs
      .then(async (loaded) => {
        if (effectiveDefs) {
          const [saveBytes, tokenBytes, defsBytes] = loaded
          const json = await api.load_analysis(saveBytes!, tokenBytes, defsBytes!, '{}')
          if (cancelled) return
          const payload = JSON.parse(json) as { summary: SaveSummary; prices: PricesResult }
          setSummary(payload.summary)
          setResult(payload.prices)
          setAnalysisReady(true)
          void storeSaveAnalysis(payload.summary, payload.prices).catch((error: unknown) => {
            setError(persistErrorMessage(error))
          })
        } else {
          const [saveBytes, tokenBytes] = loaded
          const json = await api.parse_save(saveBytes!, tokenBytes)
          if (!cancelled) setSummary(JSON.parse(json) as SaveSummary)
        }
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setAnalysisReady(false)
          setError(reason instanceof Error ? reason.message : String(reason))
        }
      })
    return () => {
      cancelled = true
    }
  }, [api, saveFile, tokensFile, effectiveDefs])

  useEffect(() => {
    if (!api || !effectiveDefs) {
      setDefsSummary(undefined)
      return
    }
    let cancelled = false
    void bytes(effectiveDefs)
      .then(async (defsBytes) => {
        const json = await api.defs_summary(defsBytes!)
        if (cancelled) return
        setDefsSummary(JSON.parse(json) as DefsSummary)
        // Icons are optional: a blob built without the gfx folder still prices.
        const icons = await Promise.resolve(api.defs_icons(defsBytes!)).catch(() => '{}')
        if (!cancelled) setGoodIcons(parseDefsIcons(JSON.parse(icons)))
      })
      .catch((reason: unknown) => {
        if (cancelled) return
        setDefsSummary(undefined)
        setGoodIcons({})
        if (defsFile) {
          setDefsFile(undefined)
          setDefsRestored(false)
          void clearStoredDefs()
          const detail = reason instanceof Error ? reason.message : String(reason)
          setError(
            `${detail.replace(/\.?$/, '.')} Rebuild definitions from your Victoria 3 game folder for this app version.`,
          )
        }
      })
    return () => {
      cancelled = true
    }
  }, [api, defsFile, effectiveDefs])

  useEffect(() => {
    const firstBuilding = summary?.buildings?.[0]
    if (firstBuilding && !whatIfOpts.building) {
      setWhatIfOpts((current) => ({ ...current, building: firstBuilding }))
    }
  }, [summary, whatIfOpts.building])

  useEffect(() => {
    const sync = () => {
      const parsed = parseHash()
      if (parsed.view) setActiveView(parsed.view)
      setMilitaryTab(parsed.militaryTab)
    }
    window.addEventListener('hashchange', sync)
    return () => window.removeEventListener('hashchange', sync)
  }, [])

  useEffect(() => {
    if (activeView !== 'alerts' || !api || !result) return
    let cancelled = false
    void Promise.resolve(api.loaded_alerts())
      .then((json) => {
        if (!cancelled) setAlertsResult(JSON.parse(json) as AlertsResult)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [activeView, api, result])

  useEffect(() => {
    if (activeView !== 'military' || !api || !result) return
    let cancelled = false
    void Promise.resolve(api.loaded_military())
      .then((json) => {
        if (!cancelled) setMilitaryResult(JSON.parse(json) as MilitarySnapshot)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [activeView, api, result])

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

  const runWhatIf = async () => {
    if (!api || !saveFile || !effectiveDefs) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes] = await Promise.all([bytes(saveFile), bytes(tokensFile)])
      const json = await api.loaded_what_if(JSON.stringify(whatIfOpts))
      const nextResult = JSON.parse(json) as PricesResult
      setResult(nextResult)
      await archiveResult('what_if', whatIfOpts, nextResult, saveBytes!, tokenBytes)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const selectView = (view: WorkspaceView) => {
    setActiveView(view)
    const next = hashForView(view)
    if (window.location.hash !== next) window.location.hash = next
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    const files = [...event.dataTransfer.files]
    const save = files.find((file) => file.name.endsWith('.v3')) ?? files[0]
    const tokens = files.find((file) => file !== save)
    applySaveFile(save, tokens ?? null)
  }

  const chooseSave = async () => {
    if (rememberedPicker) {
      try {
        const file = await pickSaveWithRememberedFolder()
        if (file) applySaveFile(file)
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
    void runWhatIf()
  }

  const submitGaps = async (event: FormEvent) => {
    event.preventDefault()
    if (!api || !saveFile || !effectiveDefs || !goal.trim()) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes] = await Promise.all([bytes(saveFile), bytes(tokensFile)])
      const json = await api.loaded_gaps(goal.trim())
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
    void Promise.all([bytes(saveFile), bytes(tokensFile)])
      .then(async ([saveBytes, tokenBytes]) => {
        const opts = { goal, max_days: 3650, label: label || null }
        const json = await api.loaded_plan(JSON.stringify(opts))
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
    applySaveFile(
      new File([record.blob.save.slice().buffer as ArrayBuffer], record.filename ?? 'archive.v3'),
      record.blob.tokens
        ? new File([record.blob.tokens.slice().buffer as ArrayBuffer], 'tokens.txt')
        : null,
    )
    setArchiveNote(`Reopened ${record.filename ?? record.id} from the local archive.`)
  }

  const hasDefs = Boolean(effectiveDefs)
  const ready = Boolean(api && saveFile && effectiveDefs && analysisReady)
  const missing = [
    ...(saveFile ? [] : ['a .v3 save']),
    ...(hasDefs ? [] : ['game definitions']),
  ]
  // The archive only reads stored records, so it stays usable without inputs.
  const gated = missing.length > 0 && activeView !== 'archive'
  const defsCounts = defsSummary
    ? ` — format v${defsSummary.blob_version}, ${defsSummary.goods} goods, ${defsSummary.labels} names, ${defsSummary.icons} icons, ${defsSummary.production_methods} production methods`
    : ''
  // A real install has dozens of goods; a handful means the fixture blob or a
  // folder pick that missed common/goods.
  const thinDefs = Boolean(defsSummary && defsSummary.goods < 10)

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
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) applySaveFile(file)
            }}
          />
          {saveFile && (
            <>
              <output>
                {saveFile.name}
                {saveRestored ? ' (kept from a previous visit)' : ''}
              </output>
              <button type="button" className="secondary" onClick={() => applySaveFile(undefined)}>
                Forget this save
              </button>
            </>
          )}
          <p className="path-hint">{savePaths.label}</p>
          <code className="path-hint-path">{savePaths.local}</code>
          <p className="path-hint">{savePaths.summary}</p>
        </div>
        <div className="support-files">
          <div className="field-with-help">
            <label>
              <span className="field-label-row">
                Token map (binary saves only)
                <FieldHelp label="About token maps">
                  <p>
                    Victoria 3 saves are binary by default: field names are stored as numeric
                    tokens. A token map is a plain-text file with one{' '}
                    <code>0x1234 field_name</code> pair per line that translates them back.
                  </p>
                  <p>
                    <strong>Most players do not need one.</strong> Outside Ironman, you can make the
                    game write readable saves: in <code>pdx_settings.json</code> (next to your save
                    folder) set <code>&quot;save_file_format&quot;: &quot;zip_text_all&quot;</code>,
                    then re-save. That save loads here with no token map. See the{' '}
                    <a
                      href="https://vic3.paradoxwikis.com/Save-game_editing"
                      target="_blank"
                      rel="noreferrer noopener"
                    >
                      Victoria 3 wiki
                    </a>
                    .
                  </p>
                  <p>
                    Ironman saves stay binary, so they do need a map. There is no official download:
                    Paradox does not publish the mapping and this project will not redistribute it.
                    Token maps are extracted from your own game build, and other tools (such as
                    pdx-tools) likewise expect a user-supplied file. Anything you pick stays in your
                    browser.
                  </p>
                </FieldHelp>
              </span>
              <input
                type="file"
                aria-label="Tokens file"
                onChange={(e) => {
                  const file = e.target.files?.[0]
                  setTokensFile(file)
                  setSaveRestored(false)
                  if (saveFile) persistSave(saveFile, file)
                }}
              />
            </label>
          </div>
          <div className="field-with-help">
            <span className="field-label-row">
              Game definitions
              <FieldHelp label="About definitions">
                <p>
                  Definitions are a postcard-encoded snapshot of goods, needs, and production
                  methods for a game patch. Build them locally from the Victoria 3 game folder.
                </p>
                <p>
                  The selected files never leave your browser. The result is kept locally so a
                  reload does not require choosing the folder again. Deployed builds ship no
                  definitions of their own.
                </p>
              </FieldHelp>
            </span>
            <small>
              {defsFile
                ? `Using your file: ${defsFile.name}${defsCounts}${
                    defsRestored ? ' (kept from a previous visit)' : ''
                  }`
                : demoDefsStatus === 'ready'
                  ? `Using the local development demo blob${defsCounts}.`
                  : demoDefsStatus === 'loading'
                    ? 'Loading development demo definitions…'
                    : 'No definitions loaded. Build them from your local Victoria 3 game folder.'}
            </small>
            <div className="defs-builder-actions">
              <button type="button" className="secondary" onClick={() => setBuilderOpen(true)}>
                Build definitions from game files…
              </button>
              {defsFile && (
                <button type="button" className="secondary" onClick={() => applyDefsFile(undefined)}>
                  Forget these definitions
                </button>
              )}
            </div>
          </div>
        </div>
      </section>

      {builderOpen && (
        <Modal
          title="Build definitions from game files"
          locked={builderBusy}
          onClose={() => setBuilderOpen(false)}
        >
          <DefsBuilder
            api={api}
            onBuilt={applyDefsFile}
            onBusyChange={setBuilderBusy}
            onDone={() => setBuilderOpen(false)}
          />
        </Modal>
      )}

      {summary && (
        <section className="save-summary" role="region" aria-label="Campaign summary">
          <div>
            <span className="hud-label">Country</span>
            <strong>{summary.tag ?? 'Unknown country'}</strong>
          </div>
          <div>
            <span className="hud-label">Date</span>
            <strong>{summary.date ?? 'Unknown date'}</strong>
          </div>
          <div>
            <span className="hud-label">Version</span>
            <strong>Victoria 3 {summary.version}</strong>
          </div>
          <div>
            <span className="hud-label">GDP</span>
            <strong>—</strong>
          </div>
          <div>
            <span className="hud-label">SoL</span>
            <strong>—</strong>
          </div>
          <div>
            <span className="hud-label">Alerts</span>
            <strong>—</strong>
          </div>
        </section>
      )}

      {error && <p role="alert">{error}</p>}

      <nav className="workspace-nav" aria-label="Analysis tools">
        {WORKSPACE_NAV.map(({ view, label }) => (
          <button
            type="button"
            key={view}
            aria-current={activeView === view ? 'page' : undefined}
            onClick={() => selectView(view)}
          >
            {label}
          </button>
        ))}
      </nav>

      {gated && (
        <p className="defs-required" role="status">
          {!hasDefs && demoDefsStatus === 'loading'
            ? 'Loading definitions…'
            : `Analysis needs ${missing.join(' and ')}. Add ${
                missing.length > 1 ? 'them' : 'it'
              } above; the tools below stay locked until then.`}
        </p>
      )}

      {busy && <ProgressBar label="Analyzing in wasm" />}

      {activeView === 'prices' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="prices-tool-heading"
        >
          <div className="tool-heading">
            <div>
              <p className="eyebrow">MARKET</p>
              <h2 id="prices-tool-heading">Goods prices</h2>
              <p>Estimate current prices from your save and selected game definitions.</p>
            </div>
          </div>
          {thinDefs && (
            <p className="demo-warning">
              {defsFile
                ? `${defsFile.name} only defines ${defsSummary?.goods} goods, so prices below cover just those. Rebuild from the game folder itself — picking a subfolder skips the files the solver needs.`
                : 'The local development demo blob defines only a few fixture goods. Build definitions from a Victoria 3 install for the full goods list.'}
            </p>
          )}
        </section>
      )}

      {activeView === 'states' && (
        <StatesPane
          result={result}
          icons={goodIcons}
          playerCountryId={summary?.country_id}
          playerMarketId={summary?.market_id}
          gated={gated}
        />
      )}

      {activeView === 'pops' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="pops-heading"
        >
          <h2 id="pops-heading">Pops</h2>
        </section>
      )}

      {activeView === 'alerts' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="alerts-heading"
        >
          <h2 id="alerts-heading">Alerts</h2>
          {alertsResult ? (
            <AlertsPane result={alertsResult} icons={goodIcons} />
          ) : (
            <p>Alerts appear after a save is priced.</p>
          )}
        </section>
      )}

      {activeView === 'military' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="military-heading"
        >
          <h2 id="military-heading">Military</h2>
          <div className="state-tabs" role="tablist" aria-label="Military branches">
            {(['army', 'navy', 'mobilization'] as const).map((tab) => (
              <button
                type="button"
                key={tab}
                role="tab"
                aria-selected={militaryTab === tab}
                onClick={() => {
                  setMilitaryTab(tab)
                  window.location.hash = hashForView('military', tab)
                }}
              >
                {tab === 'army' ? 'Army' : tab === 'navy' ? 'Navy' : 'Mobilization'}
              </button>
            ))}
          </div>
          {militaryResult ? (
            <MilitaryPane snapshot={militaryResult} tab={militaryTab} icons={goodIcons} />
          ) : (
            <p>Military details appear after a save is priced.</p>
          )}
        </section>
      )}

      {activeView === 'buildings' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="buildings-heading"
        >
          <h2 id="buildings-heading">Buildings</h2>
        </section>
      )}

      {activeView === 'what-if' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="what-if-heading"
        >
          <form className="guided-form" onSubmit={submitWhatIf}>
            <p className="eyebrow">SCENARIO</p>
            <h2 id="what-if-heading">What-if scenario</h2>
            <p>Add levels to one building type and compare the resulting prices.</p>
            <label>
              Building type
              {summary?.buildings?.length ? (
                <select
                  aria-label="Building"
                  value={String(whatIfOpts.building)}
                  onChange={(event) =>
                    setWhatIfOpts((current) => ({ ...current, building: event.target.value }))
                  }
                >
                  {summary.buildings.map((building) => (
                    <option value={building} key={building}>
                      {building.replaceAll('_', ' ')}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  aria-label="Building"
                  value={String(whatIfOpts.building)}
                  onChange={(event) =>
                    setWhatIfOpts((current) => ({ ...current, building: event.target.value }))
                  }
                  placeholder="Select a save first"
                />
              )}
            </label>
            <label>
              Extra levels
              <input
                aria-label="Extra Levels"
                type="number"
                min="1"
                step="1"
                value={Number(whatIfOpts.extra_levels)}
                onChange={(event) =>
                  setWhatIfOpts((current) => ({
                    ...current,
                    extra_levels: Number(event.target.value),
                  }))
                }
              />
            </label>
            <button
              disabled={!ready || busy || !String(whatIfOpts.building)}
              type="submit"
            >
              Run what-if
            </button>
          </form>
        </section>
      )}

      {activeView === 'timeline' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="timeline-tool-heading"
        >
          <form className="guided-form" onSubmit={submitPlan}>
            <p className="eyebrow">PLANNING</p>
            <h2 id="timeline-tool-heading">Plan timeline</h2>
            <PlanTemplatePicker
              idPrefix="Plan"
              value={selectedTemplateId}
              onChange={applyPlanTemplate}
            />
            <GoalBuilder
              key={`plan-${selectedTemplateId}`}
              idPrefix="Plan"
              goods={result?.goods.map((good) => good.id) ?? []}
              value={goal}
              onChange={setGoal}
              initialKind={selectedTemplate?.goalKind}
            />
            <label>
              Plan label (optional)
              <input
                aria-label="Plan label"
                value={label}
                onChange={(event) => setLabel(event.target.value)}
                placeholder="e.g. Rush explosives"
              />
            </label>
            <button disabled={!ready || busy || !goal.trim()} type="submit">
              Build timeline
            </button>
          </form>
        </section>
      )}

      {activeView === 'gaps' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="gaps-form-heading"
        >
          <form className="guided-form" onSubmit={(event) => void submitGaps(event)}>
            <p className="eyebrow">READINESS</p>
            <h2 id="gaps-form-heading">Goal gaps</h2>
            <PlanTemplatePicker
              idPrefix="Gaps"
              value={selectedTemplateId}
              onChange={applyPlanTemplate}
            />
            <GoalBuilder
              key={`gaps-${selectedTemplateId}`}
              idPrefix="Gaps"
              goods={result?.goods.map((good) => good.id) ?? []}
              value={goal}
              onChange={setGoal}
              initialKind={selectedTemplate?.goalKind}
            />
            <button disabled={!ready || busy || !goal.trim()} type="submit">
              Check readiness
            </button>
          </form>
        </section>
      )}

      {activeView === 'gaps' && gapsResult && (
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
          <ModelInfo />
        </section>
      )}

      {(activeView === 'prices' || activeView === 'what-if') && result && (
        <>
          {saveRestored && !analysisReady && (
            <p className="model-info">
              Showing the last analysis instantly. Tools that need a live solve unlock when the
              engine finishes reloading.
            </p>
          )}
          <PriceExplorer
            result={result}
            icons={goodIcons}
            scenario={activeView === 'what-if'}
            playerCountryId={summary?.country_id}
            playerMarketId={summary?.market_id}
          />
          <ModelInfo status={result.status} />
        </>
      )}

      {activeView === 'timeline' && planResult && (
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
          <ModelInfo />
        </section>
      )}

      {activeView === 'archive' && <section aria-labelledby="archive-heading">
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
      </section>}

      {activeView === 'archive' && comparison && (
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
      <footer className="site-footer">
        <span>
          vic3-analyzer v{__APP_VERSION__} ({__GIT_REVISION__})
        </span>
        <span>
          Built <time dateTime={__BUILD_TIME__}>{new Date(__BUILD_TIME__).toLocaleString()}</time>
        </span>
      </footer>
    </main>
  )
}

export default App
