//! Fact-table [`TableProvider`]s for a bound [`SessionBinding`].

mod arrays;
mod building_types;
mod buildings;
mod countries;
mod goods;
mod goods_by_state;
mod pops;
mod production_methods;
mod pushdown;
mod state_qualifications;
mod states;

use std::sync::Arc;

use datafusion::prelude::SessionContext;

use crate::binding::SessionBinding;
use crate::SqlError;

pub use building_types::BuildingTypesProvider;
pub use buildings::BuildingsProvider;
pub use countries::CountriesProvider;
pub use goods::GoodsProvider;
pub use goods_by_state::GoodsByStateProvider;
pub use pops::PopsProvider;
pub use production_methods::ProductionMethodsProvider;
pub use state_qualifications::StateQualificationsProvider;
pub use states::StatesProvider;

/// Register unqualified fact tables for the active in-memory binding.
pub async fn register_all(
    ctx: &SessionContext,
    binding: Arc<SessionBinding>,
) -> Result<(), SqlError> {
    ctx.register_table(
        "states",
        Arc::new(StatesProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table("goods", Arc::new(GoodsProvider::new(Arc::clone(&binding))))?;
    ctx.register_table(
        "goods_by_state",
        Arc::new(GoodsByStateProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table(
        "buildings",
        Arc::new(BuildingsProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table(
        "building_types",
        Arc::new(BuildingTypesProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table(
        "production_methods",
        Arc::new(ProductionMethodsProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table("pops", Arc::new(PopsProvider::new(Arc::clone(&binding))))?;
    ctx.register_table(
        "state_qualifications",
        Arc::new(StateQualificationsProvider::new(Arc::clone(&binding))),
    )?;
    ctx.register_table("countries", Arc::new(CountriesProvider::new(binding)))?;
    Ok(())
}
