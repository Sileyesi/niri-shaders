//! The `shader-pipeline` layer rule property: several shader passes with textures that last
//! between frames, like the layer effects of ShojiWM.
//!
//! ```kdl
//! shader-pipeline {
//!     state "flow" scale=0.5
//!     pass "water-flow.frag" target="flow" repeat=4
//!     draw "water-surface.frag" {
//!         texture "flow" from="flow"
//!         texture "sharp" from="surface"
//!         uniform "emboss" 5
//!     }
//! }
//! ```
//!
//! Textures are referred to by name: a state, `surface` for what the surface draws, or `backdrop`
//! for what's behind it (niri's blur, when the surface has it).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use knuffel::errors::DecodeError;
use knuffel::traits::ErrorSpan;

use crate::utils::expand_home_path;
use crate::window_shader::{is_identifier, parse_uniform};
use crate::{FloatOrInt, Includes, ShaderSource};

#[derive(Debug, Clone, PartialEq)]
pub struct ShaderPipeline {
    /// Textures that keep their contents between frames.
    pub states: Vec<PipelineState>,
    /// Passes that render into the states, in order, every frame.
    pub passes: Vec<PipelinePass>,
    /// The pass that draws the surface.
    pub draw: PipelinePass,
    /// Whether the draw pass goes behind the surface, rather than in place of it.
    pub behind: bool,
}

/// Name of the texture with what the surface draws.
pub const SURFACE_TEXTURE: &str = "surface";
/// Name of the texture with what's behind the surface.
pub const BACKDROP_TEXTURE: &str = "backdrop";
/// Name of the texture with what's behind the surface without the blur, where the surface blurs
/// what's under it itself. Elsewhere it's the same as the backdrop.
pub const SHARP_BACKDROP_TEXTURE: &str = "backdrop-sharp";

/// Whether the texture is what's behind the surface, blurred or not.
pub fn is_backdrop(name: &str) -> bool {
    name == BACKDROP_TEXTURE || name == SHARP_BACKDROP_TEXTURE
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelineState {
    pub name: String,
    /// Size relative to the surface.
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PipelinePass {
    pub source: ShaderSource,
    /// The state that the pass renders into. `None` for the draw pass.
    pub target: Option<String>,
    /// The texture that the pass gets as `tex`: by default the target for a pass, and the surface
    /// for the draw pass.
    pub input: Option<String>,
    /// How many times the pass runs every frame.
    pub repeat: u32,
    pub uniforms: Vec<(String, Vec<f32>)>,
    pub clock_uniforms: Vec<(String, f32)>,
    /// Other textures of the pass: the name of the sampler, and of the texture.
    pub textures: Vec<(String, String)>,
}

impl ShaderPipeline {
    /// Whether any pass reads what's behind the surface.
    pub fn uses_backdrop(&self) -> bool {
        self.passes.iter().chain([&self.draw]).any(|pass| {
            pass.input.as_deref().is_some_and(is_backdrop)
                || pass.textures.iter().any(|(_, from)| is_backdrop(from))
        })
    }
}

impl<S: ErrorSpan> knuffel::Decode<S> for ShaderPipeline {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        for val in &node.arguments {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "unexpected argument",
            ));
        }

        let mut states = Vec::new();
        let mut passes = Vec::new();
        let mut draw = None;
        let mut behind = false;

        for child in node.children() {
            match &**child.node_name {
                "state" => {
                    let name: String = match child.arguments.first() {
                        Some(val) => knuffel::traits::DecodeScalar::decode(val, ctx)?,
                        None => {
                            ctx.emit_error(DecodeError::missing(child, "state name is required"));
                            continue;
                        }
                    };
                    let mut scale = 1.;
                    for (prop, val) in &child.properties {
                        if &***prop == "scale" {
                            let value: FloatOrInt<0, 4> =
                                knuffel::traits::DecodeScalar::decode(val, ctx)?;
                            scale = value.0;
                        } else {
                            ctx.emit_error(DecodeError::unexpected(
                                prop,
                                "property",
                                format!("unexpected property `{}`", prop.escape_default()),
                            ));
                        }
                    }
                    if scale <= 0. {
                        ctx.emit_error(DecodeError::conversion(child, "scale must be above 0"));
                        continue;
                    }
                    states.push(PipelineState { name, scale });
                }
                "pass" => {
                    let (pass, _) = decode_pass(child, ctx)?;
                    if pass.target.is_none() {
                        ctx.emit_error(DecodeError::missing(
                            child,
                            "a pass needs the state it renders into, target=\"name\"",
                        ));
                        continue;
                    }
                    passes.push(pass);
                }
                "draw" => {
                    let (pass, draw_behind) = decode_pass(child, ctx)?;
                    if pass.target.is_some() || pass.repeat != 1 {
                        ctx.emit_error(DecodeError::unexpected(
                            child,
                            "node",
                            "the draw pass has no target and runs once",
                        ));
                    }
                    behind = draw_behind;
                    draw = Some(pass);
                }
                name => ctx.emit_error(DecodeError::unexpected(
                    child,
                    "node",
                    format!("unexpected node `{}`", name.escape_default()),
                )),
            }
        }

        let draw = draw.ok_or_else(|| DecodeError::missing(node, "a draw pass is required"))?;

        // Every texture that a pass refers to must exist.
        let is_state = |name: &str| states.iter().any(|state| state.name == name);
        for pass in passes.iter().chain([&draw]) {
            if let Some(target) = &pass.target {
                if !is_state(target) {
                    ctx.emit_error(DecodeError::missing(
                        node,
                        format!("there's no state called {target:?} to render into"),
                    ));
                }
            }
            let read = pass
                .input
                .iter()
                .chain(pass.textures.iter().map(|(_, from)| from));
            for name in read {
                if !is_state(name) && name != SURFACE_TEXTURE && !is_backdrop(name) {
                    ctx.emit_error(DecodeError::missing(
                        node,
                        format!(
                            "there's no texture called {name:?}: textures are states, \
                             \"surface\", \"backdrop\" or \"backdrop-sharp\""
                        ),
                    ));
                }
            }
        }

        Ok(Self {
            states,
            passes,
            draw,
            behind,
        })
    }
}

