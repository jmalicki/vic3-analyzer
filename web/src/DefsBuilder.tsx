import { unzipSync } from 'fflate'
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

async function zippedFiles(file: File): Promise<DefsSourceFile[]> {
  const archive = unzipSync(new Uint8Array(await file.arrayBuffer()))
  return Object.entries(archive)
    .filter(([path]) => usefulDefsPath(path))
    .map(([path, bytes]) => ({ path, bytes }))
}

export function DefsBuilder({ api, onBuilt, onDone, onBusyChange }: Props) {
  const [status, setStatus] = useState<string>()
  const [error, setError] = useState<string>()
  const [blob, setBlob] = useState<File>()
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
    setStatus(undefined)
    setError(undefined)
    try {
      const packed = packDefsFiles(files)
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
      setBlob(file)
      onBuilt(file)
      setStatus(
        `Built ${file.name} from ${manifest.length} definition files: ${summary.goods} goods, ${summary.production_methods} production methods. Analysis tools are unlocked.` +
          (summary.goods < 10
            ? ' That is far fewer goods than a full install — common/goods was probably missed, so drag the common folder itself and rebuild.'
            : ''),
      )
    } catch (reason) {
      fail(reason)
    }
  }

  const readSelected = async (list: FileList): Promise<DefsSourceFile[]> => {
    const chosen = [...list].filter((file) =>
      usefulDefsPath(file.webkitRelativePath || file.name),
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
    try {
      const zip = [...event.dataTransfer.files].find((file) => file.name.endsWith('.zip'))
      if (zip) {
        setProgress({ label: 'Unpacking zip' })
        await build(await zippedFiles(zip))
        return
      }
      const label = 'Reading dropped files'
      setProgress({ label, done: 0 })
      const files = await collectDroppedDefsFiles(event.dataTransfer.items ?? [], (read) => {
        if (read % PROGRESS_STRIDE === 0) setProgress({ label, done: read })
      })
      if (files.length === 0) {
        fail(
          new Error(
            'That drop had no common/*.txt files. Drag the common folder itself (or game/common).',
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

  const download = () => {
    if (!blob) return
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = blob.name
    link.click()
    URL.revokeObjectURL(url)
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
            under your Victoria 3 install&apos;s <code>game/common</code> tree.
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
            a zip of <code>common</code> works too. If you do use the dialog, paste the copied path
            with <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> (macOS) or <kbd>Ctrl</kbd>+<kbd>L</kbd>{' '}
            (Linux).
          </p>
        </FieldHelp>
      </div>
      <p>
        Prices need base costs and recipes from <code>game/common</code>; the save alone does not
        carry them. Drag that folder here, or pick it (or a zip of it) below.
      </p>
      <div
        className={dragging ? 'defs-drop dragging' : 'defs-drop'}
        aria-label="Drop the game/common folder"
        onDragOver={(event) => {
          event.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => void handleDrop(event)}
      >
        <strong>Drag the <code>common</code> folder from Finder or Explorer</strong>
        <span>
          Dropping works even where the folder picker is blocked. In Steam: right-click Victoria 3 →
          Manage → Browse local files.
        </span>
      </div>
      <div className="defs-builder-actions">
        <button
          type="button"
          className="file-button secondary"
          disabled={busy}
          onClick={() => folderInputRef.current?.click()}
        >
          Choose game/common folder
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
        <label className="file-button secondary">
          Choose definitions zip
          <input
            type="file"
            accept=".zip,application/zip"
            aria-label="Victoria 3 definitions zip"
            onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) {
                setProgress({ label: 'Unpacking zip' })
                void zippedFiles(file).then(build).catch(fail)
              }
              event.target.value = ''
            }}
          />
        </label>
        {blob && (
          <button type="button" className="secondary" onClick={download}>
            Download defs.postcard
          </button>
        )}
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
      {blob && !busy && (
        <div className="defs-builder-actions">
          <button type="button" onClick={() => onDone?.()}>
            OK
          </button>
        </div>
      )}
    </div>
  )
}
