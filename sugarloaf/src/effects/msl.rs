//! WGSL-to-MSL translation for the native Metal backend.

use naga::back::msl;

pub(super) fn validator() -> naga::valid::Validator {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
}

fn binding(binding: u32) -> naga::ResourceBinding {
    naga::ResourceBinding { group: 0, binding }
}

fn buffer_target(slot: u8) -> msl::BindTarget {
    msl::BindTarget {
        buffer: Some(slot),
        ..Default::default()
    }
}

fn texture_target(slot: u8) -> msl::BindTarget {
    msl::BindTarget {
        texture: Some(slot),
        ..Default::default()
    }
}

fn sampler_target(slot: u8) -> msl::BindTarget {
    msl::BindTarget {
        sampler: Some(msl::BindSamplerTarget::Resource(slot)),
        ..Default::default()
    }
}

/// Translate `module` to MSL with every entry point bound to `resources`.
/// Returns the source and `(wgsl name, msl name)` per entry point.
pub(super) fn to_msl(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    resources: msl::BindingMap,
) -> Result<(String, Vec<(String, String)>), String> {
    let per_entry_point_map = module
        .entry_points
        .iter()
        .map(|ep| {
            let res = msl::EntryPointResources {
                resources: resources.clone(),
                ..Default::default()
            };
            (ep.name.clone(), res)
        })
        .collect();
    let options = msl::Options {
        lang_version: (2, 1),
        per_entry_point_map,
        fake_missing_bindings: false,
        ..Default::default()
    };
    let (src, translation) =
        msl::write_string(module, info, &options, &msl::PipelineOptions::default())
            .map_err(|e| format!("MSL translation failed: {e}"))?;
    let names = module
        .entry_points
        .iter()
        .zip(translation.entry_point_names)
        .map(|(ep, name)| {
            name.map(|n| (ep.name.clone(), n))
                .map_err(|e| format!("MSL translation of `{}` failed: {e}", ep.name))
        })
        .collect::<Result<_, _>>()?;
    Ok((src, names))
}

/// Metal slots for the effect ABI (`prelude.wgsl`): uniform blocks →
/// buffers in [`super::UNIFORM_BINDINGS`] order, then the parameter
/// struct; input/original → textures 0/1, sampler → sampler 0.
pub(super) fn effect_resources() -> msl::BindingMap {
    let mut resources = msl::BindingMap::default();
    for (slot, b) in super::UNIFORM_BINDINGS.iter().enumerate() {
        resources.insert(binding(*b), buffer_target(slot as u8));
    }
    resources.insert(binding(1), texture_target(0));
    resources.insert(binding(2), texture_target(1));
    resources.insert(binding(3), sampler_target(0));
    resources.insert(
        naga::ResourceBinding {
            group: 1,
            binding: 0,
        },
        buffer_target(super::PARAMS_SLOT as u8),
    );
    resources
}

/// Metal slots for `host.wgsl`: source → texture 0, uniform → buffer 0.
pub(super) fn host_resources() -> msl::BindingMap {
    let mut resources = msl::BindingMap::default();
    resources.insert(binding(0), texture_target(0));
    resources.insert(binding(1), buffer_target(0));
    resources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_examples_translate_to_msl() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../misc/effects");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "wgsl") {
                continue;
            }
            let effect = super::super::validate(&super::super::EffectSource {
                name: path.display().to_string(),
                source: std::fs::read_to_string(&path).unwrap(),
            })
            .unwrap();
            to_msl(&effect.module, &effect.info, effect_resources())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }

    #[test]
    fn host_and_effect_bindings_translate_to_msl() {
        let host = naga::front::wgsl::parse_str(include_str!("host.wgsl")).unwrap();
        let info = validator().validate(&host).unwrap();
        let (_, names) = to_msl(&host, &info, host_resources()).unwrap();
        assert_eq!(names.len(), 3);

        let effect = super::super::validate(&super::super::EffectSource {
            name: "t.wgsl".into(),
            source: "fn effect_main(pos: vec2<f32>) -> vec4<f32> { \
                     return textureSampleLevel(rio_input, rio_sampler, pos / rio_frame.resolution, 0.0) \
                     + textureLoad(rio_original, vec2<i32>(pos), 0); }"
                .into(),
        })
        .unwrap();
        let (src, _) = to_msl(&effect.module, &effect.info, effect_resources()).unwrap();
        assert!(src.contains("[[texture(1)]]"), "{src}");
    }
}
