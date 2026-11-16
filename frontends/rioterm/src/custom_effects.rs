//! Loads `[effects.custom]` shader files and installs them into sugarloaf.

use rio_backend::config::effects::CustomEffects;
use rio_backend::sugarloaf::effects::{EffectSource, API_VERSION};
use rio_backend::sugarloaf::Sugarloaf;
use std::path::{Path, PathBuf};

/// Resolve `path` against the config directory unless it is absolute.
fn resolve(config_dir: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

/// Read every configured shader, collecting a message per unusable one.
fn load_sources(
    custom: &CustomEffects,
    config_dir: &Path,
) -> (Vec<EffectSource>, Vec<String>) {
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    if !custom.is_active() {
        return (sources, errors);
    }
    for shader in &custom.shaders {
        let path = resolve(config_dir, &shader.path);
        let name = path.display().to_string();
        match shader.api_version {
            Some(API_VERSION) => {}
            Some(v) => {
                errors.push(format!(
                    "{name}: api-version {v} is not supported (expected {API_VERSION})"
                ));
                continue;
            }
            None => {
                errors.push(format!("{name}: api-version is required"));
                continue;
            }
        }
        if !shader.params.is_empty() {
            tracing::warn!("{name}: shader params are not supported yet; ignoring them");
        }
        match std::fs::read_to_string(&path) {
            Ok(source) => sources.push(EffectSource { name, source }),
            Err(err) => errors.push(format!("{name}: {err}")),
        }
    }
    (sources, errors)
}

/// Install the configured chain into `sugarloaf`, logging every failure.
pub fn apply(sugarloaf: &mut Sugarloaf, custom: &CustomEffects) {
    let (sources, mut errors) =
        load_sources(custom, &rio_backend::config::config_dir_path());
    errors.extend(sugarloaf.set_effects(&sources));
    for error in errors {
        tracing::error!("[effects.custom] {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rio_backend::config::effects::CustomShader;

    fn shader(path: &str, api_version: Option<u32>) -> CustomShader {
        CustomShader {
            path: path.into(),
            api_version,
            params: Default::default(),
        }
    }

    #[test]
    fn relative_paths_resolve_against_config_dir() {
        let dir = Path::new("/cfg");
        assert_eq!(resolve(dir, "effects/a.wgsl"), dir.join("effects/a.wgsl"));
        let abs = std::env::temp_dir().join("x.wgsl");
        assert_eq!(resolve(dir, abs.to_str().unwrap()), abs);
    }

    #[test]
    fn disabled_chain_loads_nothing() {
        let custom = CustomEffects {
            enabled: false,
            shaders: vec![shader("missing.wgsl", Some(API_VERSION))],
        };
        let (sources, errors) = load_sources(&custom, Path::new("/cfg"));
        assert!(sources.is_empty() && errors.is_empty());
    }

    #[test]
    fn bad_entries_report_errors_and_good_ones_load() {
        let dir =
            std::env::temp_dir().join(format!("rio-effects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ok.wgsl"), "// ok").unwrap();
        let custom = CustomEffects {
            enabled: true,
            shaders: vec![
                shader("ok.wgsl", Some(API_VERSION)),
                shader("ok.wgsl", None),
                shader("ok.wgsl", Some(API_VERSION + 1)),
                shader("missing.wgsl", Some(API_VERSION)),
            ],
        };
        let (sources, errors) = load_sources(&custom, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source, "// ok");
        assert_eq!(errors.len(), 3, "{errors:?}");
    }
}
