//! Fact-table [`TableProvider`]s plus catalog host registration.
//!
//! After [`register_all`], unqualified names and `active.*` share the bound
//! snapshot. [`register_catalog_host`] leaves those unbound until `use_save`
//! and registers lazy `latest.*` views that load without mutating the session.

mod arrays;
mod building_types;
mod buildings;
mod countries;
mod goods;
mod goods_by_state;
mod latest;
mod pops;
mod production_methods;
mod pushdown;
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
    building_types_schema, buildings_schema, countries_schema, goods_by_state_schema, goods_schema,
    pops_schema, production_methods_schema, state_qualifications_schema, states_schema,
};
use crate::SqlError;

pub use building_types::BuildingTypesProvider;
pub use buildings::BuildingsProvider;
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
];

impl FactTable {
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
        }
    }

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
        }
    }
}

/// Concrete provider for one fact table over an in-memory binding.
pub fn provider_for(table: FactTable, binding: Arc<SessionBinding>) -> Arc<dyn TableProvider> {
    match table {
        FactTable::States => Arc::new(StatesProvider::new(binding)),
        FactTable::Goods => Arc::new(GoodsProvider::new(binding)),
        FactTable::GoodsByState => Arc::new(GoodsByStateProvider::new(binding)),
        FactTable::Buildings => Arc::new(BuildingsProvider::new(binding)),
        FactTable::BuildingTypes => Arc::new(BuildingTypesProvider::new(binding)),
        FactTable::ProductionMethods => Arc::new(ProductionMethodsProvider::new(binding)),
        FactTable::Pops => Arc::new(PopsProvider::new(binding)),
        FactTable::StateQualifications => Arc::new(StateQualificationsProvider::new(binding)),
        FactTable::Countries => Arc::new(CountriesProvider::new(binding)),
    }
}

const DEFAULT_CATALOG: &str = "datafusion";

/// Ensure `active` / `latest` schemas exist on the default catalog.
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

/// Register unqualified + `active.*` fact tables for the bound snapshot.
///
/// Called from `bind` and again after `use_save` so both namespaces track the
/// same session (default search path ≡ `active`).
pub async fn register_all(
    ctx: &SessionContext,
    binding: Arc<SessionBinding>,
) -> Result<(), SqlError> {
    ensure_session_schemas(ctx)?;
    for table in FACT_TABLES {
        let provider = provider_for(*table, Arc::clone(&binding));
        replace_table(ctx, table.name(), Arc::clone(&provider))?;
        replace_table(ctx, partial("active", table.name()), provider)?;
    }
    Ok(())
}

/// Register `saves`, unbound `active.*` / unqualified facts, and lazy `latest.*`.
///
/// Until `use_save`, scanning `active.*` yields [`crate::SqlError::Unbound`].
/// `latest.*` may load independently without installing the active session.
pub async fn register_catalog_host(
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
        let unbound = Arc::new(UnboundFactProvider::new(*table));
        replace_table(
            ctx,
            partial("active", table.name()),
            Arc::clone(&unbound) as _,
        )?;
        // Unqualified names resolve to active until rebound by use_save/bind.
        replace_table(ctx, table.name(), unbound)?;

        let latest = Arc::new(LatestFactProvider::new(Arc::clone(&host), *table));
        replace_table(ctx, partial("latest", table.name()), latest)?;
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
