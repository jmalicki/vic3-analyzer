import type { DefsBlobBuilder, WasmApi } from './wasm'

export function invokeTauri<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const win = window as any
  if (win.__TAURI__?.core?.invoke) {
    return win.__TAURI__.core.invoke(cmd, args)
  }
  return Promise.reject(new Error("Not in Tauri"))
}

class TauriDefsBuilder implements DefsBlobBuilder {
  addBatch(_manifestJson: string, _contents: Uint8Array): Promise<void> {
    throw new Error('Defs building not supported over Tauri bridge yet.')
  }
  neededGfxNames(): Promise<string> {
    throw new Error('Defs building not supported over Tauri bridge yet.')
  }
  finish(): Promise<Uint8Array> {
    throw new Error('Defs building not supported over Tauri bridge yet.')
  }
  free(): void {}
}

export function createTauriApi(): WasmApi {
  return {
    classify_defs_path: () => 'skip', // unused in tauri frontend
    DefsBlobBuilder: TauriDefsBuilder,
    what_if_schema: () => "{}", // Unused currently in React?
    prices_schema: () => "{}",
    build_defs_blob: async () => { throw new Error('Not supported in Tauri API') },
    defs_summary: async () => "{}", // Handled by config in Tauri
    // Blob arg ignored: desktop session already owns the defs postcard path.
    defs_icons: async () => await invokeTauri<string>('loaded_defs_icons'),
    parse_save: async () => "{}", // Tauri uses use_save
    load_analysis: async () => "{}", // Tauri uses use_save
    clear_analysis: async () => {},
    loaded_prices: async () => await invokeTauri<string>('loaded_prices'),
    loaded_military: async () => await invokeTauri<string>('loaded_military'),
    loaded_constructions: async () => await invokeTauri<string>('loaded_constructions'),
    export_save: async () => { throw new Error('Export save not currently supported in Tauri API via invoke') },
    loaded_what_if: async () => "{}",
    loaded_apply_delta: async () => "{}",
    loaded_optimize_pms: async () => "{}",
    loaded_gaps: async (goal) => await invokeTauri<string>('loaded_gaps', { goal }),
    loaded_plan: async (planOptsJson) => await invokeTauri<string>('loaded_plan', { planOptsJson }),
    loaded_alerts: async () => await invokeTauri<string>('loaded_alerts'),
    loaded_production_methods: async () => "{}",
    prices: async () => await invokeTauri<string>('loaded_prices'),
    what_if: async () => "{}",
    gaps: async (_s, _t, _d, _so, goal) => await invokeTauri<string>('loaded_gaps', { goal }),
    plan: async (_s, _t, _d, _so, planOptsJson) => await invokeTauri<string>('loaded_plan', { planOptsJson })
  }
}
