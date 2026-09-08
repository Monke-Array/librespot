mod canonical;
mod error;
mod scalar;

pub use canonical::{canonical_json, require_canonical_json};
pub use error::{Error, Result};
pub use scalar::{JSON_SAFE_INTEGER_MAX, div_round_nearest_away};

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PLAN_SCHEMA_VERSION: &str = "transition-operator-plan/1";
