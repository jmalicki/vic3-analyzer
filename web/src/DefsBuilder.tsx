import { unzipSync } from 'fflate'
import { useState, type InputHTMLAttributes } from 'react'
import {
  packDefsFiles,
  usefulDefsPath,
  type DefsSourceFile,
} from './defsFiles'
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
      setStatus(`Built ${file.name} from ${manifest.length} definition files (${file.size.toLocaleString()} bytes).`)
    } catch (reason) {
      setStatus(reason instanceof Error ? reason.message : String(reason))
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
      <strong>Build definitions in this browser</strong>
      <p>
        Select your Victoria 3 <code>game/common</code> folder, or a zip containing it. Files are
        read locally and are never uploaded.
      </p>
      <div className="defs-builder-actions">
        <label className="file-button secondary">
          Choose game/common folder
          <input
            {...directoryProps}
            type="file"
            multiple
            aria-label="Victoria 3 definitions folder"
            onChange={(event) => {
              const files = event.target.files
              if (files) void browserFiles(files).then(build)
              event.target.value = ''
            }}
          />
        </label>
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
      {status && <small role="status">{status}</small>}
    </div>
  )
}
