import { beforeEach, describe, expect, it } from 'vitest'
import { clearAnalyses, listAnalyses, saveAnalysis } from './archive'
import type { AnalysisRecord } from './types'

describe('analysis archive', () => {
  beforeEach(clearAnalyses)

  it('saves then lists an AnalysisRecord with its blob', async () => {
    const record: AnalysisRecord = {
      id: 'record-1',
      created_at: '2026-08-15T12:00:00.000Z',
      kind: 'prices',
      fingerprint: 'abc123',
      opts: {},
      result: {
        goods: [],
        residual: 0,
        status: 'converged',
        limitations: ['Frozen world'],
      },
      limitations: ['Frozen world'],
      blob: { save: new Uint8Array([1, 2, 3]) },
    }

    await saveAnalysis(record)

    const records = await listAnalyses()
    expect(records).toHaveLength(1)
    expect(records[0]).toMatchObject({
      id: 'record-1',
      fingerprint: 'abc123',
      kind: 'prices',
    })
    expect(Array.from(records[0].blob?.save ?? [])).toEqual([1, 2, 3])
  })
})
