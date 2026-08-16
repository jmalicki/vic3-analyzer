import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type InputHTMLAttributes,
} from 'react'
import {
  collectDroppedDefsFiles,
  packDefsFiles,
  usefulDefsPath,
  type DefsPathClassifier,
  type DefsSourceFile,
} from './defsFiles'
import { FieldHelp } from './FieldHelp'
import { ProgressBar } from './ProgressBar'
import { victoria3GameCommonPaths } from './savePicker'
import type { DefsSummary } from './types'
import type { WasmApi } from './wasm'

interface Props {
  api?: WasmApi
  onBuilt: (file: File) => void
  /** Called when the user acknowledges a finished build. */
  onDone?: () => void
  /** Reports whether work is in flight so the dialog cannot be dismissed. */
  onBusyChange?: (busy: boolean) => void
}

type Progress = {
  label: string
  done?: number
  total?: number
}

const directoryProps = {
  webkitdirectory: '',
  directory: '',
} as InputHTMLAttributes<HTMLInputElement>

/** Repaint every 32 files so a 3000-file install does not thrash React. */
const PROGRESS_STRIDE = 32

export function DefsBuilder({ api, onBuilt, onDone, onBusyChange }: Props) {
  const [status, setStatus] = useState<string>()
  const [error, setError] = useState<string>()
  const [built, setBuilt] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [progress, setProgress] = useState<Progress>()
  const folderInputRef = useRef<HTMLInputElement>(null)
  const commonPaths = useMemo(() => victoria3GameCommonPaths(), [])
  const busy = progress !== undefined

  useEffect(() => onBusyChange?.(busy), [busy, onBusyChange])

  const fail = (reason: unknown) => {
    setProgress(undefined)
    setStatus(undefined)
    setError(reason instanceof Error ? reason.message : String(reason))
  }

  const build = async (files: DefsSourceFile[]) => {
    if (!api) {
      fail(new Error('The analysis engine is still loading. Try again in a moment.'))
      return
    }
    setProgress({ label: 'Parsing definitions in wasm' })
    setBuilt(false)
    setStatus(undefined)
    setError(undefined)
    try {
      const classify: DefsPathClassifier = (path, isDirectory) =>
        api.classify_defs_path(path, isDirectory)
      const packed = packDefsFiles(files, classify)
      const manifest = JSON.parse(packed.manifestJson) as unknown[]
      if (manifest.length === 0) {
        throw new Error('No supported common/*.txt definition files were found.')
      }
      const bytes = await api.build_defs_blob(packed.manifestJson, packed.contents)
      const file = new File([bytes.slice().buffer as ArrayBuffer], 'defs.postcard', {
        type: 'application/octet-stream',
      })
      const summary = JSON.parse(await api.defs_summary(bytes)) as DefsSummary
      setProgress(undefined)
      setBuilt(true)
      onBuilt(file)
      setStatus(
        `Built ${file.name} format v${summary.blob_version} from ${manifest.length} definition files: ${summary.goods} goods, ${summary.labels} localized names, ${summary.production_methods} production methods. Analysis tools are unlocked.` +
          (summary.goods < 10
            ? ' That is far fewer goods than a full install — common/goods was probably missed, so drag the common folder itself and rebuild.'
            : ''),
      )
    } catch (reason) {
      fail(reason)
    }
  }

  const readSelected = async (list: FileList): Promise<DefsSourceFile[]> => {
    if (!api) throw new Error('The analysis engine is still loading. Try again in a moment.')
    const classify: DefsPathClassifier = (path, isDirectory) =>
      api.classify_defs_path(path, isDirectory)
    const chosen = [...list].filter((file) =>
      usefulDefsPath(file.webkitRelativePath || file.name, classify),
    )
    const label = 'Reading definition files'
    setProgress({ label, done: 0, total: chosen.length })
    const out: DefsSourceFile[] = []
    for (const file of chosen) {
      out.push({
        path: file.webkitRelativePath || file.name,
        bytes: new Uint8Array(await file.arrayBuffer()),
      })
      if (out.length % PROGRESS_STRIDE === 0 || out.length === chosen.length) {
        setProgress({ label, done: out.length, total: chosen.length })
      }
    }
    return out
  }

  const handleDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDragging(false)
    if (busy) return
    if (!api) {
      fail(new Error('The analysis engine is still loading. Try again in a moment.'))
      return
    }
    const classify: DefsPathClassifier = (path, isDirectory) =>
      api.classify_defs_path(path, isDirectory)
    try {
      const label = 'Reading dropped files'
      setProgress({ label, done: 0 })
      const files = await collectDroppedDefsFiles(
        event.dataTransfer.items ?? [],
        classify,
        (read) => {
          if (read % PROGRESS_STRIDE === 0) setProgress({ label, done: read })
        },
      )
      if (files.length === 0) {
        fail(
          new Error(
            'That drop had no supported definitions. Drag the Victoria 3 game folder (or game/common).',
          ),
        )
        return
      }
      await build(files)
    } catch (reason) {
      fail(reason)
    }
  }

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(commonPaths.local)
      setStatus('Path copied. Paste it into the folder dialog to jump straight there.')
    } catch {
      setStatus(`Copy this path into the folder dialog: ${commonPaths.local}`)
    }
  }

  return (
    <div className="defs-builder">
      <div className="field-label-row">
        <strong>Build definitions in this browser</strong>
        <FieldHelp label="Why definitions are needed">
          <p>
            Saves freeze the market situation — pops, buildings, trade volumes — but not the game
            rules those orders depend on. Base prices (<code>cost</code> in{' '}
            <code>common/goods</code>), production-method recipes, pop needs, and buy packages live
            under your Victoria 3 install&apos;s <code>game/common</code> tree. Display names live
            alongside it under <code>game/localization</code>.
          </p>
          <p>
            This builder packs those Clausewitz files into a local <code>defs.postcard</code> blob
            the price solver can use. Without it, only goods present in the tiny demo fixture get
            prices. Files never leave the browser.
          </p>
          <p>
            Chrome blocks Steam&apos;s install location in its folder APIs (<code>~/Library</code> on
            macOS, <code>Program Files</code> on Windows), and it will not open a path for you.
            <strong> Dragging the folder in is not restricted</strong>, so that is the reliable route;
            selecting it is entirely local and uses no upload bandwidth. If you use the dialog,
            paste the copied path with <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> (macOS) or{' '}
            <kbd>Ctrl</kbd>+<kbd>L</kbd> (Linux).
          </p>
        </FieldHelp>
      </div>
      <p>
        Prices need base costs and recipes from the game install; the save alone does not carry
        them. Drag the <code>game</code> folder for localized names, or <code>game/common</code> for
        definitions only. Only the small allowlisted files are read.
      </p>
      <div
        className={dragging ? 'defs-drop dragging' : 'defs-drop'}
        aria-label="Drop the Victoria 3 game folder"
        onDragOver={(event) => {
          event.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => void handleDrop(event)}
      >
        <strong>Drag the <code>game</code> folder from Finder or Explorer</strong>
        <span>
          The browser ignores heavy folders such as gfx and sound. Dropping works even where the
          folder picker is blocked. In Steam: right-click Victoria 3 → Manage → Browse local files.
        </span>
      </div>
      <div className="defs-builder-actions">
        <button
          type="button"
          className="file-button secondary"
          disabled={busy || !api}
          onClick={() => folderInputRef.current?.click()}
        >
          Choose game or game/common folder
        </button>
        <input
          {...directoryProps}
          ref={folderInputRef}
          type="file"
          multiple
          className="visually-hidden"
          aria-label="Victoria 3 definitions folder"
          onChange={(event) => {
            const files = event.target.files
            if (files?.length) {
              void readSelected(files).then(build).catch(fail)
            } else {
              fail(new Error('No files came back from that folder. Nothing was read.'))
            }
            event.target.value = ''
          }}
        />
      </div>
      <p className="path-hint">{commonPaths.label}</p>
      <code className="path-hint-path">{commonPaths.local}</code>
      <div className="defs-builder-actions">
        <button type="button" className="secondary" onClick={() => void copyPath()}>
          Copy path
        </button>
      </div>
      <p className="path-hint">{commonPaths.summary}</p>
      {progress && (
        <ProgressBar label={progress.label} done={progress.done} total={progress.total} />
      )}
      {error && (
        <p className="builder-error" role="alert">
          {error}
        </p>
      )}
      {status && <small role="status">{status}</small>}
      {built && !busy && (
        <div className="defs-builder-actions">
          <button type="button" onClick={() => onDone?.()}>
            OK
          </button>
        </div>
      )}
    </div>
  )
}
