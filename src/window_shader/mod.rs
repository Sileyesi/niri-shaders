//! Shaders that draw whole windows: the shader that applies to a window, and the built-in presets.
//!
//! The rendering side is in [`crate::render_helpers::window_shader`].

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use niri_config::window_shader::PRESET_PREFIX;
use niri_config::{ShaderInput, ShaderSource, WindowShader};
use niri_ipc::{Response, WindowShaderRequest};
use smithay::backend::renderer::gles::GlesRenderer;

use crate::layout::LayoutElement as _;
use crate::niri::State;
use crate::render_helpers::window_shader::compile;
use crate::utils::with_toplevel_role;
use crate::window::Mapped;

mod presets;

pub use presets::{find_preset, Preset, PRESETS};

/// The shader that a window is drawn with, with its code.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedShader {
    /// Name for the logs and IPC: the path, `preset:name` or `inline`.
    pub name: String,
    /// The code, or why there isn't any.
    pub code: Result<Rc<str>, String>,
    /// Values of the shader's own uniforms: the preset defaults, then the config.
    pub uniforms: Rc<[(String, Vec<f32>)]>,
    /// Uniforms that follow the time, with their rate.
    pub clock_uniforms: Rc<[(String, f32)]>,
    /// What the ShojiWM effect formats get as `tex`.
    pub input: ShaderInput,
    /// How far around the tile the shader can draw.
    pub margin: f64,
    /// Whether to turn on blur behind the window unless the rules say otherwise.
    pub wants_blur: bool,
}

impl ResolvedShader {
    pub fn new(shader: &WindowShader) -> Self {
        let mut uniforms = Vec::new();
        let mut margin = 0.;
        let mut wants_blur = false;

        let code = match &shader.source {
            ShaderSource::Preset(name) => match find_preset(name) {
                Some(preset) => {
                    uniforms.extend(
                        preset
                            .uniforms
                            .iter()
                            .map(|(name, values)| (String::from(*name), values.to_vec())),
                    );
                    margin = preset.margin;
                    wants_blur = preset.wants_blur;
                    Ok(Rc::from(preset.code))
                }
                None => Err(format!(
                    "there's no preset called {name:?}, the presets are: {}",
                    preset_names().join(", ")
                )),
            },
            ShaderSource::File { code, .. } => code.as_deref().map(Rc::from).map_err(Clone::clone),
            ShaderSource::Inline(code) => Ok(Rc::from(code.as_str())),
        };

        for (name, values) in &shader.uniforms {
            uniforms.retain(|(x, _)| x != name);
            uniforms.push((name.clone(), values.clone()));
        }
        // A uniform tied to the time replaces a preset default of the same name.
        uniforms.retain(|(x, _)| !shader.clock_uniforms.iter().any(|(name, _)| name == x));

        Self {
            name: shader.source.name(),
            code,
            uniforms: Rc::from(uniforms),
            clock_uniforms: Rc::from(shader.clock_uniforms.as_slice()),
            input: shader.input,
            margin: shader.margin.unwrap_or(margin),
            wants_blur,
        }
    }
}

pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|preset| preset.name).collect()
}

/// A shader set on a window at runtime, through IPC or the cycle action.
#[derive(Debug, Clone, PartialEq)]
pub enum ShaderOverride {
    /// Draw the window without a shader, whatever the rules say.
    Cleared,
    Set {
        shader: WindowShader,
        /// Modification time of the shader file when it was read, to reload it when it changes.
        mtime: Option<SystemTime>,
    },
}

impl ShaderOverride {
    /// Makes an override from what the user typed: a preset name, with or without the `preset:`
    /// prefix, a path or shader code.
    ///
    /// Relative paths are taken relative to `cwd`.
    pub fn from_arg(arg: &str, cwd: Option<&Path>) -> Self {
        let source = if find_preset(arg).is_some() {
            ShaderSource::Preset(arg.to_owned())
        } else {
            match ShaderSource::from_arg(arg) {
                ShaderSource::File { path, .. } => {
                    ShaderSource::read_file(resolve_path(&path, cwd))
                }
                source => source,
            }
        };

        Self::from_source(source)
    }

    pub fn preset(name: &str) -> Self {
        Self::from_source(ShaderSource::Preset(name.to_owned()))
    }

    fn from_source(source: ShaderSource) -> Self {
        let mtime = match &source {
            ShaderSource::File { path, .. } => file_mtime(path),
            _ => None,
        };
        Self::Set {
            shader: WindowShader {
                source,
                uniforms: Vec::new(),
                clock_uniforms: Vec::new(),
                margin: None,
                input: ShaderInput::Window,
            },
            mtime,
        }
    }

