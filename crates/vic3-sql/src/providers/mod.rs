//! Fact-table providers plus catalog host registration.
//!
//! # Namespaces
//!
//! | Registration | Unqualified + `active.*` | `latest.*` | `saves` |
//! | --- | --- | --- | --- |
//! | [`register_all`] | bound snapshot | — | — |
//! | `register_catalog_host` | [`UnboundFactProvider`] until `use_save` | [`LatestFactProvider`] | [`SavesProvider`] |
//!
//! After `use_save`, [`register_all`] replaces unbound placeholders so default
//! search path ≡ `active`.
//!
//! # Player vs world scope
//!
//! Short names (`states`, `countries`, …) and matching `active.*` / `latest.*`
//! are **player-owned only** (strict `World.player_tag`, same as `alerts()`).
//! Full-save twins are registered as `world_<table>` / `active.world_*` /
//! `latest.world_*`. `goods`, `building_types`, and `production_methods` have
//! no `world_` twin (session market snapshot / defs).
//!
//! # Pushdown (Exact vs range Exact)
//!
//! Each fact provider declares a [`pushdown::PushSupport`]:
//!
//! | Table | Exact `=` | Exact range |
//! | --- | --- | --- |
//! | `states` | `state_id`, `region_name`, `region_label`, `label`, `owner_tag` | — |
//! | `goods` | `name`, `label` | — (hash / order index) |
//! | `goods_by_state` | `state_id`, `good` | — |
//! | `buildings` | `building_id`, `state_id`, `building_type_name` | — |
//! | `building_types` | `name`, `label`, `group_id` | `name` (defs `BTreeMap`) |
//! | `production_methods` | `pm`, `pm_name` | `pm` (defs `BTreeMap`) |
//! | `pops` | `state_id`, `profession` | — |
//! | `state_qualifications` | `state_id`, `profession` | — |
//! | `countries` | `country_id`, `name`, `label` | — |
//! | `constructions` | `order_id`, `country_id`, `state_id`, `queue`, `building_type_name` | — |
//!
//! Exact means the provider filters rows itself; Unsupported predicates are
//! left for DataFusion. `latest.*` advertises Inexact and delegates after load.
//!
//! # List / IO columns
//!
//! Built by the `arrays` helper: `TEXT[]` for PM ids / short inputs / PM groups;
//! `List<Struct{good, good_name, qty}>` for building and PM goods IO.

mod arrays;
mod building_types;
mod buildings;
mod constructions;
mod countries;
mod goods;
mod goods_by_state;
mod latest;
mod pops;
mod production_methods;
pub mod pushdown;
mod saves;
mod state_qualifications;
mod states;

use std::sync::Arc;

use datafusion::catalog::{MemorySchemaProvider, TableProvider};
use datafusion::common::TableReference;
use datafusion::prelude::SessionContext;

use crate::binding::SessionBinding;
use crate::host::HostState;
use crate::schema::{
    building_types_schema, buildings_schema, constructions_schema, countries_schema,
    goods_by_state_schema, goods_schema, pops_schema, production_methods_schema,
    state_qualifications_schema, states_schema,
};
use crate::scope::TableScope;
use crate::SqlError;

pub use building_types::BuildingTypesProvider;
pub use buildings::BuildingsProvider;
pub use constructions::ConstructionsProvider;
pub use countries::CountriesProvider;
pub use goods::GoodsProvider;
pub use goods_by_state::GoodsByStateProvider;
pub use latest::{LatestFactProvider, UnboundFactProvider};
pub use pops::PopsProvider;
pub use production_methods::ProductionMethodsProvider;
pub use saves::SavesProvider;
pub use state_qualifications::StateQualificationsProvider;
pub use states::StatesProvider;

/// Fact tables exposed as `active.*` / `latest.*` / unqualified after bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactTable {
    States,
    Goods,
    GoodsByState,
    Buildings,
    BuildingTypes,
    ProductionMethods,
    Pops,
    StateQualifications,
    Countries,
    Constructions,
}

