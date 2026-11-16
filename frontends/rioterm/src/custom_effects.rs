//! Loads `[effects.custom]` shaders on a worker thread, watches them for
//! changes, and hands the latest chain to every window.

use crate::event::{EventListener, EventProxy, RioEvent};
use crate::renderer::Renderer;
use rio_backend::config::colors::term::TermColors;
use rio_backend::config::colors::{ColorArray, NamedColor};
use rio_backend::config::effects::{CustomEffects, CustomShader};
use rio_backend::sugarloaf::effects::{
    validate, EffectCursor, EffectPass, EffectSource, EffectTheme, ParamValue,
    ValidatedEffect, API_VERSION, MAX_PASSES,
};
use rio_backend::sugarloaf::Sugarloaf;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Quiet period after a file event before reloading, so multi-step editor
/// saves are read once complete.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// The chain every window should run.
#[derive(Debug, Default)]
pub struct LoadedChain {
    pub passes: Vec<EffectPass>,
    /// One message per shader that failed to load.
    pub errors: Vec<String>,
}

impl LoadedChain {
    fn same_as(&self, other: &LoadedChain) -> bool {
        self.errors == other.errors
            && self.passes.len() == other.passes.len()
            && self
                .passes
                .iter()
                .zip(&other.passes)
                .all(|(a, b)| Arc::ptr_eq(&a.effect, &b.effect) && a.params == b.params)
    }
}

/// Latest loaded chain, read by windows when created or notified.
static LATEST: Mutex<Option<Arc<LoadedChain>>> = Mutex::new(None);

fn latest() -> Arc<LoadedChain> {
    LATEST
        .lock()
        .ok()
        .and_then(|l| l.clone())
        .unwrap_or_default()
}

/// Install the latest chain into `sugarloaf`; returns every error to show,
/// including those of the GPU backend.
pub fn install_latest(sugarloaf: &mut Sugarloaf) -> Vec<String> {
    let chain = latest();
    let mut errors = chain.errors.clone();
    errors.extend(sugarloaf.set_effects(&chain.passes));
    errors
}

enum Message {
    Configure(CustomEffects, PathBuf),
    FilesChanged,
}

/// Handle to the loader thread.
pub struct EffectLoader {
    tx: Sender<Message>,
}

impl EffectLoader {
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the loader thread.
    pub fn spawn(event_proxy: EventProxy) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let watch_tx = tx.clone();
        std::thread::Builder::new()
            .name("effects-loader".into())
            .spawn(move || Worker::new(event_proxy, watch_tx).run(rx))
            .expect("spawn effects loader");
        Self { tx }
    }

    /// Load `custom` (re-reading every file) and watch it if requested.
    pub fn configure(&self, custom: &CustomEffects, config_dir: PathBuf) {
        let _ = self.tx.send(Message::Configure(custom.clone(), config_dir));
    }
}

/// Last successful pass of a configured shader, reused when it fails later.
struct Good {
    source: String,
    params: toml::Table,
    pass: EffectPass,
}

struct Worker {
    event_proxy: EventProxy,
    /// Sender for watcher callbacks.
    tx: Sender<Message>,
    config: Option<(CustomEffects, PathBuf)>,
    watcher: Option<notify::RecommendedWatcher>,
    watched_dirs: Vec<PathBuf>,
    /// File names whose events trigger a reload.
    watched_names: Arc<Mutex<HashSet<std::ffi::OsString>>>,
    /// Keyed by position and path so a file used twice keeps both.
    good: FxHashMap<(usize, PathBuf), Good>,
}

impl Worker {
    fn new(event_proxy: EventProxy, tx: Sender<Message>) -> Self {
        Self {
            event_proxy,
            tx,
            config: None,
            watcher: None,
            watched_dirs: Vec::new(),
            watched_names: Arc::default(),
            good: FxHashMap::default(),
        }
    }

