import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type InputHTMLAttributes,
} from 'react'
import {
  enumerateDroppedDefsFiles,
  isGfxDefsPath,
  isExtraIconDefsPath,
  neededGfxFile,
  packDefsFiles,
  selectedDefsFiles,
  streamDefsFiles,
  type DefsFileSource,
  type DefsPathClassifier,
  type DefsSourceFile,
} from './defsFiles'
import { FieldHelp } from './FieldHelp'
import { ProgressBar } from './ProgressBar'
import { victoria3GamePaths } from './savePicker'
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

/** Hand the event loop a turn so progress paints between wasm handoffs. */
function yieldToBrowser(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

export function DefsBuilder({ api, onBuilt, onDone, onBusyChange }: Props) {
  const [status, setStatus] = useState<string>()
  const [error, setError] = useState<string>()
  const [built, setBuilt] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [progress, setProgress] = useState<Progress>()
  const folderInputRef = useRef<HTMLInputElement>(null)
  const gamePaths = useMemo(() => victoria3GamePaths(), [])
  const busy = progress !== undefined

  useEffect(() => onBusyChange?.(busy), [busy, onBusyChange])

  const fail = (reason: unknown) => {
    setProgress(undefined)
    setStatus(undefined)
    setError(reason instanceof Error ? reason.message : String(reason))
  }

  /**
   * Run a streamed build.
   *
   * `pump` reads the source and hands over batches; each is packed, submitted
   * to wasm, and released before the next is read, so the tab never holds a
   * full install's coat-of-arms art at once. Yielding between batches lets the
   * progress bar actually repaint.
   */
  const build = async (
    locate: () => Promise<DefsFileSource[]>,
    label: string,
    emptyMessage: string,
  ) => {
    if (!api) {
      fail(new Error('The analysis engine is still loading. Try again in a moment.'))
      return
    }
    setProgress({ label })
    setBuilt(false)
    setStatus(undefined)
    setError(undefined)
    const classify: DefsPathClassifier = (path, isDirectory) =>
      api.classify_defs_path(path, isDirectory)
    const builder = new api.DefsBlobBuilder()
    let accepted = 0
    try {
      const sources = await locate()
      let total = sources.length
      setProgress({ label, done: 0, total })
      const submit = async (batch: DefsSourceFile[]) => {
        const packed = packDefsFiles(batch, classify)
        const manifest = JSON.parse(packed.manifestJson) as unknown[]
        if (manifest.length > 0) {
          builder.addBatch(packed.manifestJson, packed.contents)
          accepted += manifest.length
        }
        setProgress({ label, done: accepted, total })
        await yieldToBrowser()
      }

      // Definitions first: they name the art worth reading, and a full install
      // ships hundreds of emblems and icons that nothing points at.
      const text = sources.filter((source) => !isGfxDefsPath(source.path))
      const read = await streamDefsFiles(text, submit)
      if (accepted === 0) throw new Error(emptyMessage)

      const needed = new Set(JSON.parse(builder.neededGfxNames()) as string[])
      const art = sources.filter(
        (source) =>
          isGfxDefsPath(source.path) &&
          (isExtraIconDefsPath(source.path) || neededGfxFile(source.path, needed)),
      )
      total = text.length + art.length
      setProgress({ label, done: accepted, total })
      await streamDefsFiles(art, submit, { alreadyRead: read })
      setProgress({ label: 'Parsing definitions in wasm' })
      // finish() blocks the thread, so let the new label paint first.
      await yieldToBrowser()
      const bytes = builder.finish()
      const file = new File([bytes.slice().buffer as ArrayBuffer], 'defs.postcard', {
        type: 'application/octet-stream',
      })
      const summary = JSON.parse(await api.defs_summary(bytes)) as DefsSummary
      setProgress(undefined)
      setBuilt(true)
      onBuilt(file)
      setStatus(
        `Built ${file.name} format v${summary.blob_version} from ${accepted} definition files: ${summary.goods} goods, ${summary.labels} localized names, ${summary.icons} icons, ${summary.production_methods} production methods. Analysis tools are unlocked.` +
          (summary.goods < 10
            ? ' That is far fewer goods than a full install — common/goods was probably missed, so drag the common folder itself and rebuild.'
            : ''),
      )
    } catch (reason) {
      fail(reason)
    } finally {
      builder.free?.()
    }
  }

  const buildFromSelection = (list: FileList) => {
    if (!api) {
      fail(new Error('The analysis engine is still loading. Try again in a moment.'))
      return
    }
    const classify: DefsPathClassifier = (path, isDirectory) =>
      api.classify_defs_path(path, isDirectory)
    void build(
      async () => selectedDefsFiles(list, classify),
      'Reading definition files',
      'No supported common/*.txt definition files were found.',
    )
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
    const items = [...(event.dataTransfer.items ?? [])]
    await build(
      () => enumerateDroppedDefsFiles(items, classify),
      'Reading dropped files',
      'That drop had no supported definitions. Drag the Victoria 3 game folder itself.',
    )
  }

  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(gamePaths.local)
      setStatus('Path copied. Paste it into the folder dialog to jump straight there.')
    } catch {
      setStatus(`Copy this path into the folder dialog: ${gamePaths.local}`)
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
        them. Drag the <code>game</code> folder itself. Only the small allowlisted files are read.
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
          Choose game folder
        </button>
        {!api && (
          <small role="status">Waiting for the analysis engine…</small>
        )}
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
              buildFromSelection(files)
            } else {
              fail(new Error('No files came back from that folder. Nothing was read.'))
            }
            event.target.value = ''
          }}
        />
      </div>
      <p className="path-hint">{gamePaths.label}</p>
      <code className="path-hint-path">{gamePaths.local}</code>
      <div className="defs-builder-actions">
        <button type="button" className="secondary" onClick={() => void copyPath()}>
          Copy path
        </button>
      </div>
      <p className="path-hint">{gamePaths.summary}</p>
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
