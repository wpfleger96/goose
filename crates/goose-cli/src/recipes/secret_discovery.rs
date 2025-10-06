// Re-export types from goose core for backward compatibility
//
// The secret discovery logic has been moved to the goose crate and is now
// integrated with ResolvedRecipe to eliminate redundant recipe loading and
// fix template resolution issues with nested sub-recipes.
pub use goose::recipe::secret_discovery::SecretRequirement;