/// All campaign fact tables registered for a binding.
pub const FACT_TABLES: &[FactTable] = &[
    FactTable::States,
    FactTable::Goods,
    FactTable::GoodsByState,
    FactTable::Buildings,
    FactTable::BuildingTypes,
    FactTable::ProductionMethods,
    FactTable::Pops,
    FactTable::StateQualifications,
    FactTable::Countries,
    FactTable::Constructions,
];

impl FactTable {
    /// Unqualified SQL table name (`states`, `goods`, …).
    pub fn name(self) -> &'static str {
        match self {
            Self::States => "states",
            Self::Goods => "goods",
            Self::GoodsByState => "goods_by_state",
            Self::Buildings => "buildings",
            Self::BuildingTypes => "building_types",
            Self::ProductionMethods => "production_methods",
            Self::Pops => "pops",
            Self::StateQualifications => "state_qualifications",
            Self::Countries => "countries",
            Self::Constructions => "constructions",
        }
    }

    /// Full-save twin name (`world_states`, …), when short names are player-scoped.
    pub fn world_name(self) -> Option<&'static str> {
        match self {
            Self::States => Some("world_states"),
            Self::GoodsByState => Some("world_goods_by_state"),
            Self::Buildings => Some("world_buildings"),
            Self::Pops => Some("world_pops"),
            Self::StateQualifications => Some("world_state_qualifications"),
            Self::Countries => Some("world_countries"),
            Self::Constructions => Some("world_constructions"),
            Self::Goods | Self::BuildingTypes | Self::ProductionMethods => None,
        }
    }

    /// Whether short names filter to the played country / its states.
    pub fn is_player_scoped(self) -> bool {
        self.world_name().is_some()
    }

    /// Arrow schema for this fact table (`docs/sql.md`).
    pub fn schema(self) -> datafusion::arrow::datatypes::SchemaRef {
        match self {
            Self::States => states_schema(),
            Self::Goods => goods_schema(),
            Self::GoodsByState => goods_by_state_schema(),
            Self::Buildings => buildings_schema(),
            Self::BuildingTypes => building_types_schema(),
            Self::ProductionMethods => production_methods_schema(),
            Self::Pops => pops_schema(),
            Self::StateQualifications => state_qualifications_schema(),
            Self::Countries => countries_schema(),
            Self::Constructions => constructions_schema(),
        }
    }
}

/// Concrete provider for one fact table over an in-memory binding.
pub fn provider_for(
    table: FactTable,
    binding: Arc<SessionBinding>,
    scope: TableScope,
) -> Arc<dyn TableProvider> {
    let scope = if table.is_player_scoped() {
        scope
    } else {
        TableScope::World
    };
    match table {
        FactTable::States => Arc::new(StatesProvider::new(binding, scope)),
        FactTable::Goods => Arc::new(GoodsProvider::new(binding)),
        FactTable::GoodsByState => Arc::new(GoodsByStateProvider::new(binding, scope)),
        FactTable::Buildings => Arc::new(BuildingsProvider::new(binding, scope)),
        FactTable::BuildingTypes => Arc::new(BuildingTypesProvider::new(binding)),
        FactTable::ProductionMethods => Arc::new(ProductionMethodsProvider::new(binding)),
        FactTable::Pops => Arc::new(PopsProvider::new(binding, scope)),
        FactTable::StateQualifications => {
            Arc::new(StateQualificationsProvider::new(binding, scope))
        }
        FactTable::Countries => Arc::new(CountriesProvider::new(binding, scope)),
        FactTable::Constructions => Arc::new(ConstructionsProvider::new(binding, scope)),
    }
}

const DEFAULT_CATALOG: &str = "datafusion";

/// Ensure `active` / `latest` schemas exist on the default catalog.
///
/// # Errors
///
/// [`SqlError::Internal`] if the default DataFusion catalog is missing;
/// schema registration failures as [`SqlError::DataFusion`].
pub fn ensure_session_schemas(ctx: &SessionContext) -> Result<(), SqlError> {
    let catalog = ctx
        .catalog(DEFAULT_CATALOG)
        .ok_or_else(|| SqlError::internal(format!("missing default catalog {DEFAULT_CATALOG}")))?;
    if catalog.schema("active").is_none() {
        catalog.register_schema("active", Arc::new(MemorySchemaProvider::new()))?;
    }
    if catalog.schema("latest").is_none() {
        catalog.register_schema("latest", Arc::new(MemorySchemaProvider::new()))?;
    }
    Ok(())
}

