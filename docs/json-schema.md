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
  "required": ["scope", "goods", "countries", "states", "state_goods", "buildings", "building_types", "building_groups", "state_pops", "inputs", "residual", "status", "limitations"],
  "properties": {
    "scope": { "const": "whole_save_synthetic" },
    "goods": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["id", "base", "price", "buy", "sell"],
        "properties": {
          "id": { "type": "string" },
          "name": { "type": ["string", "null"] },
          "base": { "type": "number" },
          "price": { "type": "number" },
          "buy": { "type": "number" },
          "sell": { "type": "number" }
        }
      }
    },
    "countries": {
      "type": "array",
      "description": "Save countries with tag, localized name, and optional current-flag CoA / PNG data URL.",
      "items": {
        "type": "object",
        "required": ["id", "tag"],
        "properties": {
          "id": { "type": "integer" },
          "tag": { "type": "string" },
          "name": { "type": ["string", "null"] },
          "flag_coa": { "type": ["string", "null"] },
          "flag_data_url": { "type": ["string", "null"] }
        }
      }
    },
    "states": { "type": "array", "description": "Parsed save state identity, owner/market links, arable land, and infrastructure." },
    "state_goods": {
      "type": "array",
      "description": "State-attributed buy/sell orders with the shared synthetic market price."
    },
    "buildings": {
      "type": "array",
      "description": "Per-instance modeled PM inputs/outputs, revenue, cost, profit, short inputs, and employees by profession."
    },
    "building_types": {
      "type": "array",
      "description": "Localized building ids with group and optional city type."
    },
    "building_groups": {
      "type": "array",
      "description": "Vic3 group category, land usage, parent, and broadly available default-building metadata."
    },
    "state_pops": {
      "type": "array",
      "description": "Collapsed state pops with workforce/dependents, literacy, qualifications, and model need baskets."
    },
    "state_qualifications": {
      "type": "array",
      "description": "Per-state profession stock vs employed jobs. Shortage is jobs minus employable/qualified; monthly qualification gain is omitted unless the save stores it."
    },
    "state_needs": {
      "type": "array",
      "description": "State-summed pop need baskets at solved prices (model consumption, not a save cashflow ledger)."
    },
    "inputs": {
      "type": "object",
      "description": "What the save and definitions contributed to the market.",
      "required": [
        "pops",
        "skipped_pops",
        "buildings",
        "skipped_buildings",
        "buildings_without_method",
        "buildings_without_orders",
        "goods_with_orders"
      ],
      "properties": {
        "pops": { "type": "integer", "description": "Pops whose consumption entered the solve." },
        "skipped_pops": { "type": "integer", "description": "Save pops dropped for missing workforce/dependents (or legacy population fields) or wealth." },
        "buildings": { "type": "integer", "description": "Buildings whose goods flows entered the solve." },
        "skipped_buildings": { "type": "integer", "description": "Save buildings dropped for a missing type id." },
        "buildings_without_method": {
          "type": "integer",
          "description": "Buildings with neither saved IO nor a production method present in the definitions."
        },
        "buildings_without_orders": {
          "type": "integer",
          "description": "Buildings with no non-zero saved IO and no usable PM fallback orders."
        },
        "goods_with_orders": {
          "type": "integer",
          "description": "Goods carrying a non-zero order. Zero means every price is just its base price."
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
`state_goods.price` blends `market_price` and `state_price` using
`effective_mapi = 0.75 * market_access`. Wage pops shop at that local price
inside the residual; access then scales their orders into the single market.
Access is infrastructure-only in this milestone. Post-1.9 trade is read from
each state. Extra MAPI modifiers and overseas constraints are not yet modeled.

A market with no orders prices every good at its base price and still converges
with a zero residual, so `inputs.goods_with_orders == 0` is the only way to tell
an empty market from a balanced one.

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
    "actions": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "left": { "type": "object" },
          "right": { "type": "object" }
        }
      }
    },
    "prices": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["good", "delta"],
        "properties": {
          "good": { "type": "string" },
          "delta": { "type": "number" }
        }
      }
    },
    "gaps": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["atom", "status"],
        "properties": {
          "atom": {},
          "status": { "enum": ["still_failing", "cleared", "newly_failing"] }
        }
      }
    }
  }
}
```

Empty diff when `left` and `right` are equal (I9).
