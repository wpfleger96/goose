use std::collections::HashSet;
use crate::agents::extension::ExtensionConfig;

/// Represents a secret requirement discovered from a recipe extension
#[derive(Debug, Clone, PartialEq)]
pub struct SecretRequirement {
    /// The environment variable name (e.g., "GITHUB_TOKEN")
    pub key: String,
    /// The name of the extension that requires this secret
    pub extension_name: String,
}

impl SecretRequirement {
    pub fn new(extension_name: String, key: String) -> Self {
        Self {
            key,
            extension_name,
        }
    }

    /// Returns a human-readable description of what this secret is for
    pub fn description(&self) -> String {
        format!("Required by {} extension", self.extension_name)
    }
}

/// Extract secrets from a list of extensions
pub fn extract_secrets_from_extensions(
    extensions: &[ExtensionConfig],
    seen_keys: &mut HashSet<String>,
) -> Vec<SecretRequirement> {
    let mut secrets = Vec::new();

    for ext in extensions {
        let (extension_name, env_keys) = match ext {
            ExtensionConfig::Sse { name, env_keys, .. } => (name, env_keys),
            ExtensionConfig::Stdio { name, env_keys, .. } => (name, env_keys),
            ExtensionConfig::StreamableHttp { name, env_keys, .. } => (name, env_keys),
            ExtensionConfig::Builtin { name, .. } => (name, &Vec::new()),
            ExtensionConfig::Platform { name, .. } => (name, &Vec::new()),
            ExtensionConfig::Frontend { name, .. } => (name, &Vec::new()),
            ExtensionConfig::InlinePython { name, .. } => (name, &Vec::new()),
        };

        for key in env_keys {
            if seen_keys.insert(key.clone()) {
                let secret_req = SecretRequirement::new(extension_name.clone(), key.clone());
                secrets.push(secret_req);
            }
        }
    }

    secrets
}
