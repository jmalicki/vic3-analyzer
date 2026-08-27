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
import {
  checkout,
  clearStoredSave,
  commitStep,
  currentPointer,
  downloadName,
  listSteps,
  loadStoredSave,
  persistErrorMessage,
  storeSave,
  storeSaveAnalysis,
} from './saveStore'
import { AlertsPane } from './AlertsPane'
import { BuildingsPane } from './BuildingsPane'
import { BuildQueuesPane } from './BuildQueuesPane'
import {
  ConfirmApply,
  deltasFromSteps,
  mergeDeltas,
  worldDeltaToSavePatch,
} from './ConfirmApply'
import { MilitaryPane } from './MilitaryPane'
import { FieldHelp } from './FieldHelp'
import { GoalBuilder } from './GoalBuilder'
import { parseDefsIcons } from './GameIcon'
import { Modal } from './Modal'
import { PLAN_TEMPLATES, planTemplate } from './planTemplates'
import { PopsPane } from './PopsPane'
import { PriceExplorer } from './PriceExplorer'
import { QueryPane } from './QueryPane'
import { StatesPane } from './StatesPane'
import { ProgressBar } from './ProgressBar'
import { DesktopCatalog, DesktopSaveChip, type SaveStub } from './DesktopCatalog'
import { isTauri } from './env'
import { SettingsPane } from './SettingsPane'
import {
  canUseRememberedSavePicker,
  pickSaveWithRememberedFolder,
  victoria3SavePaths,
} from './savePicker'
import { createTauriApi, invokeTauri } from './tauriApi'
import type {
  AnalysisKind,
  AnalysisRecord,
  AnalysisResult,
  AlertsResult,
  DefsIcons,
  MilitarySnapshot,
  ConstructionsSnapshot,
  DefsSummary,
  GapSimpleSubgoal,
  GapsResult,
  PlanAction,
  PlanResult,
  PricesResult,
  SaveSummary,
  StatePop,
  Step,
  WorldDelta,
} from './types'
import type { WasmApi } from './wasm'
import { formatAnalysisEngineLoadError } from './wasm'
import { loadWasmApi } from './wasmClient'
import {
  hashForView,
  parseHash,
  WORKSPACE_NAV,
  type BuildingsTab,
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
  /** Timeline mode disables presets A* cannot close yet (gaps still OK). */
  timelineMode?: boolean
}

function PlanTemplatePicker({
  idPrefix,
  value,
  onChange,
  timelineMode = false,
}: PlanTemplatePickerProps) {
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
          {PLAN_TEMPLATES.map((template) => {
            const gapsOnly = timelineMode && !template.closesTimeline
            const disabled = !template.goal || gapsOnly
            const suffix = !template.goal
              ? ' (coming soon)'
              : gapsOnly
                ? ' (gaps only)'
                : ''
            return (
              <option key={template.id} value={template.id} disabled={disabled}>
                {template.title}
                {suffix}
              </option>
            )
          })}
        </select>
      </label>
      {selected && (
        <p className="template-description">
          {selected.description}
          {!selected.goal && ' This preset cannot be run yet.'}
          {timelineMode && selected.goal && !selected.closesTimeline &&
            ' Use Goal gaps for this preset; Build timeline cannot close it yet.'}
        </p>
      )}
    </div>
  )
}

async function bytes(file?: File): Promise<Uint8Array | undefined> {
  return file ? new Uint8Array(await file.arrayBuffer()) : undefined
}

function weightedWealth(pops?: StatePop[]): number | undefined {
  if (!pops?.length) return undefined
  let wealthSum = 0
  let popSum = 0
  for (const pop of pops) {
    const size = pop.demand_size ?? (pop.workforce ?? 0) + (pop.dependents ?? 0)
    if (pop.wealth == null || size <= 0) continue
    wealthSum += pop.wealth * size
    popSum += size
  }
  return popSum > 0 ? wealthSum / popSum : undefined
}

function hudGdp(result?: PricesResult): string {
  if (result && typeof result.gdp === 'number') {
    if (Math.abs(result.gdp) >= 1_000_000) return `${(result.gdp / 1_000_000).toFixed(1)}M`
    return result.gdp.toLocaleString()
  }
  return '—'
}

