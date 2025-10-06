use std::collections::{HashMap, HashSet};
use anyhow::Result;
use crate::recipe::Recipe;
use crate::recipe::build_recipe::{build_recipe_from_template, RecipeError};
use crate::recipe::local_recipes::load_local_recipe_file;
use crate::recipe::secret_discovery::{SecretRequirement, extract_secrets_from_extensions};

/// A fully-loaded and resolved recipe tree
///
/// This structure loads the entire recipe tree (including all nested sub-recipes)
/// in a single pass, resolving all template variables during the loading process.
/// This eliminates the need to repeatedly load and resolve recipes during different
/// phases like secret discovery and execution setup.
#[derive(Debug)]
pub struct ResolvedRecipe {
    /// The main recipe with all templates resolved
    pub recipe: Recipe,

    /// Recursively loaded sub-recipes
    pub loaded_sub_recipes: Vec<ResolvedRecipe>,
}

impl ResolvedRecipe {
    /// Recursively load and resolve all recipes and sub-recipes
    ///
    /// This function loads the entire recipe tree in one pass, resolving all template
    /// variables (like `{{ recipe_dir }}`) at each level. It handles circular dependency
    /// detection and parameter merging for sub-recipes.
    ///
    /// # Arguments
    /// * `recipe_path` - Path to the recipe file to load
    /// * `params` - Parameter key-value pairs for template substitution
    /// * `user_prompt_fn` - Optional callback for prompting user for missing parameters
    ///
    /// # Returns
    /// A fully-resolved recipe tree with all sub-recipes loaded
    pub fn load_recursive<F>(
        recipe_path: &str,
        params: Vec<(String, String)>,
        user_prompt_fn: Option<F>,
    ) -> Result<Self, RecipeError>
    where
        F: Fn(&str, &str) -> Result<String, anyhow::Error>,
    {
        let mut visited = HashSet::new();
        Self::load_recursive_internal(recipe_path, params, user_prompt_fn, &mut visited)
    }

    fn load_recursive_internal<F>(
        recipe_path: &str,
        params: Vec<(String, String)>,
        user_prompt_fn: Option<F>,
        visited_paths: &mut HashSet<String>,
    ) -> Result<Self, RecipeError>
    where
        F: Fn(&str, &str) -> Result<String, anyhow::Error>,
    {
        let recipe_file = load_local_recipe_file(recipe_path)
            .map_err(|e| RecipeError::RecipeParsing { source: e })?;

        let canonical_path = recipe_file.file_path.clone();
        let path_str = canonical_path.to_string_lossy().to_string();

        if !visited_paths.insert(path_str.clone()) {
            return Err(RecipeError::RecipeParsing {
                source: anyhow::anyhow!(
                    "Circular dependency detected: {} is already being loaded",
                    recipe_path
                ),
            });
        }

        let recipe = build_recipe_from_template(recipe_file, params.clone(), user_prompt_fn)?;

        let loaded_sub_recipes = if let Some(sub_recipes) = &recipe.sub_recipes {
            let mut loaded = Vec::new();

            for sub_recipe in sub_recipes {
                let sub_params = merge_parameters(&params, &sub_recipe.values);

                match Self::load_recursive_internal(
                    &sub_recipe.path,
                    sub_params,
                    None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
                    visited_paths,
                ) {
                    Ok(sub_resolved) => loaded.push(sub_resolved),
                    Err(e) => {
                        if e.to_string().contains("Circular dependency") {
                            return Err(e);
                        }
                        tracing::warn!(
                            "Skipping sub-recipe '{}': {}",
                            sub_recipe.path,
                            e
                        );
                        continue;
                    }
                }
            }

            loaded
        } else {
            Vec::new()
        };

        visited_paths.remove(&path_str);

        Ok(ResolvedRecipe {
            recipe,
            loaded_sub_recipes,
        })
    }

    /// Get a flattened list of all recipes in the tree
    ///
    /// Returns references to all recipes in the tree, including the main recipe
    /// and all nested sub-recipes, in depth-first order.
    pub fn flatten(&self) -> Vec<&Recipe> {
        let mut result = vec![&self.recipe];
        for sub in &self.loaded_sub_recipes {
            result.extend(sub.flatten());
        }
        result
    }

