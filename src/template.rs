use crate::seed::SEED;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub bytes: Vec<u8>,
    #[serde(default = "default_seed")]
    pub seed: bool,
}

fn default_seed() -> bool {
    true
}

/// Load all `*.toml` templates from `dir`, sorted by filename.
/// Skips/warns on bad files. Falls back to hardcoded SEED if dir is missing or empty.
pub fn load_templates(dir: &Path) -> Vec<Template> {
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
            .collect();
        paths.sort();

        for path in &paths {
            match std::fs::read_to_string(path) {
                Ok(contents) => match toml::from_str::<Template>(&contents) {
                    Ok(t) if !t.bytes.is_empty() && t.seed => templates.push(t),
                    Ok(t) if !t.bytes.is_empty() => {}
                    Ok(_) => eprintln!("[template] skipping {}: bytes is empty", path.display()),
                    Err(e) => eprintln!("[template] skipping {}: {e}", path.display()),
                },
                Err(e) => eprintln!("[template] skipping {}: {e}", path.display()),
            }
        }
    }

    if templates.is_empty() {
        templates.push(Template {
            name: "ancestor".to_string(),
            description: "Single length-adaptive primordial replicator".to_string(),
            bytes: SEED.to_vec(),
            seed: true,
        });
    }

    templates
}
