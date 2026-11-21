//! The `shader` window rule property, which draws the window through a fragment shader.

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use knuffel::errors::DecodeError;
use knuffel::traits::ErrorSpan;

use crate::utils::expand_home_path;
use crate::{FloatOrInt, Includes};

/// Prefix of shader names that refer to the shaders built into niri.
pub const PRESET_PREFIX: &str = "preset:";

#[derive(Debug, Clone, PartialEq)]
pub struct WindowShader {
    pub source: ShaderSource,
    /// Values of the shader's own uniforms, in the order they appear in the config.
    ///
    /// One to four floats each, which become a `float`, `vec2`, `vec3` or `vec4`.
    pub uniforms: Vec<(String, Vec<f32>)>,
    /// Uniforms that follow the time, `uniform "name" time-scale=1.0`, with their rate.
    pub clock_uniforms: Vec<(String, f32)>,
    /// How far around the tile the shader can draw, in logical pixels.
    ///
    /// `None` means the default of the preset, or 0 for other shaders.
    pub margin: Option<f64>,
    /// What the ShojiWM effect formats get as `tex`.
    pub input: ShaderInput,
}

/// What a shader gets as its input texture, like the `input` of a ShojiWM effect.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ShaderInput {
    /// The window, with its border and decorations. The shader draws the window.
    #[default]
    Window,
    /// What's behind the window, blurred when the window has blur. The shader draws behind the
    /// window, which goes on top of it.
    Backdrop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShaderSource {
    /// One of the shaders built into niri, by name without the `preset:` prefix.
    Preset(String),
    /// Shader read from a file, or the error from reading it.
    File {
        path: PathBuf,
        code: Result<String, String>,
    },
    /// Shader code written in the config.
    Inline(String),
}

impl ShaderSource {
    /// Interprets the argument of `shader`: a preset name, a file path or shader code.
    ///
    /// Relative paths are left as is, see [`ShaderSource::read_file()`] for reading them.
    pub fn from_arg(arg: &str) -> Self {
        if let Some(name) = arg.strip_prefix(PRESET_PREFIX) {
            Self::Preset(name.trim().to_owned())
        } else if is_code(arg) {
            Self::Inline(arg.to_owned())
        } else {
            Self::read_file(PathBuf::from(arg))
        }
    }

    /// Reads a shader file.
    ///
    /// A missing file is not an error for the config, the window is drawn without the shader.
    pub fn read_file(path: PathBuf) -> Self {
        let code =
            fs::read_to_string(&path).map_err(|err| format!("error reading {path:?}: {err}"));
        Self::File { path, code }
    }

    /// Short human-readable name of the shader, for the logs and IPC.
    pub fn name(&self) -> String {
        match self {
            ShaderSource::Preset(name) => format!("{PRESET_PREFIX}{name}"),
            ShaderSource::File { path, .. } => path.to_string_lossy().into_owned(),
            ShaderSource::Inline(_) => String::from("inline"),
        }
    }
}

/// Whether the argument of `shader` is shader code rather than a path.
fn is_code(arg: &str) -> bool {
    arg.contains('\n') || arg.contains(';') || arg.contains('{')
}

impl<S: ErrorSpan> knuffel::Decode<S> for WindowShader {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        let mut source = None;
        let mut path_arg = None;

