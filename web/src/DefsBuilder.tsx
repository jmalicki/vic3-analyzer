import { unzipSync } from 'fflate'
import { useMemo, useRef, useState, type InputHTMLAttributes } from 'react'
import {
  packDefsFiles,
  usefulDefsPath,
  type DefsSourceFile,
} from './defsFiles'
import { FieldHelp } from './FieldHelp'
import { victoria3GameCommonPaths } from './savePicker'
import type { WasmApi } from './wasm'

interface Props {
  api?: WasmApi
  onBuilt: (file: File) => void
}

const directoryProps = {
  webkitdirectory: '',
  directory: '',
} as InputHTMLAttributes<HTMLInputElement>

async function browserFiles(files: FileList): Promise<DefsSourceFile[]> {
  return Promise.all(
    [...files]
      .filter((file) => usefulDefsPath(file.webkitRelativePath || file.name))
      .map(async (file) => ({
        path: file.webkitRelativePath || file.name,
        bytes: new Uint8Array(await file.arrayBuffer()),
      })),
  )
}

async function zippedFiles(file: File): Promise<DefsSourceFile[]> {
  const archive = unzipSync(new Uint8Array(await file.arrayBuffer()))
  return Object.entries(archive)
    .filter(([path]) => usefulDefsPath(path))
    .map(([path, bytes]) => ({ path, bytes }))
}

export function DefsBuilder({ api, onBuilt }: Props) {
  const [status, setStatus] = useState<string>()
  const [blob, setBlob] = useState<File>()
  const folderInputRef = useRef<HTMLInputElement>(null)
  const commonPaths = useMemo(() => victoria3GameCommonPaths(), [])

  const build = async (files: DefsSourceFile[]) => {
    if (!api) return
    setStatus('Building definitions…')
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
      setBlob(file)
      onBuilt(file)
      setStatus(
        `Built ${file.name} from ${manifest.length} definition files (${file.size.toLocaleString()} bytes).`,
      )
    } catch (reason) {
      setStatus(reason instanceof Error ? reason.message : String(reason))
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
            Browsers cannot open a path for you, and Chrome&apos;s newer folder API refuses Steam&apos;s
            install location outright (<code>~/Library</code> on macOS, <code>Program Files</code> on
            Windows). Copy the path below, then paste it in the folder dialog — macOS{' '}
            <kbd>Cmd</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd>, Linux <kbd>Ctrl</kbd>+<kbd>L</kbd>, Windows
            address bar. Picking a zip of <code>common</code> avoids the dialog entirely.
          </p>
        </FieldHelp>
      </div>
      <p>
        Prices need base costs and recipes from <code>game/common</code>; the save alone does not
        carry them. Pick that folder (or a zip of it) to build a local defs blob.
      </p>
      <div className="defs-builder-actions">
        <button
          type="button"
          className="file-button secondary"
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
            if (files) void browserFiles(files).then(build)
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
              if (file) void zippedFiles(file).then(build)
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
      {status && <small role="status">{status}</small>}
    </div>
  )
}