    /// Reads the shader file again if it changed since it was read, and returns whether it did.
    pub fn reload_if_changed(&mut self) -> bool {
        let Self::Set { shader, mtime } = self else {
            return false;
        };
        let ShaderSource::File { path, .. } = &shader.source else {
            return false;
        };

        let new_mtime = file_mtime(path);
        if new_mtime == *mtime {
            return false;
        }

        *mtime = new_mtime;
        shader.source = ShaderSource::read_file(path.clone());
        true
    }

    /// Name of the preset, if this is one.
    pub fn preset_name(&self) -> Option<&str> {
        match self {
            Self::Set {
                shader:
                    WindowShader {
                        source: ShaderSource::Preset(name),
                        ..
                    },
                ..
            } => Some(name),
            _ => None,
        }
    }
}

fn resolve_path(path: &Path, cwd: Option<&Path>) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        if let Some(home) = std::env::home_dir() {
            return home.join(rest);
        }
    }
    match cwd {
        Some(cwd) if path.is_relative() => cwd.join(path),
        _ => path.to_owned(),
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Returns the preset after `current` in the list, or the first one, or `None` after the last
/// one, to go back to the window rules.
pub fn next_preset(current: Option<&str>) -> Option<&'static Preset> {
    let current = current.map(|name| name.strip_prefix(PRESET_PREFIX).unwrap_or(name));
    let idx = current.and_then(|name| PRESETS.iter().position(|p| p.name == name));
    let start = idx.map_or(0, |idx| idx + 1);
    PRESETS[start..].iter().find(|preset| preset.cycle)
}

/// Finds a window by id, or the focused one.
fn find_window(state: &mut State, id: Option<u64>) -> Option<&mut Mapped> {
    let layout = &mut state.niri.layout;
    match id {
        Some(id) => layout
            .workspaces_mut()
            .find_map(|ws| ws.windows_mut().find(|w| w.id().get() == id)),
        None => layout
            .active_workspace_mut()
            .and_then(|ws| ws.active_window_mut()),
    }
}

/// Compiles the window's shader and returns its info for IPC.
fn shader_info(
    renderer: Option<&mut GlesRenderer>,
    mapped: &Mapped,
) -> Option<niri_ipc::WindowShaderInfo> {
    let resolved = mapped.window_shader()?;
    let (app_id, title) = with_toplevel_role(mapped.toplevel(), |role| {
        (role.app_id.clone(), role.title.clone())
    });

    let mut info = niri_ipc::WindowShaderInfo {
        window_id: mapped.id().get(),
        app_id,
        title,
        shader: resolved.name.clone(),
        is_override: mapped.shader_override().is_some(),
        format: None,
        animated: false,
        error: None,
        warnings: Vec::new(),
    };

    let res = match (&resolved.code, renderer) {
        (Ok(code), Some(renderer)) => compile(renderer, &resolved.name, code, resolved.input),
        (Ok(_), None) => Err(String::from("there's no renderer to compile the shader")),
        (Err(err), _) => Err(err.clone()),
    };
    match res {
        Ok(compiled) => {
            info.format = Some(compiled.format.name().to_owned());
            info.animated = compiled.is_animated(&resolved.uniforms, &resolved.clock_uniforms);
            info.warnings = compiled.uniform_problems(&resolved.uniforms);
        }
        Err(err) => info.error = Some(err),
    }

    Some(info)
}

/// Handles `niri msg window-shader`.
pub fn handle_request(state: &mut State, request: WindowShaderRequest) -> Result<Response, String> {
    let id = match &request {
        WindowShaderRequest::Set { id, .. } | WindowShaderRequest::Clear { id, .. } => *id,
        WindowShaderRequest::List => {
            let mut windows = Vec::new();
            let State { backend, niri } = state;
            backend.with_primary_renderer(|renderer| {
                for (_, mapped) in niri.layout.windows() {
                    windows.extend(shader_info(Some(renderer), mapped));
                }
            });

            let presets = PRESETS.iter().map(|p| p.name.to_owned()).collect();
            return Ok(Response::WindowShaders(niri_ipc::WindowShaders {
                presets,
                windows,
            }));
        }
    };

    let Some(mapped) = find_window(state, id) else {
        return Err(match id {
            Some(id) => format!("there's no window with id {id}"),
            None => String::from("there's no focused window"),
        });
    };

    let shader = match request {
        WindowShaderRequest::Set { shader, .. } => Some(ShaderOverride::from_arg(&shader, None)),
        WindowShaderRequest::Clear { none: true, .. } => Some(ShaderOverride::Cleared),
        WindowShaderRequest::Clear { .. } => None,
        WindowShaderRequest::List => unreachable!(),
    };
    mapped.set_shader_override(shader);
    let id = mapped.id().get();

    // Compile right away to report errors.
    let State { backend, niri } = state;
    let info = backend
        .with_primary_renderer(|renderer| {
            let mapped = niri
                .layout
                .windows()
                .find(|(_, m)| m.id().get() == id)
                .map(|(_, m)| m)?;
            shader_info(Some(renderer), mapped)
        })
        .flatten();

    // FIXME: granular
    niri.queue_redraw_all();

    match info.and_then(|info| info.error) {
        Some(err) => Err(format!(
            "the window is drawn without the shader because of an error:\n{err}"
        )),
        None => Ok(Response::Handled),
    }
}