        let mut args = node.arguments.iter();
        if let Some(val) = args.next() {
            let arg: String = knuffel::traits::DecodeScalar::decode(val, ctx)?;
            if arg.starts_with(PRESET_PREFIX) || is_code(&arg) {
                source = Some(ShaderSource::from_arg(&arg));
            } else {
                path_arg = Some((PathBuf::from(arg), val));
            }
        }
        for val in args {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "unexpected argument",
            ));
        }

        for (name, val) in &node.properties {
            if &***name == "path" {
                let path: String = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                path_arg = Some((PathBuf::from(path), val));
            } else {
                ctx.emit_error(DecodeError::unexpected(
                    name,
                    "property",
                    format!("unexpected property `{}`", name.escape_default()),
                ));
            }
        }

        if let Some((path, val)) = path_arg {
            if source.is_some() {
                ctx.emit_error(DecodeError::unexpected(
                    &val.literal,
                    "property",
                    "the shader is already set by the argument",
                ));
            } else {
                let path = expand_home_path(path, node, ctx)?;

                // Store the path even when it fails to read, so that it's watched and the shader
                // appears as soon as the file does.
                let includes = ctx.get::<Rc<RefCell<Includes>>>().unwrap();
                includes.borrow_mut().0.push(path.clone());

                source = Some(ShaderSource::read_file(path));
            }
        }

        let source = source.ok_or_else(|| {
            DecodeError::missing(node, "a shader path, preset name or code is required")
        })?;

        let mut uniforms: Vec<(String, Vec<f32>)> = Vec::new();
        let mut clock_uniforms: Vec<(String, f32)> = Vec::new();
        let mut margin = None;
        let mut input = ShaderInput::default();

        for child in node.children() {
            match &**child.node_name {
                "uniform" => parse_uniform(child, ctx, &mut uniforms, &mut clock_uniforms)?,
                "input" => {
                    let value: String = crate::utils::parse_arg_node("input", child, ctx)?;
                    match &*value {
                        "window" => input = ShaderInput::Window,
                        "backdrop" => input = ShaderInput::Backdrop,
                        _ => ctx.emit_error(DecodeError::conversion(
                            &child.arguments[0].literal,
                            "input must be \"window\" or \"backdrop\"",
                        )),
                    }
                }
                "margin" => {
                    let value: FloatOrInt<0, 1000> =
                        crate::utils::parse_arg_node("margin", child, ctx)?;
                    margin = Some(value.0);
                }
                name => {
                    ctx.emit_error(DecodeError::unexpected(
                        child,
                        "node",
                        format!("unexpected node `{}`", name.escape_default()),
                    ));
                }
            }
        }

        Ok(Self {
            source,
            uniforms,
            clock_uniforms,
            margin,
            input,
        })
    }
}

/// Parses `uniform "name" 1 2 3` or `uniform "name" time-scale=1.0` into the lists.
///
/// A later uniform of the same name replaces the earlier one.
pub(crate) fn parse_uniform<S: ErrorSpan>(
    child: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
    uniforms: &mut Vec<(String, Vec<f32>)>,
    clock_uniforms: &mut Vec<(String, f32)>,
) -> Result<(), DecodeError<S>> {
    let mut args = child.arguments.iter();
    let Some(name) = args.next() else {
        ctx.emit_error(DecodeError::missing(child, "uniform name is required"));
        return Ok(());
    };
    let name: String = knuffel::traits::DecodeScalar::decode(name, ctx)?;
    if !is_identifier(&name) {
        ctx.emit_error(DecodeError::conversion(
            &child.arguments[0].literal,
            "uniform name must be a GLSL identifier",
        ));
        return Ok(());
    }

    // A uniform that follows the time.
    let mut rate = None;
    for (prop, val) in &child.properties {
        if &***prop == "time-scale" {
            let value: FloatOrInt<{ i32::MIN }, { i32::MAX }> =
                knuffel::traits::DecodeScalar::decode(val, ctx)?;
            rate = Some(value.0 as f32);
        } else {
            ctx.emit_error(DecodeError::unexpected(
                prop,
                "property",
                format!("unexpected property `{}`", prop.escape_default()),
            ));
        }
    }
    if let Some(rate) = rate {
        if let Some(val) = args.next() {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "a uniform with time-scale takes no values",
            ));
        }
        uniforms.retain(|(x, _)| *x != name);
        clock_uniforms.retain(|(x, _)| *x != name);
        clock_uniforms.push((name, rate));
        return Ok(());
    }

    let mut values = Vec::new();
    for val in args {
        let value: FloatOrInt<{ i32::MIN }, { i32::MAX }> =
            knuffel::traits::DecodeScalar::decode(val, ctx)?;
        values.push(value.0 as f32);
    }
    if !(1..=4).contains(&values.len()) {
        ctx.emit_error(DecodeError::unexpected(
            child,
            "node",
            "uniform needs from 1 to 4 numbers",
        ));
        return Ok(());
    }

    uniforms.retain(|(x, _)| *x != name);
    clock_uniforms.retain(|(x, _)| *x != name);
    uniforms.push((name, values));
    Ok(())
}

pub(crate) fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