fn partial(schema: &str, table: &str) -> TableReference {
    TableReference::partial(schema, table)
}

fn register_fact_names(
    ctx: &SessionContext,
    table: FactTable,
    short: Arc<dyn TableProvider>,
    world: Option<Arc<dyn TableProvider>>,
) -> Result<(), SqlError> {
    replace_table(ctx, table.name(), Arc::clone(&short))?;
    replace_table(ctx, partial("active", table.name()), short)?;
    if let (Some(world_name), Some(world)) = (table.world_name(), world) {
        replace_table(ctx, world_name, Arc::clone(&world))?;
        replace_table(ctx, partial("active", world_name), world)?;
    }
    Ok(())
}

/// Register unqualified + `active.*` fact tables for the bound snapshot.
///
/// Called from `bind` and again after `use_save` so both namespaces track the
/// same session (default search path ≡ `active`). Short names are player-scoped
/// where applicable; `world_*` is the full save.
///
/// # Errors
///
/// Schema or table registration failures as [`SqlError`].
pub async fn register_all(
    ctx: &SessionContext,
    binding: Arc<SessionBinding>,
) -> Result<(), SqlError> {
    ensure_session_schemas(ctx)?;
    for table in FACT_TABLES {
        let short = provider_for(*table, Arc::clone(&binding), TableScope::Player);
        let world = table
            .world_name()
            .map(|_| provider_for(*table, Arc::clone(&binding), TableScope::World));
        register_fact_names(ctx, *table, short, world)?;
    }
    Ok(())
}

/// Register `saves`, unbound `active.*` / unqualified facts, and lazy `latest.*`.
///
/// Until `use_save`, scanning `active.*` yields [`crate::SqlError::Unbound`].
/// `latest.*` may load independently without installing the active session.
///
/// # Errors
///
/// Schema or table registration failures as [`SqlError`].
pub(crate) async fn register_catalog_host(
    ctx: &SessionContext,
    host: Arc<HostState>,
) -> Result<(), SqlError> {
    ensure_session_schemas(ctx)?;
    replace_table(
        ctx,
        "saves",
        Arc::new(SavesProvider::new(Arc::clone(&host))),
    )?;

    for table in FACT_TABLES {
        let unbound_short = Arc::new(UnboundFactProvider::new(*table));
        replace_table(
            ctx,
            partial("active", table.name()),
            Arc::clone(&unbound_short) as _,
        )?;
        // Unqualified names resolve to active until rebound by use_save/bind.
        replace_table(ctx, table.name(), Arc::clone(&unbound_short) as _)?;

        let latest_short = Arc::new(LatestFactProvider::new(
            Arc::clone(&host),
            *table,
            TableScope::Player,
        ));
        replace_table(ctx, partial("latest", table.name()), latest_short)?;

        if let Some(world_name) = table.world_name() {
            let unbound_world = Arc::new(UnboundFactProvider::new(*table));
            replace_table(
                ctx,
                partial("active", world_name),
                Arc::clone(&unbound_world) as _,
            )?;
            replace_table(ctx, world_name, unbound_world)?;

            let latest_world = Arc::new(LatestFactProvider::new(
                Arc::clone(&host),
                *table,
                TableScope::World,
            ));
            replace_table(ctx, partial("latest", world_name), latest_world)?;
        }
    }
    Ok(())
}

fn replace_table(
    ctx: &SessionContext,
    table_ref: impl Into<TableReference>,
    provider: Arc<dyn TableProvider>,
) -> Result<(), SqlError> {
    let table_ref = table_ref.into();
    let _ = ctx.deregister_table(table_ref.clone());
    ctx.register_table(table_ref, provider)?;
    Ok(())
}
