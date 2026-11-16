use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Effects {
    #[serde(default = "bool::default", rename = "custom-mouse-cursor")]
    pub custom_mouse_cursor: bool,
    #[serde(default = "bool::default", rename = "trail-cursor")]
    pub trail_cursor: bool,
    /// Trail color as a hex string ("#F07178"). Unset means the trail
    /// takes the cursor color.
    #[serde(default, rename = "trail-cursor-color")]
    pub trail_cursor_color: Option<String>,
    /// Trail opacity, 0.0 to 1.0.
    #[serde(default = "default_trail_opacity", rename = "trail-cursor-opacity")]
    pub trail_cursor_opacity: f32,
    /// Decay pair in milliseconds: how long the fastest and the
    /// slowest corner of the trail take to catch up with the cursor.
    /// The gap between the two is what stretches the trail.
    #[serde(default = "default_trail_decay", rename = "trail-cursor-decay")]
    pub trail_cursor_decay: [u32; 2],
    /// Cursor jumps at or under this many cells (on both axes) do not
    /// start a trail. 0 animates every movement.
    #[serde(
        default = "default_trail_start_threshold",
        rename = "trail-cursor-start-threshold"
    )]
    pub trail_cursor_start_threshold: u32,
    /// External WGSL post-processing chain.
    #[serde(default)]
    pub custom: CustomEffects,
}

/// `[effects.custom]`: user-provided WGSL passes applied to each panel.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct CustomEffects {
    #[serde(default)]
    pub enabled: bool,
    /// Reload shader files when they change on disk.
    #[serde(default = "default_hot_reload", rename = "hot-reload")]
    pub hot_reload: bool,
    /// Frame rate cap for continuously running chains; see [`Self::max_fps`].
    #[serde(default = "default_max_fps", rename = "max-fps")]
    pub max_fps: u32,
    /// Passes in execution order.
    #[serde(default)]
    pub shaders: Vec<CustomShader>,
}

impl Default for CustomEffects {
    fn default() -> Self {
        Self {
            enabled: false,
            hot_reload: default_hot_reload(),
            max_fps: default_max_fps(),
            shaders: Vec::new(),
        }
    }
}

/// One `[[effects.custom.shaders]]` entry.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct CustomShader {
    /// WGSL file; relative paths resolve against the config directory.
    pub path: String,
    /// Shader ABI the file targets; unspecified means the latest. Checked
    /// when loading so a mismatch disables only this shader.
    #[serde(default, rename = "api-version")]
    pub api_version: Option<u32>,
    /// Named values for the shader's `EffectParams` struct.
    #[serde(default)]
    pub params: toml::Table,
}

/// Accepted range of `max-fps`.
pub const MAX_FPS_RANGE: std::ops::RangeInclusive<u32> = 1..=240;

impl CustomEffects {
    /// Whether any pass should run.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.shaders.is_empty()
    }

    /// `max-fps` clamped to [`MAX_FPS_RANGE`].
    pub fn max_fps(&self) -> u32 {
        self.max_fps
            .clamp(*MAX_FPS_RANGE.start(), *MAX_FPS_RANGE.end())
    }
}

fn default_hot_reload() -> bool {
    true
}

fn default_max_fps() -> u32 {
    60
}

fn default_trail_opacity() -> f32 {
    1.0
}

fn default_trail_decay() -> [u32; 2] {
    [100, 400]
}

fn default_trail_start_threshold() -> u32 {
    2
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            custom_mouse_cursor: false,
            trail_cursor: false,
            trail_cursor_color: None,
            trail_cursor_opacity: default_trail_opacity(),
            trail_cursor_decay: default_trail_decay(),
            trail_cursor_start_threshold: default_trail_start_threshold(),
            custom: CustomEffects::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_bare_table() {
        let effects: Effects = toml::from_str("").unwrap();
        assert_eq!(effects, Effects::default());
        assert_eq!(effects.trail_cursor_opacity, 1.0);
        assert_eq!(effects.trail_cursor_decay, [100, 400]);
        assert_eq!(effects.trail_cursor_start_threshold, 2);
        assert!(effects.trail_cursor_color.is_none());
    }

    #[test]
    fn full_table_parses() {
        let effects: Effects = toml::from_str(
            r##"
            trail-cursor = true
            trail-cursor-color = "#F07178"
            trail-cursor-opacity = 0.4
            trail-cursor-decay = [50, 250]
            trail-cursor-start-threshold = 0
            "##,
        )
        .unwrap();
        assert!(effects.trail_cursor);
        assert_eq!(effects.trail_cursor_color.as_deref(), Some("#F07178"));
        assert_eq!(effects.trail_cursor_opacity, 0.4);
        assert_eq!(effects.trail_cursor_decay, [50, 250]);
        assert_eq!(effects.trail_cursor_start_threshold, 0);
    }

    #[test]
    fn custom_shaders_parse() {
        let effects: Effects = toml::from_str(
            r##"
            [custom]
            enabled = true

            [[custom.shaders]]
            path = "effects/a.wgsl"
            api-version = 1

            [custom.shaders.params]
            color = [1.0, 0.5, 0.0, 1.0]
            width = 2.0

            [[custom.shaders]]
            path = "/abs/b.wgsl"
            "##,
        )
        .unwrap();
        let custom = &effects.custom;
        assert!(custom.is_active());
        assert_eq!(custom.shaders.len(), 2);
        assert_eq!(custom.shaders[0].api_version, Some(1));
        assert_eq!(custom.shaders[0].params["width"].as_float(), Some(2.0));
        assert_eq!(custom.shaders[1].api_version, None);
        assert!(custom.shaders[1].params.is_empty());
    }

    #[test]
    fn custom_is_inactive_by_default() {
        let effects: Effects = toml::from_str("").unwrap();
        assert!(!effects.custom.is_active());
        assert!(effects.custom.hot_reload);
        assert_eq!(effects.custom.max_fps(), 60);
    }

    #[test]
    fn max_fps_is_clamped() {
        let custom: CustomEffects = toml::from_str("max-fps = 0").unwrap();
        assert_eq!(custom.max_fps(), 1);
        let custom: CustomEffects = toml::from_str("max-fps = 1000").unwrap();
        assert_eq!(custom.max_fps(), 240);
    }
}
