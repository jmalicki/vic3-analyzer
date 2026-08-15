# JSON Schema (draft)

P1 hand-written contract. P5 generates schemars from shared structs and **fails the test** if this file (or a checked-in `schema/*.json`) drifts.

Types below are JSON Schema 2020-12 sketches. `PathBuf` never appears. Bytes are base64 only if we ever embed a blob in JSON; IndexedDB stores blobs out of band.

## SolveOpts

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "residual_eps": { "type": "number", "default": 1e-6 },
    "max_iters": { "type": "integer", "minimum": 1, "default": 100 }
  }
}
```

## WhatIfOpts

```json
{
  "type": "object",
  "required": ["building", "extra_levels"],
  "additionalProperties": false,
  "properties": {
    "building": { "type": "string" },
    "extra_levels": { "type": "integer", "minimum": 0 }
  }
}
```

## PlanOpts

```json
{
  "type": "object",
  "required": ["goal"],
  "additionalProperties": false,
  "properties": {
    "goal": { "type": "string", "description": "DSL string, see docs/dsl.md" },
    "max_days": { "type": "integer", "minimum": 0 },
    "label": { "type": "string" },
    "parent_id": { "type": "string", "format": "uuid" }
  }
}
```

## PricesResult

```json
{
  "type": "object",
  "required": ["goods", "residual", "status", "limitations"],
  "properties": {
    "goods": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["id", "base", "price", "buy", "sell"],
        "properties": {
          "id": { "type": "string" },
          "base": { "type": "number" },
          "price": { "type": "number" },
          "buy": { "type": "number" },
          "sell": { "type": "number" }
        }
      }
    },
    "residual": { "type": "number" },
    "status": { "enum": ["converged", "max_iters", "failed"] },
    "limitations": { "type": "array", "items": { "type": "string" } }
  }
}
```

`status = converged` implies `residual < SolveOpts.residual_eps` (I5).

## AnalysisRecord

```json
{
  "type": "object",
  "required": ["id", "created_at", "kind", "fingerprint", "opts", "result", "limitations"],
  "properties": {
    "id": { "type": "string", "format": "uuid" },
    "created_at": { "type": "string", "format": "date-time" },
    "label": { "type": "string" },
    "kind": { "enum": ["prices", "what_if", "gaps", "plan"] },
    "fingerprint": { "type": "string" },
    "date": { "type": "string" },
    "country": { "type": "string" },
    "filename": { "type": "string" },
    "opts": { "type": "object" },
    "result": { "type": "object" },
    "limitations": { "type": "array", "items": { "type": "string" } },
    "parent_id": { "type": "string", "format": "uuid" }
  }
}
```

Save bytes are **not** in this JSON when talking to the UI list endpoint; they live in IndexedDB / sibling blob files keyed by `fingerprint`.

## CompareResult (P11)

```json
{
  "type": "object",
  "required": ["left", "right", "same_fingerprint"],
  "properties": {
    "left": { "type": "string", "format": "uuid" },
    "right": { "type": "string", "format": "uuid" },
    "same_fingerprint": { "type": "boolean" },
    "day_cost_delta": { "type": "integer" },
    "actions": { "type": "array" },
    "prices": { "type": "array" },
    "gaps": { "type": "array" }
  }
}
```

Empty diff when `left` and `right` are equal (I9).
