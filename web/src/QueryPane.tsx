import { useState, useEffect } from 'react'
import { isTauri } from './env'
import { invokeTauri } from './tauriApi'
import { Modal } from './Modal'

const EXAMPLES = [
  ["saves", "SELECT name, kind, loaded FROM saves ORDER BY mtime DESC LIMIT 10;"],
  ["alerts", "SELECT id, kind, severity, title FROM alerts() ORDER BY severity LIMIT 20;"],
  ["shortage", "SELECT good, shortage, price FROM goods WHERE shortage > 0 ORDER BY shortage DESC LIMIT 15;"],
  [
    "underemployed",
    "SELECT s.state_id, s.region_name, m.action, m.title\nFROM states s\nJOIN suggest_mitigations() m USING (state_id)\nWHERE is_underemployed(s.state_id)\nORDER BY m.rank\nLIMIT 20;",
  ],
  ["plan", "SELECT step, day, action, detail FROM plan('research(tech=nitroglycerin)') ORDER BY step;"],
]

export function QueryPane() {
  const [query, setQuery] = useState('SELECT * FROM states LIMIT 10')
  const [result, setResult] = useState<any[]>([])
  const [columns, setColumns] = useState<string[]>([])
  const [error, setError] = useState<string>('')
  const [showDocs, setShowDocs] = useState(false)
  const [docs, setDocs] = useState<{ sql_md: string, udf_md: string }>()

  useEffect(() => {
    if (isTauri()) {
      invokeTauri<{ sql_md: string, udf_md: string }>('sql_docs').then(setDocs).catch(console.error)
    }
  }, [])

  if (!isTauri()) {
    return (
      <section aria-labelledby="query-heading">
        <h2 id="query-heading">Advanced SQL Queries</h2>
        <div className="alert warning">
          <p>
            <strong>Tauri Desktop App Required</strong>
            <br/>
            The Advanced SQL Query engine requires native multithreading and C-based compression libraries, which are not currently available in the web version. Please use the Tauri Desktop app for SQL queries.
          </p>
        </div>
      </section>
    )
  }

  const runQuery = async () => {
    console.log('PAGE LOG: runQuery called!', query)
    try {
      setError('')
      const jsonStr = await invokeTauri<string>('sql_query', { sql: query })
      try {
        const res = JSON.parse(jsonStr)
        if (res && Array.isArray(res.columns) && Array.isArray(res.rows)) {
          const mappedRows = res.rows.map((rowArr: any[]) => {
            const obj: Record<string, any> = {}
            res.columns.forEach((col: string, i: number) => {
              obj[col] = rowArr[i]
            })
            return obj
          })
          setResult(mappedRows)
          setColumns(res.columns)
        } else if (Array.isArray(res)) {
          // Fallback in case backend returns array of objects directly
          setResult(res)
          if (res.length > 0) {
            setColumns(Object.keys(res[0]))
          } else {
            setColumns([])
          }
        } else {
          setError('Query returned unexpected format: ' + jsonStr)
          setResult([])
          setColumns([])
        }
      } catch (e) {
        setError('Invalid JSON returned: ' + jsonStr)
        setResult([])
        setColumns([])
      }
    } catch (e) {
      setError(String(e))
      setResult([])
      setColumns([])
    }
  }

  return (
    <section aria-labelledby="query-heading">
      <h2 id="query-heading">Advanced SQL Queries</h2>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '1rem', height: '100%' }}>
        
        <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap' }}>
          {EXAMPLES.map(([label, q]) => (
            <button key={label} type="button" data-ex={label} className="ghost" onClick={() => setQuery(q)}>{label}</button>
          ))}
        </div>

        <div style={{ display: 'flex', gap: '1rem', alignItems: 'flex-start' }}>
          <textarea
            id="sql-editor"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            style={{ flex: 1, minHeight: '150px', fontFamily: 'monospace', padding: '0.5rem' }}
            aria-label="SQL Query"
          />
          <div style={{ display: 'flex', flexDirection: 'column', gap: '0.5rem' }}>
            <button id="run-sql" type="button" className="primary" onClick={runQuery}>Execute</button>
            <button type="button" onClick={() => setShowDocs(true)}>View Dialect Docs</button>
          </div>
        </div>

        {error && <div className="alert error">{error}</div>}
        
        <div className="results-wrap">
          {columns.length > 0 ? (
            <table>
              <thead>
                <tr>
                  {columns.map(c => <th key={c}>{c}</th>)}
                </tr>
              </thead>
              <tbody id="results-body">
                {result.map((row, i) => (
                  <tr key={i}>
                    {columns.map(c => (
                      <td key={c} className={c === 'state_id' || c === 'good' ? 'nav-key' : ''} data-col={c}>
                        {typeof row[c] === 'object' ? JSON.stringify(row[c]) : String(row[c])}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            result.length === 0 && !error && <p>No results.</p>
          )}
        </div>
      </div>

      {showDocs && docs && (
        <Modal title="SQL Dialect Documentation" onClose={() => setShowDocs(false)}>
          <div style={{ padding: '1rem', maxHeight: '70vh', overflowY: 'auto' }}>
            <h3>Available UDFs and Tables</h3>
            <pre style={{ whiteSpace: 'pre-wrap', marginBottom: '1rem' }}>{docs.udf_md}</pre>
            <hr />
            <h3>SQL Guide</h3>
            <pre style={{ whiteSpace: 'pre-wrap' }}>{docs.sql_md}</pre>
          </div>
        </Modal>
      )}
    </section>
  )
}