/// Decodes a `pass` or `draw` node, and its `behind` property.
fn decode_pass<S: ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<(PipelinePass, bool), DecodeError<S>> {
    let path: String = match node.arguments.first() {
        Some(val) => knuffel::traits::DecodeScalar::decode(val, ctx)?,
        None => {
            return Err(DecodeError::missing(
                node,
                "the path of the shader is required",
            ))
        }
    };
    let path = expand_home_path(PathBuf::from(path), node, ctx)?;
    // Watch the file, so that editing it reloads the pipeline.
    let includes = ctx.get::<Rc<RefCell<Includes>>>().unwrap();
    includes.borrow_mut().0.push(path.clone());
    let source = ShaderSource::read_file(path);

    let mut target = None;
    let mut input = None;
    let mut behind = false;
    let mut repeat = 1;
    for (prop, val) in &node.properties {
        match &***prop {
            "target" => target = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?),
            "input" => input = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?),
            "behind" => behind = knuffel::traits::DecodeScalar::decode(val, ctx)?,
            "repeat" => {
                let value: FloatOrInt<1, 64> = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                repeat = value.0 as u32;
            }
            _ => ctx.emit_error(DecodeError::unexpected(
                prop,
                "property",
                format!("unexpected property `{}`", prop.escape_default()),
            )),
        }
    }

    let mut uniforms = Vec::new();
    let mut clock_uniforms = Vec::new();
    let mut textures: Vec<(String, String)> = Vec::new();

    for child in node.children() {
        match &**child.node_name {
            "uniform" => parse_uniform(child, ctx, &mut uniforms, &mut clock_uniforms)?,
            "texture" => {
                let name: String = parse_arg_name(child, ctx)?;
                if !is_identifier(&name) {
                    ctx.emit_error(DecodeError::conversion(
                        child,
                        "texture name must be a GLSL identifier",
                    ));
                    continue;
                }

                let mut texture = None;
                for (prop, val) in &child.properties {
                    if &***prop == "from" {
                        let from: String = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                        texture = Some(from);
                    } else {
                        ctx.emit_error(DecodeError::unexpected(
                            prop,
                            "property",
                            format!("unexpected property `{}`", prop.escape_default()),
                        ));
                    }
                }

                let Some(texture) = texture else {
                    ctx.emit_error(DecodeError::missing(
                        child,
                        "a texture needs the name of what it shows, from=\"name\"",
                    ));
                    continue;
                };
                textures.retain(|(x, _)| *x != name);
                textures.push((name, texture));
            }
            name => ctx.emit_error(DecodeError::unexpected(
                child,
                "node",
                format!("unexpected node `{}`", name.escape_default()),
            )),
        }
    }

    let pass = PipelinePass {
        source,
        target,
        input,
        repeat,
        uniforms,
        clock_uniforms,
        textures,
    };
    Ok((pass, behind))
}

/// The first argument of a node, which may have properties too.
fn parse_arg_name<S: ErrorSpan>(
    node: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Result<String, DecodeError<S>> {
    match node.arguments.first() {
        Some(val) => knuffel::traits::DecodeScalar::decode(val, ctx),
        None => Err(DecodeError::missing(node, "a name is required")),
    }
}
