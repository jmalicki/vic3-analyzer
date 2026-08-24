# JSON Schema Contracts

This document specifies the shared JSON Schema (draft 2020-12) contracts for options, mutations, and analytical result structures.

Shared structs derive `schemars` and serde serialization, ensuring identical data contracts across the CLI (`--json`), Web UI, Desktop companion, and MCP server.

Core option structs never contain `PathBuf` or OS-specific filesystem fields.

## SolveOpts

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "residual_eps": { "type": "number", "default": 1e-6 },
    "max_iters": { "type": "integer", "minimum": 1, "default": 100 },
    "warm_rel": {
      "type": "array",
      "items": { "type": "number" },
      "description": "Previous relative prices in goods-with-base-price order. Ignored when length does not match; omitted when unset."
    }
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

## WorldDelta

Preview mutation applied to a cloned world (extra levels, then production methods). Does not write a save. Subsidy entries are accepted and ignored.

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "production_methods": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["building_id", "methods"],
        "additionalProperties": false,
        "properties": {
          "building_id": { "type": "integer", "minimum": 0 },
          "methods": { "type": "array", "items": { "type": "string" } }
        }
      }
    },
    "extra_levels": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["extra_levels"],
        "additionalProperties": false,
        "properties": {
          "building": { "type": "string", "description": "Building type id; used when building_id is omitted." },
          "building_id": { "type": "integer", "minimum": 0, "description": "Single instance; wins over building when both are set." },
          "extra_levels": { "type": "integer", "minimum": 0 }
        }
      }
    },
    "subsidize": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["building_id", "enabled"],
        "additionalProperties": false,
        "properties": {
          "building_id": { "type": "integer", "minimum": 0 },
          "enabled": { "type": "boolean" }
        }
      }
    }
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
    "allow_pm_changes": {
      "type": "boolean",
      "default": false,
      "description": "When true, planning may emit SwitchPm edges (off by default)"
    },
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
    "limitations": { "type": "array", "items": { "type": "string" } },
    "relative": {
      "type": "array",
      "items": { "type": "number" },
      "description": "Relative prices (price/base) matching goods order; omitted when empty. Feed back as SolveOpts.warm_rel."
    }
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

## OptimizePmsOpts

```json
{
  "type": "object",
  "required": ["axis"],
  "additionalProperties": false,
  "properties": {
    "axis": { "enum": ["income", "productivity", "sol"] }
  }
}
```

## OptimizeResult

Greedy production-method suggestion. `world_delta` is a [`WorldDelta`] for a later apply. `prices` is a compact summary (residual, status, scored income / productivity / SoL), not a full `PricesResult`.

```json
{
  "type": "object",
  "required": ["axis", "changes", "delta", "limitations", "world_delta"],
  "properties": {
    "axis": { "enum": ["income", "productivity", "sol"] },
    "changes": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["building_type", "building_id", "from", "to"],
        "properties": {
          "building_type": { "type": "string" },
          "building_id": { "type": "integer", "minimum": 0 },
          "from": { "type": "array", "items": { "type": "string" } },
          "to": { "type": "array", "items": { "type": "string" } }
        }
      }
    },
    "delta": {
      "type": "object",
      "required": ["income", "productivity", "sol", "residual"],
      "properties": {
        "income": { "type": "number" },
        "productivity": { "type": "number" },
        "sol": { "type": "number" },
        "residual": { "type": "number" }
      }
    },
    "prices": {
      "type": "object",
      "description": "Compact last-trial summary; omitted only if serialization skips it."
    },
    "limitations": { "type": "array", "items": { "type": "string" } },
    "world_delta": { "$ref": "#/WorldDelta" }
  }
}
```

## AlertsResult

Shortage expanders from `alerts(world, defs, prices)`. `severity` `1` is urgent; underemployed buildings are `2`. Mitigations may include a tagged `action` used by Apply. Shortage interventions include an `effect` from a local IO perturbation (pop demand held, closed-form price step). PM advice names a specific method already in use on that building type. Qualification advice is filtered from `pop_types` + the state's mix; copy is in `crates/vic3-prices/advice/qualifications/`. Employment alerts are per state and may include a `staffing` array of buildings with per-profession gaps (`employed_here` / `jobs_here` / `missing_here` and state `jobs` / `stock` / `shortage`).

