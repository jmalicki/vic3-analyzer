//! Wall times for a live save. Ignored unless `VIC3_SAVE` is set.
//!
//! ```text
//! VIC3_SAVE=… VIC3_TOKENS=… VIC3_DEFS=… \
//!   cargo test -p vic3-prices --release --test live_timing -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use vic3_defs::GameDefs;
use vic3_load::{empty_tokens, load_path_world, load_tokens_path};
use vic3_prices::{
    preview, solve, ExtraLevelsDelta, ProductionMethodDelta, SolveOpts, World, WorldDelta,
};

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) plus VIC3_GAME or VIC3_DEFS"]
fn live_solve_timings() {
    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
    let defs = if let Ok(path) = std::env::var("VIC3_DEFS") {
        vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode blob")
    } else {
        vic3_defs::load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME"))
            .expect("game defs")
    };

    let load_started = Instant::now();
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("tokens"),
        Err(_) => empty_tokens(),
    };
    let save = load_path_world(&save_path, tokens).expect("load save");
    let parse_elapsed = load_started.elapsed();
    let world_started = Instant::now();
    let world = World::from_save(&save, &defs);
    drop(save);
    let from_save_elapsed = world_started.elapsed();

    let with_pm = world
        .buildings
        .iter()
        .filter(|b| !b.production_methods.is_empty())
        .count();
    eprintln!(
        "parse {:?}  from_save {:?}  buildings {} ({} with PMs)  pops {}  goods {}",
        parse_elapsed,
        from_save_elapsed,
        world.buildings.len(),
        with_pm,
        world.pop_count(),
        defs.goods_order.len()
    );
    let clone_started = Instant::now();
    let _ = world.clone();
    eprintln!("world.clone {:?}", clone_started.elapsed());

    let cold = time_runs("cold solve", 3, || {
        solve(&world, &defs, SolveOpts::default())
    });
    let relative = cold.relative.clone();
    let warm_opts = with_warm_rel(&relative);
    time_runs("warm solve", 3, || solve(&world, &defs, warm_opts.clone()));

    let building = world.buildings.first().expect("live save has buildings");
    let extra = WorldDelta {
        extra_levels: vec![ExtraLevelsDelta {
            building: None,
            building_id: Some(building.id),
            extra_levels: 1,
        }],
        ..WorldDelta::default()
    };
    time_runs("preview +1 level (warm)", 3, || {
        preview(&world, &defs, &extra, with_warm_rel(&relative))
    });

    if let Some(delta) = pm_swap_delta(&world, &defs) {
        eprintln!(
            "pm swap building {} {:?} -> {:?}",
            delta.production_methods[0].building_id,
            world
                .buildings
                .iter()
                .find(|b| b.id == delta.production_methods[0].building_id)
                .map(|b| &b.production_methods),
            delta.production_methods[0].methods
        );
        time_runs("preview PM swap (warm)", 3, || {
            preview(&world, &defs, &delta, with_warm_rel(&relative))
        });
    } else {
        eprintln!("no alternate PM list on a shared building type; skipped PM swap");
    }
}

fn pm_swap_delta(world: &World, defs: &GameDefs) -> Option<WorldDelta> {
    for building in &world.buildings {
        let Some(pm) = building.production_methods.first() else {
            continue;
        };
        let alt = world.buildings.iter().find_map(|other| {
            if other.building == building.building
                && other.production_methods != building.production_methods
                && !other.production_methods.is_empty()
            {
                Some(other.production_methods.clone())
            } else {
                None
            }
        });
        if let Some(methods) = alt {
            return Some(WorldDelta {
                production_methods: vec![ProductionMethodDelta {
                    building_id: building.id,
                    methods,
                }],
                ..WorldDelta::default()
            });
        }
        let other_pm = defs.production_methods.keys().find(|id| *id != pm).cloned();
        if let Some(other) = other_pm {
            let mut methods = building.production_methods.clone();
            methods[0] = other;
            return Some(WorldDelta {
                production_methods: vec![ProductionMethodDelta {
                    building_id: building.id,
                    methods,
                }],
                ..WorldDelta::default()
            });
        }
    }
    None
}

fn with_warm_rel(relative: &[f64]) -> SolveOpts {
    SolveOpts {
        warm_rel: Some(relative.to_vec()),
        ..SolveOpts::default()
    }
}

fn time_runs<T>(label: &str, n: usize, mut run: impl FnMut() -> T) -> T {
    let mut last = None;
    let mut times = Vec::new();
    for i in 0..n {
        let started = Instant::now();
        last = Some(run());
        let elapsed = started.elapsed();
        times.push(elapsed);
        eprintln!("{label} run {} {:?}", i + 1, elapsed);
    }
    times.sort();
    eprintln!("{label} median {:?}", median(&times));
    last.expect("n > 0")
}

fn median(times: &[Duration]) -> Duration {
    times[times.len() / 2]
}