function hudSol(result?: PricesResult): string {
  if (result && typeof result.sol === 'number') return result.sol.toFixed(1)
  if (result && typeof result.population_weighted_wealth === 'number') {
    return result.population_weighted_wealth.toFixed(1)
  }
  const weighted = weightedWealth(result?.state_pops)
  return weighted == null ? '—' : weighted.toFixed(1)
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
    return `Queue building level: ${action.QueueBuildingLevel.building} in state ${action.QueueBuildingLevel.state_id}`
  }
  if ('QueueInterest' in action) {
    return `Queue interest (${action.QueueInterest.kind}): ${action.QueueInterest.id}`
  }
  if ('QueueHireMilitary' in action) {
    return `Hire to full: ${action.QueueHireMilitary.building}`
  }
  if ('QueueLaw' in action) return `Queue law: ${action.QueueLaw.law}`
  if ('SwitchPm' in action) {
    return `Switch PM on building ${action.SwitchPm.building_id}`
  }
  if ('AdjustTax' in action) return `Adjust tax by ${action.AdjustTax.delta}`
  const { days, event } = action.WaitForEvent
  if ('TechCompleted' in event) {
    return `Wait ${days} days for ${event.TechCompleted.tech}`
  }
  if ('BuildingCompleted' in event) {
    const sid = event.BuildingCompleted.state_id
    return sid != null
      ? `Wait ${days} days for ${event.BuildingCompleted.building} in state ${sid}`
      : `Wait ${days} days for ${event.BuildingCompleted.building}`
  }
  if ('InterestDeclared' in event) {
    return `Wait ${days} days for interest ${event.InterestDeclared.id}`
  }
  if ('HireCompleted' in event) {
    return `Wait ${days} days to staff ${event.HireCompleted.building}`
  }
  if ('LawEnacted' in event) {
    return `Wait ${days} days for law ${event.LawEnacted.law}`
  }
  return `Wait ${days} days for payday`
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
  const desktop = isTauri()
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
  const [desktopSaveName, setDesktopSaveName] = useState<string>()
  const [catalogRefresh, setCatalogRefresh] = useState(0)
  const [summary, setSummary] = useState<SaveSummary>()
  const [defsSummary, setDefsSummary] = useState<DefsSummary>()
  const [goodIcons, setGoodIcons] = useState<DefsIcons>({})
  const [result, setResult] = useState<PricesResult>()
  const [gapsResult, setGapsResult] = useState<GapsResult>()
  const [planResult, setPlanResult] = useState<PlanResult>()
  const [alertsResult, setAlertsResult] = useState<AlertsResult>()
  const [militaryResult, setMilitaryResult] = useState<MilitarySnapshot>()
  const [constructionsResult, setConstructionsResult] = useState<ConstructionsSnapshot>()
  const [timelineStep, setTimelineStep] = useState<Step>()
  const [sessionBytes, setSessionBytes] = useState<Uint8Array>()
  const [pendingApply, setPendingApply] = useState<{
    delta: WorldDelta
    preview: PricesResult
    error?: string
  }>()
  const [goal, setGoal] = useState('research(tech=nitroglycerin)')
  const [label, setLabel] = useState('')
  const [selectedTemplateId, setSelectedTemplateId] = useState('')
  const [whatIfOpts, setWhatIfOpts] = useState<Record<string, unknown>>({
    building: '',
    extra_levels: 1,
  })
  const [activeView, setActiveView] = useState<WorkspaceView>(() => {
    const parsed = parseHash().view
    if (parsed) return parsed
    return isTauri() ? 'saves' : 'prices'
  })
  const [militaryTab, setMilitaryTab] = useState<MilitaryTab>(() => parseHash().militaryTab)
  const [buildingsTab, setBuildingsTab] = useState<BuildingsTab>(
    () => parseHash().buildingsTab,
  )
  const [locationHash, setLocationHash] = useState(() => window.location.hash)
  const [records, setRecords] = useState<AnalysisRecord[]>([])
  const [selectedRecordIds, setSelectedRecordIds] = useState<string[]>([])
  const [archiveNote, setArchiveNote] = useState<string>()
  const [busy, setBusy] = useState(false)
  const [builderOpen, setBuilderOpen] = useState(false)
  const [builderBusy, setBuilderBusy] = useState(false)
  const [analysisReady, setAnalysisReady] = useState(false)
  const [error, setError] = useState<string>()
  const saveInputRef = useRef<HTMLInputElement>(null)
  const defsInputRef = useRef<HTMLInputElement>(null)
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
    void storeSave(save, tokens)
      .then(async () => {
        const pointer = await currentPointer()
        if (!pointer) return
        const steps = await listSteps(pointer.timeline_id)
        setTimelineStep(steps.find((step) => step.id === pointer.step_id))
      })
      .catch((error: unknown) => {
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
    setAlertsResult(undefined)
    setMilitaryResult(undefined)
    setConstructionsResult(undefined)
    setSummary(undefined)
    setAnalysisReady(false)
    setSessionBytes(undefined)
    setTimelineStep(undefined)
    setPendingApply(undefined)
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
    setAlertsResult(undefined)
    setMilitaryResult(undefined)
    setConstructionsResult(undefined)
    setAnalysisReady(false)
    void (file ? storeDefs(file) : clearStoredDefs()).catch(() => {
      setError('Definitions could not be saved in this browser; they last until reload.')
    })
  }

  const clearAnalysisUi = () => {
    setResult(undefined)
    setGapsResult(undefined)
    setPlanResult(undefined)
    setAlertsResult(undefined)
    setMilitaryResult(undefined)
    setConstructionsResult(undefined)
    setSummary(undefined)
    setAnalysisReady(false)
    setPendingApply(undefined)
  }

  /** Desktop: bind a catalog stub via Rust (`use_save`) and hydrate prices. */
  const useDesktopSave = async (stub: SaveStub) => {
    clearAnalysisUi()
    setBusy(true)
    setError(undefined)
    try {
      const json = await invokeTauri<string>('use_save', {
        name: stub.name,
        location: stub.location,
      })
      const payload = JSON.parse(json) as {
        summary?: SaveSummary
      }
      // Keep country_id / market_id / buildings so scope filters and What-if work.
      setSummary({
        ...(payload.summary ?? {}),
        tag: payload.summary?.tag ?? stub.country ?? undefined,
        date: payload.summary?.date ?? stub.in_game_date ?? undefined,
        version: payload.summary?.version ?? '—',
      })
      setDesktopSaveName(stub.name)
      const pricesJson = await invokeTauri<string>('loaded_prices')
      setResult(JSON.parse(pricesJson) as PricesResult)
      setAnalysisReady(true)
      selectView('prices')
    } catch (reason) {
      setDesktopSaveName(undefined)
      clearAnalysisUi()
      setError(reason instanceof Error ? reason.message : String(reason))
      throw reason
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    if (desktop) return
    let cancelled = false
    void loadStoredSave()
      .then(async (stored) => {
        if (!stored || cancelled) return
        setSaveFile(stored.save)
        setTokensFile(stored.tokens)
        setSaveRestored(true)
        if (stored.summary) setSummary(stored.summary)
        if (stored.prices) setResult(stored.prices)
        const pointer = await currentPointer()
        if (!pointer || cancelled) return
        const steps = await listSteps(pointer.timeline_id)
        const step = steps.find((item) => item.id === pointer.step_id)
        if (cancelled) return
        setTimelineStep(step)
        if (step?.patched_bytes) setSessionBytes(step.patched_bytes)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [desktop])

  useEffect(() => {
    void listAnalyses().then(setRecords)
    if (desktop) {
      setApi(createTauriApi())
      return
    }
    void Promise.resolve(wasmApi ?? loadWasmApi())
      .then((loaded) => {
        setApi(loaded)
      })
      .catch((reason: unknown) => {
        // loadWasm already formats known failures; keep a single prefix if something else rejects.
        setError(
          reason instanceof Error && reason.message.startsWith('Could not load the analysis engine')
            ? reason.message
            : formatAnalysisEngineLoadError(reason),
        )
      })
  }, [wasmApi, desktop])

  useEffect(() => {
    if (desktop) return
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
  }, [desktop])

  useEffect(() => {
    if (desktop) return
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
  }, [desktop])

  useEffect(() => {
    if (desktop || !api || !saveFile) {
      if (!desktop && !saveFile && api) void api.clear_analysis()
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
          const json = await api.load_analysis(
            sessionBytes ?? saveBytes!,
            tokenBytes,
            defsBytes!,
            '{}',
          )
          if (cancelled) return
          const payload = JSON.parse(json) as { summary: SaveSummary; prices: PricesResult }
          setSummary(payload.summary)
          setResult(payload.prices)
          setAnalysisReady(true)
          setError(undefined)
          void storeSaveAnalysis(payload.summary, payload.prices).catch((error: unknown) => {
            setError(persistErrorMessage(error))
          })
        } else {
          const [saveBytes, tokenBytes] = loaded
          const json = await api.parse_save(sessionBytes ?? saveBytes!, tokenBytes)
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
  }, [api, saveFile, tokensFile, effectiveDefs, sessionBytes, desktop])

  useEffect(() => {
    if (desktop || !api || !effectiveDefs) {
      if (desktop) return
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
  }, [api, defsFile, effectiveDefs, desktop])

  /** Desktop: icons come from the companion session, not a browser defs blob. */
  useEffect(() => {
    if (!desktop) return
    if (!api || !analysisReady) {
      setGoodIcons({})
      return
    }
    let cancelled = false
    void Promise.resolve(api.defs_icons(new Uint8Array()))
      .then((icons) => {
        if (!cancelled) setGoodIcons(parseDefsIcons(JSON.parse(icons)))
      })
      .catch(() => {
        if (!cancelled) setGoodIcons({})
      })
    return () => {
      cancelled = true
    }
  }, [desktop, api, analysisReady])

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
      setBuildingsTab(parsed.buildingsTab)
      setLocationHash(window.location.hash)
    }
    window.addEventListener('hashchange', sync)
    return () => window.removeEventListener('hashchange', sync)
  }, [])

  useEffect(() => {
    if (!api || !result || !analysisReady) return
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
  }, [api, result, analysisReady])

  useEffect(() => {
    if (activeView !== 'military' || !api || !result || !analysisReady) return
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
  }, [activeView, api, result, analysisReady])

  useEffect(() => {
    if (activeView !== 'buildings' || buildingsTab !== 'queues' || !api || !result || !analysisReady) {
      return
    }
    let cancelled = false
    void Promise.resolve(api.loaded_constructions())
      .then((json) => {
        if (!cancelled) setConstructionsResult(JSON.parse(json) as ConstructionsSnapshot)
      })
      .catch((reason: unknown) => {
        if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason))
      })
    return () => {
      cancelled = true
    }
  }, [activeView, buildingsTab, api, result, analysisReady])

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

  const applyWhatIf = async (opts: Record<string, unknown>) => {
    if (!api || !saveFile || !effectiveDefs) return
    setBusy(true)
    setError(undefined)
    try {
      const [saveBytes, tokenBytes] = await Promise.all([bytes(saveFile), bytes(tokensFile)])
      const json = await api.loaded_what_if(JSON.stringify(opts))
      const nextResult = JSON.parse(json) as PricesResult
      setResult(nextResult)
      await archiveResult('what_if', opts, nextResult, saveBytes!, tokenBytes)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const requestApply = async (delta: WorldDelta) => {
    if (!api || !result) return
    setBusy(true)
    setError(undefined)
    try {
      const json = await api.loaded_apply_delta(JSON.stringify(delta))
      setPendingApply({ delta, preview: JSON.parse(json) as PricesResult })
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const confirmPendingApply = async () => {
    if (!api || !pendingApply || !saveFile || !effectiveDefs || !result) return
    setBusy(true)
    setPendingApply((current) => current && { ...current, error: undefined })
    try {
      const stored = await loadStoredSave()
      if (!stored) throw new Error('No origin save to patch')
      const originBytes = new Uint8Array(await stored.save.arrayBuffer())
      const pointer = await currentPointer()
      const steps = pointer ? await listSteps(pointer.timeline_id) : []
      const merged = mergeDeltas([...deltasFromSteps(steps, pointer?.step_id), pendingApply.delta])
      const patch = worldDeltaToSavePatch(merged, result.buildings ?? [])
      if (!patch) throw new Error('This change cannot be written to the save')
      const patched = await api.export_save(originBytes, JSON.stringify(patch))
      const [tokenBytes, defsBytes] = await Promise.all([bytes(tokensFile), bytes(effectiveDefs)])
      const json = await api.load_analysis(patched, tokenBytes, defsBytes!, '{}')
      const payload = JSON.parse(json) as { summary: SaveSummary; prices: PricesResult }
      const step = await commitStep({
        mutations: [pendingApply.delta],
        summary: payload.summary,
        prices: payload.prices,
        patchedBytes: patched,
      })
      void storeSaveAnalysis(payload.summary, payload.prices).catch((error: unknown) => {
        setError(persistErrorMessage(error))
      })
      setSummary(payload.summary)
      setResult(payload.prices)
      setSessionBytes(patched)
      setTimelineStep(step)
      setPendingApply(undefined)
      setAlertsResult(undefined)
      setMilitaryResult(undefined)
      setConstructionsResult(undefined)
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason)
      setPendingApply((current) => current && { ...current, error: message })
    } finally {
      setBusy(false)
    }
  }

  const undoStep = async () => {
    if (!api || !effectiveDefs || !timelineStep?.parent_step_id) return
    const pointer = await currentPointer()
    if (!pointer) return
    setBusy(true)
    setError(undefined)
    try {
      const steps = await listSteps(pointer.timeline_id)
      const parent = steps.find((step) => step.id === timelineStep.parent_step_id)
      await checkout(pointer.origin_id, pointer.timeline_id, timelineStep.parent_step_id)
      const stored = await loadStoredSave()
      const originBytes = stored ? new Uint8Array(await stored.save.arrayBuffer()) : undefined
      const reload = parent?.patched_bytes ?? originBytes
      if (!reload) throw new Error('No save bytes to restore')
      const [tokenBytes, defsBytes] = await Promise.all([bytes(tokensFile), bytes(effectiveDefs)])
      const json = await api.load_analysis(reload, tokenBytes, defsBytes!, '{}')
      const payload = JSON.parse(json) as { summary: SaveSummary; prices: PricesResult }
      setSummary(payload.summary)
      setResult(payload.prices)
      setSessionBytes(parent?.patched_bytes)
      setTimelineStep(parent)
      setAlertsResult(undefined)
      setMilitaryResult(undefined)
      setConstructionsResult(undefined)
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
    } finally {
      setBusy(false)
    }
  }

  const downloadPatched = async () => {
    const stored = await loadStoredSave()
    if (!stored) return
    const pointer = await currentPointer()
    const steps = pointer ? await listSteps(pointer.timeline_id) : []
    const step = steps.find((item) => item.id === pointer?.step_id) ?? timelineStep
    const blobBytes = step?.patched_bytes ?? new Uint8Array(await stored.save.arrayBuffer())
    const name = downloadName(
      stored.save.name,
      stored.summary?.date ?? summary?.date ?? 'unknown',
      step?.id ?? 'origin',
    )
    const url = URL.createObjectURL(new Blob([blobBytes.slice().buffer as ArrayBuffer]))
    const link = document.createElement('a')
    link.href = url
    link.download = name
    link.click()
    URL.revokeObjectURL(url)
  }

  const runWhatIf = async () => {
    await applyWhatIf(whatIfOpts)
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

  const formatGap = (atom: GapSimpleSubgoal) => (typeof atom === 'string' ? atom : JSON.stringify(atom))

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

  const hasDefs = desktop || Boolean(effectiveDefs)
  const ready = desktop
    ? Boolean(api && desktopSaveName && analysisReady)
    : Boolean(api && saveFile && effectiveDefs && analysisReady)
  const missing = desktop
    ? [...(desktopSaveName ? [] : ['a cataloged save'])]
    : [...(saveFile ? [] : ['a .v3 save']), ...(hasDefs ? [] : ['game definitions'])]
  // The archive only reads stored records, so it stays usable without inputs.
  // Saves / Settings stay available on desktop so paths and catalog work without a bind.
  const gated =
    missing.length > 0 &&
    activeView !== 'archive' &&
    !(desktop && (activeView === 'settings' || activeView === 'saves'))
  const defsCounts = defsSummary
    ? ` — format v${defsSummary.blob_version}, ${defsSummary.goods} goods, ${defsSummary.labels} names, ${defsSummary.icons} icons, ${defsSummary.production_methods} production methods`
    : ''
  // A real install has dozens of goods; a handful means the fixture blob or a
  // folder pick that missed common/goods.
  const thinDefs = Boolean(defsSummary && defsSummary.goods < 10)
  const pricesListView = !/^#\/prices\/(good|state|building)\//.test(locationHash)
  const navItems = desktop
    ? [
        { view: 'saves' as const, label: 'Saves' },
        ...WORKSPACE_NAV,
        { view: 'settings' as const, label: 'Settings' },
      ]
    : WORKSPACE_NAV

  return (
    <main>
      <header>
        <p className="eyebrow">LOCAL ECONOMY WORKBENCH</p>
        <h1>Victoria 3 Analyzer</h1>
        <p>
          {desktop
            ? 'Native companion: auto-detect game and saves, then analyze from disk — no browser upload.'
            : 'Offline market equilibrium solver, what-if simulator, and strategic planner.'}
        </p>
      </header>

      {desktop ? (
        <DesktopSaveChip
          loadedName={desktopSaveName}
          summaryTag={summary?.tag}
          summaryDate={summary?.date}
          busy={busy}
          onOpenSaves={() => selectView('saves')}
        />
      ) : (
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
              <button type="button" className="secondary" onClick={() => defsInputRef.current?.click()}>
                Choose definitions file…
              </button>
              <input
                ref={defsInputRef}
                aria-label="Definitions file"
                type="file"
                accept=".postcard,application/octet-stream"
                className="visually-hidden"
                onChange={(event) => {
                  const file = event.target.files?.[0]
                  if (file) applyDefsFile(file)
                }}
              />
              {defsFile && (
                <button type="button" className="secondary" onClick={() => applyDefsFile(undefined)}>
                  Forget these definitions
                </button>
              )}
            </div>
          </div>
        </div>
      </section>
      )}

      {pendingApply && result && (
        <ConfirmApply
          delta={pendingApply.delta}
          current={result}
          preview={pendingApply.preview}
          error={pendingApply.error}
          busy={busy}
          onConfirm={() => void confirmPendingApply()}
          onCancel={() => setPendingApply(undefined)}
        />
      )}

      {builderOpen && !desktop && (
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
            <strong>{hudGdp(result)}</strong>
          </div>
          <div>
            <span className="hud-label">SoL</span>
            <strong>{hudSol(result)}</strong>
          </div>
          <div>
            <span className="hud-label">Alerts</span>
            <strong>{alertsResult ? String(alertsResult.alerts.length) : '—'}</strong>
          </div>
          <div className="hud-actions">
            <button type="button" disabled={desktop || !saveFile} onClick={() => void downloadPatched()}>
              Download
            </button>
            {timelineStep?.parent_step_id ? (
              <button
                type="button"
                className="secondary"
                disabled={busy}
                onClick={() => void undoStep()}
              >
                Undo
              </button>
            ) : null}
          </div>
        </section>
      )}

      {error && <p role="alert">{error}</p>}

      <nav className="workspace-nav" aria-label="Analysis tools">
        {navItems.map(({ view, label }) => (
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
          {desktop
            ? `Analysis needs ${missing.join(' and ')}. Open Saves to pick one, or Settings if the game folder is wrong.`
            : !hasDefs && demoDefsStatus === 'loading'
              ? 'Loading definitions…'
              : `Analysis needs ${missing.join(' and ')}. Add ${
                  missing.length > 1 ? 'them' : 'it'
                } above; the tools below stay locked until then.`}
        </p>
      )}

      {busy && <ProgressBar label="Analyzing" />}

      {activeView === 'prices' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby={pricesListView ? 'prices-tool-heading' : undefined}
        >
          {(pricesListView || !result) && (
            <>
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
            </>
          )}
          {result ? (
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
                playerCountryId={summary?.country_id}
                playerMarketId={summary?.market_id}
                alerts={alertsResult?.alerts}
                onApply={(delta) => void requestApply(delta)}
              />
              <ModelInfo status={result.status} />
            </>
          ) : (
            <p>Prices appear after a save is priced.</p>
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
          alerts={alertsResult?.alerts}
          onApply={(delta) => void requestApply(delta)}
        />
      )}

      {activeView === 'pops' && (
        <PopsPane
          result={result}
          icons={goodIcons}
          playerCountryId={summary?.country_id}
          playerMarketId={summary?.market_id}
          gated={gated}
          alerts={alertsResult?.alerts}
          onApply={(delta) => void requestApply(delta)}
        />
      )}

      {activeView === 'alerts' && (
        <section
          className={gated ? 'workspace-page needs-defs' : 'workspace-page'}
          aria-labelledby="alerts-heading"
        >
          <h2 id="alerts-heading">Alerts</h2>
          {alertsResult ? (
            <AlertsPane
              result={alertsResult}
              icons={goodIcons}
              states={result?.states}
              buildings={result?.buildings}
              playerCountryId={summary?.country_id}
            />
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
          <div className="state-tabs" role="tablist" aria-label="Buildings views">
            {(['overview', 'queues'] as const).map((tab) => (
              <button
                type="button"
                key={tab}
                role="tab"
                aria-selected={buildingsTab === tab}
                onClick={() => {
                  setBuildingsTab(tab)
                  window.location.hash = hashForView('buildings', undefined, tab)
                }}
              >
                {tab === 'overview' ? 'Overview' : 'Queues'}
              </button>
            ))}
          </div>
          {buildingsTab === 'queues' ? (
            constructionsResult ? (
              <BuildQueuesPane snapshot={constructionsResult} icons={goodIcons} />
            ) : (
              <p>Construction queues appear after a save is priced.</p>
            )
          ) : (
            <BuildingsPane
              result={result}
              icons={goodIcons}
              playerCountryId={summary?.country_id}
              playerMarketId={summary?.market_id}
              gated={gated}
              api={api}
              alerts={alertsResult?.alerts}
              onWhatIf={(building, extraLevels) => {
                void applyWhatIf({ building, extra_levels: extraLevels })
              }}
              onApply={(delta) => void requestApply(delta)}
              embedded
            />
          )}
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
              timelineMode
            />
            <GoalBuilder
              key={`plan-${selectedTemplateId}`}
              idPrefix="Plan"
              goods={result?.goods.map((good) => good.name) ?? []}
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
            <button
              disabled={
                !ready ||
                busy ||
                !goal.trim() ||
                Boolean(selectedTemplate && !selectedTemplate.closesTimeline)
              }
              type="submit"
            >
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
              goods={result?.goods.map((good) => good.name) ?? []}
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
            <p>No unsatisfied simple subgoals.</p>
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

      {activeView === 'query' && <QueryPane />}

      {activeView === 'saves' && desktop && (
        <DesktopCatalog
          loadedName={desktopSaveName}
          refreshKey={catalogRefresh}
          onUseSave={useDesktopSave}
        />
      )}

      {activeView === 'settings' && desktop && (
        <SettingsPane onConfigChange={() => setCatalogRefresh((n) => n + 1)} />
      )}

      {activeView === 'what-if' && result && (
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
            scenario
            playerCountryId={summary?.country_id}
            playerMarketId={summary?.market_id}
            alerts={alertsResult?.alerts}
            onApply={(delta) => void requestApply(delta)}
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
                  <li key={`${formatGap(change.simple_subgoal)}-${index}`}>
                    {formatGap(change.simple_subgoal)} · {change.status.replaceAll('_', ' ')}
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
          Victoria 3 Analyzer v{__APP_VERSION__} ({__GIT_REVISION__})
        </span>
        <span>
          Built <time dateTime={__BUILD_TIME__}>{new Date(__BUILD_TIME__).toLocaleString()}</time>
        </span>
      </footer>
    </main>
  )
}

export default App