    /// Discover all secrets required by this recipe and its sub-recipes
    ///
    /// This function traverses the pre-loaded recipe tree and collects all secrets
    /// required by MCP extensions at any level, deduplicating by key name.
    ///
    /// # Returns
    /// A vector of SecretRequirement objects, deduplicated by key name
    pub fn discover_secrets(&self) -> Vec<SecretRequirement> {
        let mut secrets = Vec::new();
        let mut seen_keys = HashSet::new();

        if let Some(extensions) = &self.recipe.extensions {
            secrets.extend(extract_secrets_from_extensions(extensions, &mut seen_keys));
        }

        for sub_resolved in &self.loaded_sub_recipes {
            let sub_secrets = sub_resolved.discover_secrets();
            for secret in sub_secrets {
                if seen_keys.insert(secret.key.clone()) {
                    secrets.push(secret);
                }
            }
        }

        secrets
    }
}

/// Merge parent parameters with sub-recipe specific values
///
/// Sub-recipe values override parent parameters with the same key.
fn merge_parameters(
    parent_params: &[(String, String)],
    sub_recipe_values: &Option<HashMap<String, String>>,
) -> Vec<(String, String)> {
    let mut merged: HashMap<String, String> = parent_params.iter().cloned().collect();

    if let Some(values) = sub_recipe_values {
        for (key, value) in values {
            merged.insert(key.clone(), value.clone());
        }
    }

    merged.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_parameters_no_sub_values() {
        let parent = vec![
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ];
        let sub_values = None;

        let result = merge_parameters(&parent, &sub_values);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&("key1".to_string(), "value1".to_string())));
        assert!(result.contains(&("key2".to_string(), "value2".to_string())));
    }

    #[test]
    fn test_merge_parameters_with_sub_values() {
        let parent = vec![
            ("key1".to_string(), "value1".to_string()),
            ("key2".to_string(), "value2".to_string()),
        ];
        let mut sub_values_map = HashMap::new();
        sub_values_map.insert("key2".to_string(), "overridden".to_string());
        sub_values_map.insert("key3".to_string(), "new_value".to_string());
        let sub_values = Some(sub_values_map);

        let result = merge_parameters(&parent, &sub_values);

        assert_eq!(result.len(), 3);
        assert!(result.contains(&("key1".to_string(), "value1".to_string())));
        assert!(result.contains(&("key2".to_string(), "overridden".to_string())));
        assert!(result.contains(&("key3".to_string(), "new_value".to_string())));
    }

    #[test]
    fn test_load_single_recipe() {
        let recipe_content = r#"
version: "1.0.0"
title: "Test Recipe"
description: "A simple test recipe"
instructions: "Do something"
"#;
        let temp_dir = tempfile::tempdir().unwrap();
        let recipe_path = temp_dir.path().join("test_recipe.yaml");
        std::fs::write(&recipe_path, recipe_content).unwrap();

        let resolved = ResolvedRecipe::load_recursive(
            recipe_path.to_str().unwrap(),
            vec![],
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        )
        .unwrap();

        assert_eq!(resolved.recipe.title, "Test Recipe");
        assert_eq!(resolved.loaded_sub_recipes.len(), 0);
    }

    #[test]
    fn test_load_recipe_with_sub_recipes() {
        let temp_dir = tempfile::tempdir().unwrap();

        let sub_recipe_content = r#"
version: "1.0.0"
title: "Sub Recipe"
description: "A sub recipe"
instructions: "Do sub thing"
"#;
        let sub_recipe_path = temp_dir.path().join("sub_recipe.yaml");
        std::fs::write(&sub_recipe_path, sub_recipe_content).unwrap();

        let main_recipe_content = format!(
            r#"
version: "1.0.0"
title: "Main Recipe"
description: "A main recipe with sub-recipes"
instructions: "Do main thing"
sub_recipes:
  - name: "sub"
    path: "{}"
"#,
            sub_recipe_path.to_str().unwrap()
        );
        let main_recipe_path = temp_dir.path().join("main_recipe.yaml");
        std::fs::write(&main_recipe_path, main_recipe_content).unwrap();

        let resolved = ResolvedRecipe::load_recursive(
            main_recipe_path.to_str().unwrap(),
            vec![],
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        )
        .unwrap();

        assert_eq!(resolved.recipe.title, "Main Recipe");
        assert_eq!(resolved.loaded_sub_recipes.len(), 1);
        assert_eq!(resolved.loaded_sub_recipes[0].recipe.title, "Sub Recipe");
    }

    #[test]
    fn test_circular_dependency_detection() {
        let temp_dir = tempfile::tempdir().unwrap();

        let recipe_a_path = temp_dir.path().join("recipe_a.yaml");
        let recipe_b_path = temp_dir.path().join("recipe_b.yaml");

        let recipe_a_content = format!(
            r#"
version: "1.0.0"
title: "Recipe A"
description: "Recipe A"
instructions: "A"
sub_recipes:
  - name: "b"
    path: "{}"
"#,
            recipe_b_path.to_str().unwrap()
        );
        std::fs::write(&recipe_a_path, recipe_a_content).unwrap();

        let recipe_b_content = format!(
            r#"
version: "1.0.0"
title: "Recipe B"
description: "Recipe B"
instructions: "B"
sub_recipes:
  - name: "a"
    path: "{}"
"#,
            recipe_a_path.to_str().unwrap()
        );
        std::fs::write(&recipe_b_path, recipe_b_content).unwrap();

        let result = ResolvedRecipe::load_recursive(
            recipe_a_path.to_str().unwrap(),
            vec![],
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Circular dependency"));
    }

    #[test]
    fn test_diamond_dependency_handling() {
        let temp_dir = tempfile::tempdir().unwrap();

        let recipe_d_content = r#"
version: "1.0.0"
title: "Recipe D"
description: "Shared dependency"
instructions: "D"
"#;
        let recipe_d_path = temp_dir.path().join("recipe_d.yaml");
        std::fs::write(&recipe_d_path, recipe_d_content).unwrap();

        let recipe_b_content = format!(
            r#"
version: "1.0.0"
title: "Recipe B"
description: "B depends on D"
instructions: "B"
sub_recipes:
  - name: "d"
    path: "{}"
"#,
            recipe_d_path.to_str().unwrap()
        );
        let recipe_b_path = temp_dir.path().join("recipe_b.yaml");
        std::fs::write(&recipe_b_path, recipe_b_content).unwrap();

        let recipe_c_content = format!(
            r#"
version: "1.0.0"
title: "Recipe C"
description: "C depends on D"
instructions: "C"
sub_recipes:
  - name: "d"
    path: "{}"
"#,
            recipe_d_path.to_str().unwrap()
        );
        let recipe_c_path = temp_dir.path().join("recipe_c.yaml");
        std::fs::write(&recipe_c_path, recipe_c_content).unwrap();

        let recipe_a_content = format!(
            r#"
version: "1.0.0"
title: "Recipe A"
description: "A depends on B and C"
instructions: "A"
sub_recipes:
  - name: "b"
    path: "{}"
  - name: "c"
    path: "{}"
"#,
            recipe_b_path.to_str().unwrap(),
            recipe_c_path.to_str().unwrap()
        );
        let recipe_a_path = temp_dir.path().join("recipe_a.yaml");
        std::fs::write(&recipe_a_path, recipe_a_content).unwrap();

        let resolved = ResolvedRecipe::load_recursive(
            recipe_a_path.to_str().unwrap(),
            vec![],
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        )
        .unwrap();

        assert_eq!(resolved.recipe.title, "Recipe A");
        assert_eq!(resolved.loaded_sub_recipes.len(), 2);

        let recipe_b = &resolved.loaded_sub_recipes[0];
        let recipe_c = &resolved.loaded_sub_recipes[1];

        assert_eq!(recipe_b.recipe.title, "Recipe B");
        assert_eq!(recipe_c.recipe.title, "Recipe C");

        assert_eq!(recipe_b.loaded_sub_recipes.len(), 1);
        assert_eq!(recipe_c.loaded_sub_recipes.len(), 1);

        assert_eq!(recipe_b.loaded_sub_recipes[0].recipe.title, "Recipe D");
        assert_eq!(recipe_c.loaded_sub_recipes[0].recipe.title, "Recipe D");
    }

    #[test]
    fn test_flatten() {
        let temp_dir = tempfile::tempdir().unwrap();

        let sub_recipe_content = r#"
version: "1.0.0"
title: "Sub Recipe"
description: "A sub recipe"
instructions: "Sub"
"#;
        let sub_recipe_path = temp_dir.path().join("sub_recipe.yaml");
        std::fs::write(&sub_recipe_path, sub_recipe_content).unwrap();

        let main_recipe_content = format!(
            r#"
version: "1.0.0"
title: "Main Recipe"
description: "Main"
instructions: "Main"
sub_recipes:
  - name: "sub"
    path: "{}"
"#,
            sub_recipe_path.to_str().unwrap()
        );
        let main_recipe_path = temp_dir.path().join("main_recipe.yaml");
        std::fs::write(&main_recipe_path, main_recipe_content).unwrap();

        let resolved = ResolvedRecipe::load_recursive(
            main_recipe_path.to_str().unwrap(),
            vec![],
            None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
        )
        .unwrap();

        let flattened = resolved.flatten();
        assert_eq!(flattened.len(), 2);
        assert_eq!(flattened[0].title, "Main Recipe");
        assert_eq!(flattened[1].title, "Sub Recipe");
    }
}