    fn run(mut self, rx: Receiver<Message>) {
        while let Ok(message) = rx.recv() {
            let mut debounce = matches!(message, Message::FilesChanged);
            let mut configured = !debounce;
            self.handle(message);
            // Coalesce bursts; configuration changes apply without delay.
            loop {
                let next = if debounce {
                    rx.recv_timeout(DEBOUNCE)
                } else {
                    rx.try_recv().map_err(|e| match e {
                        TryRecvError::Empty => RecvTimeoutError::Timeout,
                        TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                    })
                };
                match next {
                    Ok(message) => {
                        let changed = matches!(message, Message::FilesChanged);
                        debounce |= changed;
                        configured |= !changed;
                        self.handle(message);
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            self.reload(configured);
        }
    }

    fn handle(&mut self, message: Message) {
        if let Message::Configure(custom, dir) = message {
            self.config = Some((custom, dir));
            self.update_watches();
        }
    }

    /// Load the configured chain and publish it. File events that change
    /// nothing are dropped; a configuration reload always publishes, since
    /// it clears the windows' error toasts.
    fn reload(&mut self, configured: bool) {
        let Some((custom, dir)) = &self.config else {
            return;
        };
        let mut chain = LoadedChain::default();
        let mut used = HashSet::new();
        if custom.is_active() {
            if custom.shaders.len() > MAX_PASSES {
                chain.errors.push(format!(
                    "at most {MAX_PASSES} shaders are supported; ignoring the rest"
                ));
            }
            for (index, shader) in custom.shaders.iter().take(MAX_PASSES).enumerate() {
                let path = resolve(dir, &shader.path);
                let key = (index, path);
                match load(shader, &key.1, self.good.get(&key)) {
                    Ok(good) => {
                        chain.passes.push(good.pass.clone());
                        self.good.insert(key.clone(), good);
                    }
                    Err(error) => match self.good.get(&key) {
                        Some(good) => {
                            chain.passes.push(good.pass.clone());
                            chain.errors.push(format!(
                                "{error}\n(keeping the last working version)"
                            ));
                        }
                        None => chain.errors.push(error),
                    },
                }
                used.insert(key);
            }
        }
        // Removed or disabled shaders release their previous versions.
        self.good.retain(|key, _| used.contains(key));

        let Ok(mut latest) = LATEST.lock() else {
            return;
        };
        if !configured && latest.as_deref().is_some_and(|l| l.same_as(&chain)) {
            return;
        }
        for error in &chain.errors {
            tracing::error!("[effects.custom] {error}");
        }
        *latest = Some(Arc::new(chain));
        drop(latest);
        EventListener::send_event(
            &self.event_proxy,
            RioEvent::CustomEffectsChanged,
            rio_backend::event::WindowId::from(0),
        );
    }

    /// Watch the parent directories of configured files, so atomic rename
    /// saves and re-creation are seen.
    fn update_watches(&mut self) {
        use notify::Watcher;
        let (dirs, names) = match &self.config {
            Some((custom, dir)) if custom.is_active() && custom.hot_reload => {
                let paths: Vec<_> = custom
                    .shaders
                    .iter()
                    .map(|s| resolve(dir, &s.path))
                    .collect();
                let mut dirs: Vec<PathBuf> = paths
                    .iter()
                    .filter_map(|p| p.parent().map(Path::to_path_buf))
                    .collect();
                dirs.sort();
                dirs.dedup();
                let names = paths.iter().filter_map(|p| p.file_name()).map(Into::into);
                (dirs, names.collect())
            }
            _ => (Vec::new(), HashSet::new()),
        };
        if let Ok(mut watched) = self.watched_names.lock() {
            *watched = names;
        }
        if dirs == self.watched_dirs {
            return;
        }
        self.watcher = None;
        self.watched_dirs = Vec::new();
        if dirs.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let names = self.watched_names.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else {
                return;
            };
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            // Matching by file name may reload needlessly, never too little.
            let relevant = names.lock().is_ok_and(|names| {
                event
                    .paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|n| names.contains(n)))
            });
            if relevant {
                let _ = tx.send(Message::FilesChanged);
            }
        };
        let mut watcher = match notify::recommended_watcher(handler) {
            Ok(w) => w,
            Err(err) => {
                tracing::error!("[effects.custom] unable to watch shaders: {err}");
                return;
            }
        };
        for dir in &dirs {
            if let Err(err) = watcher.watch(dir, notify::RecursiveMode::NonRecursive) {
                tracing::error!(
                    "[effects.custom] unable to watch {}: {err}",
                    dir.display()
                );
            }
        }
        self.watcher = Some(watcher);
        self.watched_dirs = dirs;
    }
}

/// Resolve `path` against the config directory unless it is absolute.
fn resolve(config_dir: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Joining component-wise keeps the platform separator throughout.
        path.components()
            .fold(config_dir.to_path_buf(), |acc, c| acc.join(c))
    }
}

/// Load one configured shader, reusing `previous` where its inputs are
/// unchanged.
fn load(
    shader: &CustomShader,
    path: &Path,
    previous: Option<&Good>,
) -> Result<Good, String> {
    let name = path.display().to_string();
    // Unspecified targets the latest ABI.
    if let Some(v) = shader.api_version.filter(|&v| v != API_VERSION) {
        return Err(format!(
            "{name}:\napi-version {v} is not supported (expected {API_VERSION})"
        ));
    }
    let source =
        std::fs::read_to_string(path).map_err(|err| format!("{name}:\n{err}"))?;
    let effect: Arc<ValidatedEffect> = match previous {
        Some(good) if good.source == source => {
            if good.params == shader.params {
                return Ok(Good {
                    source,
                    params: good.params.clone(),
                    pass: good.pass.clone(),
                });
            }
            good.pass.effect.clone()
        }
        _ => Arc::new(validate(&EffectSource {
            name: name.clone(),
            source: source.clone(),
        })?),
    };
    let values = param_values(&shader.params).map_err(|e| format!("{name}:\n{e}"))?;
    let params = effect
        .pack_params(&values)
        .map_err(|e| format!("{name}:\n{e}"))?;
    Ok(Good {
        source,
        params: shader.params.clone(),
        pass: EffectPass { effect, params },
    })
}

