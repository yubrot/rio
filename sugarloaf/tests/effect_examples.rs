// Validates the external effect examples shipped under `misc/effects`.
// Scanned at runtime so removing the examples never breaks the build.

use sugarloaf::effects::{validate, EffectSource};

#[test]
fn bundled_effect_examples_validate() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../misc/effects");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut failures = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "wgsl") {
            continue;
        }
        let source = EffectSource {
            name: path.display().to_string(),
            source: std::fs::read_to_string(&path).unwrap(),
        };
        if let Err(err) = validate(&source) {
            failures.push(err);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