/// Draws the window with the next preset, then goes back to the window rules after the last one.
pub fn cycle(state: &mut State, id: Option<u64>) {
    let Some(mapped) = find_window(state, id) else {
        return;
    };

    let current = mapped
        .shader_override()
        .and_then(ShaderOverride::preset_name);
    let next = next_preset(current);
    match next {
        Some(preset) => info!("window shader: preset:{}", preset.name),
        None => info!("window shader: back to the window rules"),
    }
    mapped.set_shader_override(next.map(|preset| ShaderOverride::preset(preset.name)));

    // FIXME: granular
    state.niri.queue_redraw_all();
}

/// Reads the shader files set through IPC again if they changed.
pub fn reload_override_files(state: &mut State) {
    let mut changed = false;
    state.niri.layout.with_windows_mut(|mapped, _| {
        changed |= mapped.reload_shader_override();
    });

    if changed {
        // FIXME: granular
        state.niri.queue_redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_helpers::window_shader::Format;

    #[test]
    fn presets_have_unique_names_and_a_format() {
        assert!(!PRESETS.is_empty());
        for (i, preset) in PRESETS.iter().enumerate() {
            assert!(
                PRESETS[..i].iter().all(|p| p.name != preset.name),
                "duplicate preset {}",
                preset.name
            );
            assert!(
                Format::detect(preset.code).is_ok(),
                "preset {} has no entry point",
                preset.name
            );
        }
    }

    #[test]
    fn preset_defaults_are_overridden_by_config() {
        let shader = WindowShader {
            source: ShaderSource::Preset(String::from("island-glass")),
            uniforms: vec![(String::from("refraction_px"), vec![40.])],
            clock_uniforms: vec![],
            margin: Some(3.),
            input: ShaderInput::Window,
        };
        let resolved = ResolvedShader::new(&shader);
        assert!(resolved.code.is_ok());
        assert_eq!(resolved.margin, 3.);

        let refraction: Vec<_> = resolved
            .uniforms
            .iter()
            .filter(|(name, _)| name == "refraction_px")
            .collect();
        assert_eq!(refraction, [&(String::from("refraction_px"), vec![40.])]);
    }

    #[test]
    fn unknown_preset_is_an_error() {
        let shader = WindowShader {
            source: ShaderSource::Preset(String::from("nope")),
            uniforms: vec![],
            clock_uniforms: vec![],
            margin: None,
            input: ShaderInput::Window,
        };
        let err = ResolvedShader::new(&shader).code.unwrap_err();
        assert!(err.contains("island-glass"));
    }

    #[test]
    fn cycles_through_presets() {
        assert_eq!(next_preset(None).unwrap().name, PRESETS[0].name);
        assert_eq!(
            next_preset(Some(PRESETS[0].name)).unwrap().name,
            PRESETS[1].name
        );
        let last = PRESETS.iter().rfind(|p| p.cycle).unwrap().name;
        assert!(next_preset(Some(last)).is_none());
        assert!(next_preset(Some("preset:dissolve")).is_none());

        let mut current = None;
        let mut seen = Vec::new();
        while let Some(preset) = next_preset(current) {
            assert!(preset.cycle);
            seen.push(preset.name);
            current = Some(preset.name);
        }
        assert_eq!(seen.len(), PRESETS.iter().filter(|p| p.cycle).count());
        assert_eq!(
            next_preset(Some("not a preset")).unwrap().name,
            PRESETS[0].name
        );
    }

    #[test]
    fn override_from_arg() {
        let o = ShaderOverride::from_arg("crt", None);
        assert_eq!(o.preset_name(), Some("crt"));
        let o = ShaderOverride::from_arg("preset:crt", None);
        assert_eq!(o.preset_name(), Some("crt"));

        let o = ShaderOverride::from_arg("does/not/exist.frag", Some(Path::new("/tmp")));
        let ShaderOverride::Set { shader, .. } = o else {
            panic!()
        };
        let ShaderSource::File { path, code } = shader.source else {
            panic!()
        };
        assert_eq!(path, Path::new("/tmp/does/not/exist.frag"));
        assert!(code.is_err());
    }
}
