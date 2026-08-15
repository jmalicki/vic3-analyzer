import type { JsonSchema } from './types'

interface Props {
  schema: JsonSchema
  value: Record<string, unknown>
  onChange: (value: Record<string, unknown>) => void
}

function titleFor(name: string, schema: JsonSchema): string {
  return schema.title ?? name.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
}

function resolve(schema: JsonSchema, root: JsonSchema): JsonSchema {
  if (!schema.$ref?.startsWith('#/$defs/')) return schema
  return root.$defs?.[schema.$ref.slice('#/$defs/'.length)] ?? schema
}

function SchemaObject({
  schema,
  root,
  value,
  onChange,
  legend,
}: {
  schema: JsonSchema
  root: JsonSchema
  value: Record<string, unknown>
  onChange: (value: Record<string, unknown>) => void
  legend?: string
}) {
  const fields = Object.entries(schema.properties ?? {})
  const contents = fields.map(([name, unresolved]) => {
    const field = resolve(unresolved, root)
    const required = schema.required?.includes(name) ?? false
    if (field.type === 'object' || field.properties) {
      return (
        <SchemaObject
          key={name}
          schema={field}
          root={root}
          legend={titleFor(name, field)}
          value={(value[name] as Record<string, unknown> | undefined) ?? {}}
          onChange={(nested) => onChange({ ...value, [name]: nested })}
        />
      )
    }

    const numeric = field.type === 'number' || field.type === 'integer'
    return (
      <label className="schema-field" key={name}>
        <span>{titleFor(name, field)}</span>
        <input
          aria-label={titleFor(name, field)}
          name={name}
          type={numeric ? 'number' : 'text'}
          step={field.type === 'integer' ? 1 : 'any'}
          min={field.minimum}
          required={required}
          value={String(value[name] ?? field.default ?? '')}
          onChange={(event) => {
            const next = numeric
              ? event.target.value === ''
                ? ''
                : Number(event.target.value)
              : event.target.value
            onChange({ ...value, [name]: next })
          }}
        />
        {field.description && <small>{field.description}</small>}
      </label>
    )
  })

  if (!legend) return <>{contents}</>
  return (
    <fieldset>
      <legend>{legend}</legend>
      {contents}
    </fieldset>
  )
}

export function SchemaForm({ schema, value, onChange }: Props) {
  return <SchemaObject schema={schema} root={schema} value={value} onChange={onChange} />
}
