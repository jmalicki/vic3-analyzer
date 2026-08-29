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
    preview, solve, ExtraLevelsDelta, ProductionMethodDelta, SolveOpts, SolveStrategy, World,
    WorldDelta,
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
            building_type_id: None,
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
            if other.building_type_id == building.building_type_id
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

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) plus VIC3_GAME or VIC3_DEFS"]
fn live_equilibrate_vs_solve_cold() {
    use vic3_prices::equilibrate;
    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
    let defs = if let Ok(path) = std::env::var("VIC3_DEFS") {
        vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode blob")
    } else {
        vic3_defs::load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME"))
            .expect("game defs")
    };
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("tokens"),
        Err(_) => empty_tokens(),
    };
    let save = load_path_world(&save_path, tokens).expect("load save");
    let world = World::from_save(&save, &defs);
    drop(save);

    // Warm up once each.
    let _ = equilibrate(&world, &defs, SolveOpts::default());
    let _ = solve(&world, &defs, SolveOpts::default());

    time_runs("cold equilibrate (no report)", 5, || {
        equilibrate(&world, &defs, SolveOpts::default())
    });
    time_runs("cold solve (full PricesResult)", 5, || {
        solve(&world, &defs, SolveOpts::default())
    });
}

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) plus VIC3_GAME or VIC3_DEFS"]
fn live_equilibrate_warm_vs_cold() {
    use vic3_prices::equilibrate;
    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
    let defs = if let Ok(path) = std::env::var("VIC3_DEFS") {
        vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode blob")
    } else {
        vic3_defs::load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME"))
            .expect("game defs")
    };
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("tokens"),
        Err(_) => empty_tokens(),
    };
    let save = load_path_world(&save_path, tokens).expect("load save");
    let world = World::from_save(&save, &defs);
    drop(save);

    let building = world.buildings.first().expect("buildings");
    let bumped = world.clone().with_extra_levels_on_id(building.id, 1);

    let baseline = equilibrate(&world, &defs, SolveOpts::default());
    let relative = baseline.relative.clone();

    // Warm caches / code once.
    let _ = equilibrate(&bumped, &defs, SolveOpts::default());
    let _ = equilibrate(&bumped, &defs, with_warm_rel(&relative));

    time_runs("cold equilibrate on +1 level", 5, || {
        equilibrate(&bumped, &defs, SolveOpts::default())
    });
    time_runs(
        "warm equilibrate on +1 level (from baseline rel)",
        5,
        || equilibrate(&bumped, &defs, with_warm_rel(&relative)),
    );

    // Same-world warm (identity) — lower bound on warm benefit.
    time_runs("cold equilibrate same world", 5, || {
        equilibrate(&world, &defs, SolveOpts::default())
    });
    time_runs("warm equilibrate same world", 5, || {
        equilibrate(&world, &defs, with_warm_rel(&relative))
    });
}

#[test]
#[ignore = "set VIC3_SAVE (and VIC3_TOKENS for binary) plus VIC3_GAME or VIC3_DEFS"]
fn live_nested_vs_joint_equilibrate() {
    use vic3_prices::equilibrate;

    let save_path = std::env::var("VIC3_SAVE").expect("VIC3_SAVE");
    let defs = if let Ok(path) = std::env::var("VIC3_DEFS") {
        vic3_defs::decode_blob(&std::fs::read(path).expect("defs blob")).expect("decode blob")
    } else {
        vic3_defs::load_from_path(std::env::var("VIC3_GAME").expect("VIC3_GAME"))
            .expect("game defs")
    };
    let tokens = match std::env::var("VIC3_TOKENS") {
        Ok(path) => load_tokens_path(path).expect("tokens"),
        Err(_) => empty_tokens(),
    };
    let save = load_path_world(&save_path, tokens).expect("load save");
    let world = World::from_save(&save, &defs);
    drop(save);

    let nested_opts = SolveOpts {
        strategy: SolveStrategy::Nested,
        ..SolveOpts::default()
    };
    let joint_opts = SolveOpts {
        strategy: SolveStrategy::Joint,
        ..SolveOpts::default()
    };

    // Warm up once each (Joint aliases Nested until a later PR).
    let _ = equilibrate(&world, &defs, nested_opts.clone());
    let _ = equilibrate(&world, &defs, joint_opts.clone());

    let nested = time_runs("nested equilibrate", 5, || {
        equilibrate(&world, &defs, nested_opts.clone())
    });
    let joint = time_runs("joint equilibrate", 5, || {
        equilibrate(&world, &defs, joint_opts.clone())
    });

    eprintln!(
        "nested stats strategy={:?} param_dim={} residual_evals={} jacobian_evals={} residual={}",
        nested.stats.strategy,
        nested.stats.param_dim,
        nested.stats.n_residual_evals,
        nested.stats.n_jacobian_evals,
        nested.residual
    );
    eprintln!(
        "joint stats strategy={:?} param_dim={} residual_evals={} jacobian_evals={} residual={}",
        joint.stats.strategy,
        joint.stats.param_dim,
        joint.stats.n_residual_evals,
        joint.stats.n_jacobian_evals,
        joint.residual
    );
}
