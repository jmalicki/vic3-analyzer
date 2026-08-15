import { unzipSync } from 'fflate'
import { useMemo, useRef, useState, type InputHTMLAttributes } from 'react'
import {
  packDefsFiles,
  usefulDefsPath,
  type DefsSourceFile,
} from './defsFiles'
import { FieldHelp } from './FieldHelp'
import {
  canUseRememberedDirectoryPicker,
  pickGameCommonWithRememberedFolder,
  victoria3GameCommonPaths,
} from './savePicker'
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
  const rememberedFolder = canUseRememberedDirectoryPicker()

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

  const chooseFolder = async () => {
    if (rememberedFolder) {
      try {
        const picked = await pickGameCommonWithRememberedFolder()
        if (!picked) return
        const files = await Promise.all(
          picked
            .filter((entry) => usefulDefsPath(entry.path))
            .map(async (entry) => ({
              path: entry.path,
              bytes: new Uint8Array(await entry.file.arrayBuffer()),
            })),
        )
        await build(files)
        return
      } catch (reason) {
        setStatus(reason instanceof Error ? reason.message : String(reason))
        return
      }
    }
    folderInputRef.current?.click()
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
            prices. Files never leave the browser; Chromium can remember the folder after you choose
            it once.
          </p>
        </FieldHelp>
      </div>
      <p>
        Prices need base costs and recipes from <code>game/common</code>; the save alone does not
        carry them. Pick that folder (or a zip of it) to build a local defs blob.
      </p>
      <div className="defs-builder-actions">
        <button type="button" className="file-button secondary" onClick={() => void chooseFolder()}>
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
      <p className="path-hint">{commonPaths.summary}</p>
      <code className="path-hint-path">{commonPaths.local}</code>
      {status && <small role="status">{status}</small>}
    </div>
  )
}