/// Convert configured TOML values; the shader defines their names.
fn param_values(table: &toml::Table) -> Result<Vec<(String, ParamValue)>, String> {
    let number = |v: &toml::Value| match v {
        toml::Value::Float(f) => Some(*f),
        toml::Value::Integer(i) => Some(*i as f64),
        _ => None,
    };
    table
        .iter()
        .map(|(key, value)| {
            let parsed = match value {
                toml::Value::Array(items) => items
                    .iter()
                    .map(number)
                    .collect::<Option<Vec<_>>>()
                    .map(ParamValue::Vector),
                v => number(v).map(ParamValue::Number),
            };
            parsed.map(|v| (key.clone(), v)).ok_or_else(|| {
                format!("parameter `{key}` must be a number or an array of numbers")
            })
        })
        .collect()
}

/// Theme colors of a panel, including its terminal's OSC overrides.
pub fn panel_theme(
    renderer: &Renderer,
    term_colors: &TermColors,
    cursor: ColorArray,
) -> EffectTheme {
    let color = |i: usize| renderer.color(i, term_colors);
    let mut theme = EffectTheme {
        foreground: color(NamedColor::Foreground as usize),
        background: color(NamedColor::Background as usize),
        cursor,
        selection_foreground: renderer.named_colors.selection_foreground,
        selection_background: renderer.named_colors.selection_background,
        ..Default::default()
    };
    for (i, c) in theme.palette.iter_mut().enumerate() {
        *c = color(i);
    }
    theme
}

/// Displayed cursor of a panel, in panel-local pixels.
#[allow(clippy::too_many_arguments)]
pub fn panel_cursor(
    col: u16,
    row: u16,
    cell: [f32; 2],
    shape: rio_backend::ansi::CursorShape,
    visible: bool,
    blink_on: bool,
    preedit: bool,
    color: ColorArray,
) -> EffectCursor {
    use rio_backend::ansi::CursorShape;
    // IME composition always shows a block.
    let shape = match (preedit, shape) {
        (true, _) | (_, CursorShape::Block) => 0,
        (_, CursorShape::Underline) => 1,
        (_, CursorShape::Beam) => 2,
        (_, CursorShape::Hidden) => 3,
    };
    EffectCursor {
        rect: [col as f32 * cell[0], row as f32 * cell[1], cell[0], cell[1]],
        color,
        shape,
        visible: visible && shape != 3,
        blink_on,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAMS_SHADER: &str = r#"
struct EffectParams { strength: f32 }
@group(1) @binding(0) var<uniform> params: EffectParams;
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    return textureLoad(rio_input, vec2<i32>(pos), 0) * params.strength;
}
"#;

    fn shader(path: &str, api_version: Option<u32>, params: &str) -> CustomShader {
        CustomShader {
            path: path.into(),
            api_version,
            params: toml::from_str(params).unwrap(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("rio-effects-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn relative_paths_resolve_against_config_dir() {
        let dir = Path::new("/cfg");
        assert_eq!(resolve(dir, "effects/a.wgsl"), dir.join("effects/a.wgsl"));
        let abs = std::env::temp_dir().join("x.wgsl");
        assert_eq!(resolve(dir, abs.to_str().unwrap()), abs);
        let nested = resolve(dir, "effects/a.wgsl");
        assert_eq!(nested.parent().unwrap().file_name().unwrap(), "effects");
    }

    #[test]
    fn api_version_and_missing_files_are_reported() {
        let dir = temp_dir("api");
        let path = dir.join("p.wgsl");
        std::fs::write(&path, PARAMS_SHADER).unwrap();
        let ok = load(&shader("p.wgsl", None, "strength = 1"), &path, None);
        let bad = load(
            &shader("p.wgsl", Some(API_VERSION + 1), "strength = 1"),
            &path,
            None,
        );
        let missing = load(&shader("m.wgsl", None, ""), &dir.join("m.wgsl"), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ok.is_ok());
        assert!(bad.err().unwrap().contains("api-version"));
        assert!(missing.is_err());
    }

    #[test]
    fn params_are_packed_and_value_changes_reuse_the_shader() {
        let dir = temp_dir("params");
        let path = dir.join("p.wgsl");
        std::fs::write(&path, PARAMS_SHADER).unwrap();
        let first = load(&shader("p.wgsl", None, "strength = 2"), &path, None).unwrap();
        let second = load(
            &shader("p.wgsl", None, "strength = 0.5"),
            &path,
            Some(&first),
        )
        .unwrap();
        let wrong = load(&shader("p.wgsl", None, "strength = \"x\""), &path, None);
        let unknown = load(
            &shader("p.wgsl", None, "strength = 1\nwidth = 1"),
            &path,
            None,
        );
        std::fs::remove_dir_all(&dir).unwrap();

        let value = |g: &Good| f32::from_ne_bytes(g.pass.params[..4].try_into().unwrap());
        assert_eq!(value(&first), 2.0);
        assert_eq!(value(&second), 0.5);
        assert!(Arc::ptr_eq(&first.pass.effect, &second.pass.effect));
        assert!(wrong.err().unwrap().contains("must be a number"));
        assert!(unknown.err().unwrap().contains("unknown parameter `width`"));
    }
}