```json
{
  "type": "object",
  "required": ["alerts", "limitations"],
  "properties": {
    "alerts": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["id", "kind", "severity", "title", "summary", "evidence", "mitigations"],
        "properties": {
          "id": { "type": "string" },
          "kind": {
            "enum": [
              "electricity_shortage",
              "transportation_shortage",
              "goods_shortage",
              "needs_unmet",
              "low_market_access",
              "unfilled_education",
              "unfilled_pops",
              "underemployed"
            ]
          },
          "severity": { "type": "integer", "minimum": 1 },
          "title": { "type": "string" },
          "summary": { "type": "string" },
          "state_id": { "type": "integer", "minimum": 0 },
          "building_id": { "type": "integer", "minimum": 0 },
          "good_id": { "type": "string" },
          "evidence": {
            "type": "array",
            "items": {
              "type": "object",
              "required": ["label", "value"],
              "properties": {
                "label": { "type": "string" },
                "value": { "type": "string" }
              }
            }
          },
          "mitigations": {
            "type": "array",
            "items": {
              "type": "object",
              "required": ["id", "title", "detail", "rank", "apply_ready"],
              "properties": {
                "id": { "type": "string" },
                "title": { "type": "string" },
                "detail": { "type": "string" },
                "rank": { "type": "integer", "minimum": 0 },
                "apply_ready": { "type": "boolean" },
                "action": { "type": "object" },
                "effect": { "type": "string" }
              }
            }
          },
          "staffing": {
            "type": "array",
            "items": {
              "type": "object",
              "required": [
                "building_id",
                "building_name",
                "type_id",
                "staffing",
                "level",
                "professions"
              ],
              "properties": {
                "building_id": { "type": "integer" },
                "building_name": { "type": "string" },
                "type_id": { "type": "string" },
                "staffing": { "type": "number" },
                "level": { "type": "number" },
                "professions": {
                  "type": "array",
                  "items": {
                    "type": "object",
                    "required": [
                      "profession_id",
                      "employed_here",
                      "jobs_here",
                      "missing_here",
                      "state_jobs",
                      "state_stock",
                      "state_shortage"
                    ],
                    "properties": {
                      "profession_id": { "type": "string" },
                      "profession_name": { "type": "string" },
                      "employed_here": { "type": "number" },
                      "jobs_here": { "type": "number" },
                      "missing_here": { "type": "number" },
                      "state_jobs": { "type": "number" },
                      "state_stock": { "type": "number" },
                      "state_shortage": { "type": "number" }
                    }
                  }
                }
              }
            }
          }
        }
      }
    },
    "limitations": { "type": "array", "items": { "type": "string" } }
  }
}
```

## ConstructionsSnapshot

Player-scoped build queues from `loaded_constructions` / Buildings → Queues. Same projection as SQL `constructions` (`World.constructions`), not the single planning head `queued_building`.

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["private", "government"],
  "properties": {
    "private": {
      "type": "array",
      "items": {
        "allOf": [
          { "$ref": "#/$defs/ConstructionOrderSnapshot" },
          { "properties": { "queue": { "const": "private" } } }
        ]
      }
    },
    "government": {
      "type": "array",
      "items": {
        "allOf": [
          { "$ref": "#/$defs/ConstructionOrderSnapshot" },
          { "properties": { "queue": { "const": "government" } } }
        ]
      }
    }
  },
  "$defs": {
    "ConstructionOrderSnapshot": {
      "type": "object",
      "additionalProperties": false,
      "required": ["id", "queue", "building", "country_id", "state_id", "state_name", "building_name", "remaining"],
      "properties": {
        "id": { "type": "integer" },
        "queue": { "type": "string", "enum": ["private", "government"] },
        "country_id": { "type": ["integer", "null"] },
        "state_id": { "type": ["integer", "null"] },
        "state_name": { "type": ["string", "null"] },
        "building": { "type": "string" },
        "building_name": { "type": ["string", "null"] },
        "remaining": { "type": ["number", "null"] }
      }
    }
  }
}
```
