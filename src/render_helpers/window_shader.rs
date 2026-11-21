//! Shaders that draw whole windows, set with the `shader` window rule.
//!
//! The tile (the window with its border, focus ring and shadow) is rendered to an offscreen
//! texture, the same way as for the open animation, and then drawn through the user's fragment
//! shader. Two formats of shaders are supported, told apart by the function that they define:
//!
//! - niri: `vec4 window_color(vec3 coords_geo, vec3 size_geo)`, with the same coordinates and
//!   uniform names as the custom animation shaders.
//! - ShojiWM: `vec4 shader_main(EffectContext effect)`, with the `EffectContext` prelude of ShojiWM
//!   and the window texture called `tex`, so that ShojiWM effect shaders work unchanged.
//! - ShojiWM paint: `vec4 paint_main(PaintContext ctx)`, the decoration shaders of ShojiWM. They
//!   paint the frame of the window, and the window shows through its hole.
//! - Older ShojiWM: `vec4 shader_main(vec2 uv, vec2 rect_size)`, where `tex` is what's behind the
//!   window. The result goes behind the window.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use anyhow::Context as _;
use glam::{Mat3, Vec2};
use niri_config::{CornerRadius, ShaderInput};
use smithay::backend::renderer::element::{Element as _, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{
    GlesRenderer, GlesTexture, Uniform, UniformType, UniformValue,
};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::Texture as _;
use smithay::utils::{Logical, Point, Rectangle, Scale, Size, Transform};

use super::offscreen::{OffscreenBuffer, OffscreenData};
use super::shader_element::{FramebufferBackdrop, ShaderProgram, ShaderRenderElement};
use super::shaders::{mat3_uniform, ProgramType, Shaders};

/// Declarations of the uniforms common to both formats.
const HEADER: &str = include_str!("shaders/window_header.frag");
/// The window texture and helpers of the niri format.
const NIRI_PRELUDE: &str = include_str!("shaders/window_prelude.frag");
const NIRI_EPILOGUE: &str = include_str!("shaders/window_epilogue.frag");
/// The `EffectContext` prelude of the ShojiWM format, after the textures.
const SHOJI_EFFECT_CONTEXT: &str = include_str!("shaders/shoji_effect_context.frag");
const SHOJI_EPILOGUE: &str = include_str!("shaders/shoji_epilogue.frag");
/// The ShojiWM format with `input "backdrop"`.
const SHOJI_BACKDROP_EPILOGUE: &str = include_str!("shaders/shoji_backdrop_epilogue.frag");
/// The `PaintContext` prelude of the ShojiWM paint format.
const SHOJI_PAINT_PRELUDE: &str = include_str!("shaders/shoji_paint_prelude.frag");
const SHOJI_PAINT_EPILOGUE: &str = include_str!("shaders/shoji_paint_epilogue.frag");
/// The older ShojiWM API, with the backdrop as `tex`.
const SHOJI_LEGACY_PRELUDE: &str = include_str!("shaders/shoji_legacy_prelude.frag");
const SHOJI_LEGACY_EPILOGUE: &str = include_str!("shaders/shoji_legacy_epilogue.frag");

/// Seconds after which `niri_time` wraps around, to keep it precise as a `float`.
const TIME_WRAP_SECS: f64 = 3600.;

/// Uniform that ShojiWM effects use for their progress, which niri drives with the close animation
/// unless the config sets it.
const PROGRESS_UNIFORM: &str = "progress";

/// Uniform that ShojiWM configs feed with the time, which niri sets to `niri_time` unless the
/// config sets it.
const TIME_UNIFORM: &str = "time";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `vec4 window_color(vec3 coords_geo, vec3 size_geo)`.
    Niri,
    /// `vec4 shader_main(EffectContext effect)`.
    ShojiWm,
    /// `vec4 shader_main(EffectContext effect)` with `input "backdrop"`: `tex` is what's behind
    /// the window, and the result goes behind it.
    ShojiWmBackdrop,
    /// `vec4 paint_main(PaintContext ctx)`.
    ShojiWmPaint,
    /// `vec4 shader_main(vec2 uv, vec2 rect_size)`.
    ShojiWmLegacy,
}

impl Format {
    /// Tells the format by the entry point that the code defines.
    pub fn detect(code: &str) -> Result<Self, String> {
        let code = strip_comments(code);
        let found: Vec<_> = [
            ("window_color", Self::Niri),
            ("shader_main", Self::ShojiWm),
            ("paint_main", Self::ShojiWmPaint),
        ]
        .into_iter()
        .filter(|(name, _)| mentions_function(&code, name))
        .map(|(name, format)| {
            // The older API takes the coordinates instead of an EffectContext.
            if format == Self::ShojiWm && first_param_type(&code, name) == Some("vec2") {
                (name, Self::ShojiWmLegacy)
            } else {
                (name, format)
            }
        })
        .collect();

        match *found {
            [(_, format)] => Ok(format),
            [] => Err(String::from(
                "the shader must define `vec4 window_color(vec3 coords_geo, vec3 size_geo)`, \
                 `vec4 shader_main(EffectContext effect)` or `vec4 paint_main(PaintContext ctx)`",
            )),
            _ => {
                let names: Vec<_> = found.iter().map(|(name, _)| format!("{name}()")).collect();
                Err(format!(
                    "the shader defines {}, it must have only one of them",
                    names.join(" and ")
                ))
            }
        }
    }

    /// The format to compile the shader as, for the given input.
    fn with_input(self, input: ShaderInput) -> Self {
        match (self, input) {
            (Format::ShojiWm, ShaderInput::Backdrop) => Format::ShojiWmBackdrop,
            (format, _) => format,
        }
    }

    pub fn texture_name(self) -> &'static str {
        match self {
            Format::Niri
            | Format::ShojiWmPaint
            | Format::ShojiWmLegacy
            | Format::ShojiWmBackdrop => "niri_tex",
            Format::ShojiWm => "tex",
        }
    }

    /// Name of the texture with what's behind the window, in formats that have it.
    pub fn backdrop_texture_name(self) -> Option<&'static str> {
        match self {
            Format::Niri => Some("niri_backdrop"),
            Format::ShojiWmLegacy | Format::ShojiWmBackdrop => Some("tex"),
            Format::ShojiWm | Format::ShojiWmPaint => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Niri => "niri",
            Format::ShojiWm => "ShojiWM",
            Format::ShojiWmBackdrop => "ShojiWM, backdrop input",
            Format::ShojiWmPaint => "ShojiWM paint",
            Format::ShojiWmLegacy => "ShojiWM (older API)",
        }
    }

    /// Whether shaders of this format can draw around the tile, within the margin.
    fn has_margin(self) -> bool {
        matches!(self, Format::Niri | Format::ShojiWmPaint)
    }
}

/// Removes `//` and `/* */` comments, keeping the line breaks.
fn strip_comments(code: &str) -> String {
    let mut rv = String::with_capacity(code.len());
    let mut rest = code;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |i| &after[i..]);
        } else if let Some(after) = rest.strip_prefix("/*") {
            let end = after.find("*/").map_or(after.len(), |i| i + 2);
            rv.extend(after[..end.min(after.len())].chars().filter(|&c| c == '\n'));
            rest = &after[end.min(after.len())..];
        } else {
            let c = rest.chars().next().unwrap();
            rv.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    rv
}

/// Type of the first parameter of the first `name(` in the code.
fn first_param_type<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let start = code.find(name)? + name.len();
    let params = code[start..].trim_start().strip_prefix('(')?.trim_start();
    let end = params
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(params.len());
    Some(&params[..end])
}

/// Whether the code has `name` as a whole identifier followed by `(`.
fn mentions_function(code: &str, name: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    code.match_indices(name).any(|(i, _)| {
        let before = code[..i].chars().next_back();
        let after = code[i + name.len()..].trim_start();
        !before.is_some_and(is_ident) && after.starts_with('(')
    })
}

/// Puts the user's code between the declarations and the `main()` of its format.
pub fn assemble(code: &str, format: Format) -> String {
    let (prelude, epilogue): (&[&str], _) = match format {
        Format::Niri => (&[NIRI_PRELUDE], NIRI_EPILOGUE),
        Format::ShojiWm => (
            &[
                "uniform sampler2D tex;\n#define niri_tex tex\n\n",
                SHOJI_EFFECT_CONTEXT,
            ],
            SHOJI_EPILOGUE,
        ),
        Format::ShojiWmBackdrop => (
            &[
                "uniform sampler2D tex;\nuniform sampler2D niri_tex;\n\n",
                SHOJI_EFFECT_CONTEXT,
            ],
            SHOJI_BACKDROP_EPILOGUE,
        ),
        Format::ShojiWmPaint => (&[SHOJI_PAINT_PRELUDE], SHOJI_PAINT_EPILOGUE),
        Format::ShojiWmLegacy => (&[SHOJI_LEGACY_PRELUDE], SHOJI_LEGACY_EPILOGUE),
    };

    let mut src = String::from("#version 100\n");
    src.push_str(HEADER);
    for part in prelude {
        src.push_str(part);
    }
    // Make the compiler count lines from the start of the user's code. Mesa numbers the line
    // after `#line N` as N (the GLSL ES 1.00 spec says N + 1, but drivers follow later versions).
    src.push_str("\n#line 1\n");
    src.push_str(code);
    src.push('\n');
    src.push_str(epilogue);
    src
}

fn code_key(code: &str, input: ShaderInput) -> u64 {
    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    (input == ShaderInput::Backdrop).hash(&mut hasher);
    hasher.finish()
}

/// A window shader that compiled successfully.
#[derive(Debug, Clone)]
pub struct CompiledShader {
    /// Hash of the code, which is also the key of the program in [`Shaders`].
    pub key: u64,
    pub format: Format,
    pub program: ShaderProgram,
}

impl CompiledShader {
    /// Whether the shader changes over time, so the window needs a redraw every frame.
    ///
    /// `clock_uniforms` are the uniforms that the config ties to the time.
    pub fn is_animated(
        &self,
        user_uniforms: &[(String, Vec<f32>)],
        clock_uniforms: &[(String, f32)],
    ) -> bool {
        self.program.uses_uniform("niri_time")
            || self.binds_time(user_uniforms)
            || clock_uniforms
                .iter()
                .any(|(name, _)| self.program.uses_uniform(name))
    }

    /// Whether niri sets the shader's own `time` uniform, which it does unless the config does.
    fn binds_time(&self, user_uniforms: &[(String, Vec<f32>)]) -> bool {
        self.program.uniform_type(TIME_UNIFORM) == Some(UniformType::_1f)
            && !user_uniforms.iter().any(|(name, _)| name == TIME_UNIFORM)
    }

    /// Whether the shader gets what's behind the window as a texture of its own.
    pub fn uses_backdrop(&self) -> bool {
        self.program.uses_uniform("niri_geo_to_backdrop")
    }

    /// Whether the shader follows the pointer.
    pub fn uses_cursor(&self) -> bool {
        self.program.uses_uniform("niri_cursor")
    }

    /// Whether the shader changes for a while after the window gets the focus.
    pub fn uses_focus_time(&self) -> bool {
        self.program.uses_uniform("niri_focus_time")
    }

    /// Whether the shader draws the close animation of the window itself.
    pub fn uses_close_progress(&self, user_uniforms: &[(String, Vec<f32>)]) -> bool {
        self.program.uses_uniform("niri_close_progress")
            || (self.program.uses_uniform(PROGRESS_UNIFORM)
                && !user_uniforms
                    .iter()
                    .any(|(name, _)| name == PROGRESS_UNIFORM))
    }

    /// Returns the uniforms from the config that the shader doesn't use as given.
    pub fn uniform_problems(&self, user_uniforms: &[(String, Vec<f32>)]) -> Vec<String> {
        let mut rv = Vec::new();
        for (name, values) in user_uniforms {
            let Some(type_) = self.program.uniform_type(name) else {
                rv.push(format!(
                    "uniform \"{name}\" is not used by the shader (or not declared in it)"
                ));
                continue;
            };
            let expected = user_uniform_value(values).type_();
            if type_ != expected {
                rv.push(format!(
                    "uniform \"{name}\" has {} value(s) in the config, but the shader declares \
                     it as {}",
                    values.len(),
                    glsl_type_name(type_),
                ));
            }
        }
        rv
    }
}

fn glsl_type_name(type_: UniformType) -> &'static str {
    match type_ {
        UniformType::_1f => "float",
        UniformType::_2f => "vec2",
        UniformType::_3f => "vec3",
        UniformType::_4f => "vec4",
        UniformType::_1i => "int",
        UniformType::Matrix2x2 => "mat2",
        UniformType::Matrix3x3 => "mat3",
        UniformType::Matrix4x4 => "mat4",
        _ => "another type",
    }
}

fn user_uniform_value(values: &[f32]) -> UniformValue {
    match *values {
        [x] => UniformValue::_1f(x),
        [x, y] => UniformValue::_2f(x, y),
        [x, y, z] => UniformValue::_3f(x, y, z),
        [x, y, z, w, ..] => UniformValue::_4f(x, y, z, w),
        [] => UniformValue::_1f(0.),
    }
}

/// Window shader programs, compiled on first use and cached by the hash of their code.
///
/// Windows with the same shader share one program, and code that failed to compile isn't compiled
/// again.
#[derive(Debug, Default)]
pub struct WindowShaderPrograms {
    entries: HashMap<u64, Result<CompiledShader, String>>,
}

impl WindowShaderPrograms {
    pub fn get(&self, key: u64) -> Option<ShaderProgram> {
        let compiled = self.entries.get(&key)?.as_ref().ok()?;
        Some(compiled.program.clone())
    }
}

/// Returns the compiled program for the code, compiling it if it's new.
///
/// On errors, returns the message with the compiler log, where line numbers match the user's code.
/// `name` is used in the log.
pub fn compile(
    renderer: &mut GlesRenderer,
    name: &str,
    code: &str,
    input: ShaderInput,
) -> Result<CompiledShader, String> {
    let key = code_key(code, input);
    if let Some(entry) = Shaders::get(renderer).window.borrow().entries.get(&key) {
        return entry.clone();
    }

    let _span = tracy_client::span!("window_shader::compile");

    let res = Format::detect(code).and_then(|format| {
        let format = format.with_input(input);
        let src = assemble(code, format);
        let mut textures = vec![format.texture_name()];
        textures.extend(format.backdrop_texture_name());
        ShaderProgram::compile_user(renderer, &src, &textures).map(|program| CompiledShader {
            key,
            format,
            program,
        })
    });

    match &res {
        Ok(compiled) => {
            debug!(
                "compiled window shader {name} ({} format{})",
                compiled.format.name(),
                if compiled.is_animated(&[], &[]) {
                    ", animated"
                } else {
                    ""
                }
            );
        }
        Err(err) => warn!("error compiling window shader {name}:\n{err}"),
    }

    Shaders::get(renderer)
        .window
        .borrow_mut()
        .entries
        .insert(key, res.clone());
    res
}

/// Converts the time from the animation clock to the value for `niri_time`.
pub fn time_uniform(time: std::time::Duration) -> f32 {
    (time.as_secs_f64() % TIME_WRAP_SECS) as f32
}

/// What a window shader is drawn with, other than the tile contents.
#[derive(Debug, Clone)]
pub struct Params {
    /// Size of the tile, which is the geometry for `coords_geo`.
    pub geo_size: Size<f64, Logical>,
    /// Window geometry within the tile.
    pub window_rect: Rectangle<f64, Logical>,
    /// Outer corner radius of the tile.
    pub radius: CornerRadius,
    /// Corner radius of the window itself.
    pub window_radius: CornerRadius,
    pub border_width: f64,
    pub clip_to_geometry: bool,
    /// How far around the contents the shader can draw, for the formats that can.
    pub margin: f64,
    pub time: f32,
    /// Focus from 0 to 1.
    pub focused: f32,
    /// Pointer position relative to the tile, in logical pixels.
    pub cursor: Option<Point<f64, Logical>>,
    pub close_progress: f32,
    pub random_seed: f32,
    /// Seconds since the window got the focus, or -1 if it never had it.
    pub focus_time: f32,
    /// Color of the window's border (or focus ring without a border), premultiplied.
    pub border_color: [f32; 4],
    pub user_uniforms: Rc<[(String, Vec<f32>)]>,
    /// Uniforms that the config ties to the time, with how fast they go.
    pub clock_uniforms: Rc<[(String, f32)]>,
}

/// A window shader that draws the close animation of its window, with the parameters from right
/// before the window closed.
#[derive(Debug, Clone)]
pub struct CloseShader {
    pub shader: CompiledShader,
    pub params: Params,
}

/// Common uniforms of both formats, and the ones specific to the shader's format.
/// `backdrop` is the offset and size of the texture with the backdrop, if the shader gets one.
fn uniforms(
    shader: &CompiledShader,
    params: &Params,
    area: Rectangle<f64, Logical>,
    tex_offset: Point<f64, Logical>,
    tex_size: Size<i32, smithay::utils::Buffer>,
    backdrop: Option<(Point<f64, Logical>, Size<i32, smithay::utils::Buffer>)>,
    scale: Scale<f64>,
) -> Vec<Uniform<'static>> {
    let area_loc = Vec2::new(area.loc.x as f32, area.loc.y as f32);
    let area_size = Vec2::new(area.size.w as f32, area.size.h as f32);
    let geo_size = Vec2::new(params.geo_size.w as f32, params.geo_size.h as f32);

    // The tile is at (0, 0), area and texture offsets are relative to it.
    let input_to_geo =
        Mat3::from_scale(area_size / geo_size) * Mat3::from_translation(area_loc / area_size);

    let tex_scale = Vec2::new(scale.x as f32, scale.y as f32);
    // From the geometry coordinates to those of a texture at this offset from the tile.
    let geo_to = |offset: Point<f64, Logical>, size: Size<i32, smithay::utils::Buffer>| {
        let loc = Vec2::new(offset.x as f32, offset.y as f32);
        let size = Vec2::new(size.w as f32, size.h as f32) / tex_scale;
        Mat3::from_translation(-loc / size) * Mat3::from_scale(geo_size / size)
    };
    let tex_size_px = Vec2::new(tex_size.w as f32, tex_size.h as f32);
    let geo_to_tex = geo_to(tex_offset, tex_size);

    let cursor = params.cursor.map_or([-1., -1.], |pos| {
        [
            (pos.x / params.geo_size.w) as f32,
            (pos.y / params.geo_size.h) as f32,
        ]
    });

    let mut rv = vec![
        mat3_uniform("niri_input_to_geo", input_to_geo),
        Uniform::new("niri_geo_size", geo_size.to_array()),
        mat3_uniform("niri_geo_to_tex", geo_to_tex),
        Uniform::new("niri_time", params.time),
        Uniform::new("niri_focused", params.focused),
        Uniform::new("niri_cursor", cursor),
        Uniform::new("niri_close_progress", params.close_progress),
        Uniform::new("niri_random_seed", params.random_seed),
        Uniform::new("niri_corner_radius", <[f32; 4]>::from(params.radius)),
        Uniform::new("niri_border_width", params.border_width as f32),
        Uniform::new("niri_border_color", params.border_color),
        Uniform::new("niri_focus_time", params.focus_time),
    ];

    if let Some((offset, size)) = backdrop {
        rv.push(mat3_uniform("niri_geo_to_backdrop", geo_to(offset, size)));
    }

    if shader.binds_time(&params.user_uniforms)
        && !params
            .clock_uniforms
            .iter()
            .any(|(name, _)| name == TIME_UNIFORM)
    {
        rv.push(Uniform::new(TIME_UNIFORM, params.time));
    }
    for (name, rate) in &*params.clock_uniforms {
        rv.push(Uniform::new(name.clone(), params.time * rate));
    }

    if matches!(shader.format, Format::ShojiWm | Format::ShojiWmBackdrop) {
        let s = scale.x as f32;
        // With the backdrop input, the context describes the backdrop texture.
        let (tex_offset, tex_size_px) = match (shader.format, backdrop) {
            (Format::ShojiWmBackdrop, Some((offset, size))) => {
                (offset, Vec2::new(size.w as f32, size.h as f32))
            }
            _ => (tex_offset, tex_size_px),
        };
        // Rects in physical pixels of the texture, as (x, y, width, height).
        let rect = |rect: Rectangle<f64, Logical>| {
            [
                (rect.loc.x - tex_offset.x) as f32 * s,
                (rect.loc.y - tex_offset.y) as f32 * s,
                rect.size.w as f32 * s,
                rect.size.h as f32 * s,
            ]
        };
        let radius = |radius: CornerRadius| <[f32; 4]>::from(radius.scaled_by(s));
        let border = params.border_width as f32 * s;

        let frame_rect = rect(Rectangle::from_size(params.geo_size));
        let content_rect = rect(params.window_rect);
        let clip = params.clip_to_geometry;

        rv.extend([
            Uniform::new("niri_shoji_texture_size", tex_size_px.to_array()),
            Uniform::new("niri_shoji_content_rect", content_rect),
            Uniform::new("niri_shoji_frame_rect", frame_rect),
            Uniform::new("effect_frame_radius_phy_px", radius(params.radius)),
            Uniform::new("effect_frame_border_phy_px", [border; 4]),
            Uniform::new(
                "effect_clip_rect_phy_px",
                if clip { content_rect } else { [0.; 4] },
            ),
            Uniform::new(
                "effect_clip_radius_phy_px",
                if clip {
                    radius(params.window_radius)
                } else {
                    [0.; 4]
                },
            ),
            Uniform::new("effect_has_clip", if clip { 1f32 } else { 0. }),
            Uniform::new("effect_scale", s),
        ]);
    }

    if shader.format == Format::ShojiWmPaint {
        let s = scale.x as f32;
        let size = [params.geo_size.w as f32 * s, params.geo_size.h as f32 * s];
        let window = params.window_rect;
        let inner_rect = [
            window.loc.x as f32 * s,
            window.loc.y as f32 * s,
            window.size.w as f32 * s,
            window.size.h as f32 * s,
        ];
        let border = params.border_width as f32 * s;

        // The ones that the config can override come before the user uniforms, which are set
        // last.
        rv.extend([
            Uniform::new("shoji_node_rect", [0., 0., size[0], size[1]]),
            Uniform::new("shoji_border", [border; 4]),
            Uniform::new("shoji_radius", <[f32; 4]>::from(params.radius.scaled_by(s))),
            Uniform::new("shoji_padding", [0f32; 4]),
            Uniform::new("shoji_content_rect", inner_rect),
            Uniform::new("shoji_inner_rect", inner_rect),
            Uniform::new(
                "shoji_inner_radius",
                <[f32; 4]>::from(params.window_radius.scaled_by(s)),
            ),
            Uniform::new("shoji_hole", 1f32),
            Uniform::new("shoji_scale", s),
            Uniform::new("shoji_background", [0f32; 4]),
            Uniform::new("shoji_border_color_top", params.border_color),
            Uniform::new("shoji_border_color_right", params.border_color),
            Uniform::new("shoji_border_color_bottom", params.border_color),
            Uniform::new("shoji_border_color_left", params.border_color),
            Uniform::new("shoji_opacity", 1f32),
            Uniform::new("shoji_clip_enabled", 0f32),
            Uniform::new("shoji_rounded_clip_enabled", 0f32),
            Uniform::new("shoji_box_fill", 1f32),
            Uniform::new("shoji_box_border", 1f32),
        ]);
    }

    if shader.uses_close_progress(&params.user_uniforms) {
        rv.push(Uniform::new(PROGRESS_UNIFORM, params.close_progress));
    }

    rv.extend(
        params
            .user_uniforms
            .iter()
            .map(|(name, values)| Uniform::new(name.clone(), user_uniform_value(values))),
    );

    rv
}

/// Flattens uniform values for telling whether anything changed since the last frame.
pub fn push_uniform_bits(bits: &mut Vec<u32>, uniform: &Uniform) {
    match &uniform.value {
        UniformValue::_1f(x) => bits.push(x.to_bits()),
        UniformValue::_2f(x, y) => bits.extend([x.to_bits(), y.to_bits()]),
        UniformValue::_3f(x, y, z) => bits.extend([x.to_bits(), y.to_bits(), z.to_bits()]),
        UniformValue::_4f(x, y, z, w) => {
            bits.extend([x.to_bits(), y.to_bits(), z.to_bits(), w.to_bits()])
        }
        UniformValue::Matrix3x3 { matrices, .. } => {
            bits.extend(matrices.iter().flatten().map(|x| x.to_bits()))
        }
        _ => bits.push(u32::MAX),
    }
}

/// What a shader gets as the backdrop, for shaders that get it apart from the window.
pub enum Backdrop<'a, E> {
    None,
    /// The elements behind the window, positioned like the window elements.
    Elements(&'a [E]),
    /// The framebuffer under the shader, for what elements can't have, like other windows. The
    /// background effect should be drawn right under the shader.
    Framebuffer,
}

/// Draws a tile through its window shader.
#[derive(Debug, Default)]
pub struct WindowShaderBuffer {
    offscreen: OffscreenBuffer,
    /// What's behind the window, for shaders that get it apart from the window.
    backdrop: OffscreenBuffer,
    /// Id and commit of the element drawn last, and the inputs it was drawn with.
    ///
    /// The element keeps the id, and its commit changes only when the inputs change, so a shader
    /// that doesn't change over time causes damage only when the window does.
    last: RefCell<Option<(Id, CommitCounter, Vec<u32>)>>,
}

impl WindowShaderBuffer {
    /// Renders the tile elements, positioned with the tile at (0, 0), through the shader.
    ///
    /// `location` is where the tile should end up.
    #[allow(clippy::too_many_arguments)]
    pub fn render<E: RenderElement<GlesRenderer>>(
        &self,
        renderer: &mut GlesRenderer,
        elements: &[E],
        backdrop: Backdrop<E>,
        shader: &CompiledShader,
        params: &Params,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> anyhow::Result<(ShaderRenderElement, OffscreenData)> {
        let _span = tracy_client::span!("WindowShaderBuffer::render");

        let (elem, _sync_point, mut data) = self
            .offscreen
            .render(renderer, scale, elements)
            .context("error rendering to offscreen buffer")?;

        // OffscreenBuffer renders with Transform::Normal and the scale that we passed.
        let tex_offset = elem.offset();
        let texture = elem.texture().clone();
        let tex_size = texture.size();

        let area = draw_area(
            shader,
            params,
            Rectangle::new(tex_offset, elem.logical_size()),
            scale,
        );

        // The backdrop from the framebuffer covers the area that the shader draws.
        let framebuffer_backdrop_size = matches!(backdrop, Backdrop::Framebuffer).then(|| {
            area.size
                .to_physical_precise_round(scale)
                .to_logical(1)
                .to_buffer(1, Transform::Normal)
        });
        let backdrop = match backdrop {
            Backdrop::Elements(elements) => {
                let (elem, _sync_point, _data) = self
                    .backdrop
                    .render(renderer, scale, elements)
                    .context("error rendering the backdrop to offscreen buffer")?;
                Some((elem.offset(), elem.texture().clone(), elem.current_commit()))
            }
            Backdrop::None | Backdrop::Framebuffer => None,
        };
        let backdrop_geo = match framebuffer_backdrop_size {
            Some(size) => Some((area.loc, size)),
            None => backdrop
                .as_ref()
                .map(|(offset, texture, _)| (*offset, texture.size())),
        };
        let uniforms = uniforms(
            shader,
            params,
            area,
            tex_offset,
            tex_size,
            backdrop_geo,
            scale,
        );

        let commit_bits = |commit: CommitCounter| {
            commit.distance(Some(CommitCounter::default())).unwrap_or(0) as u32
        };
        let mut bits = vec![
            (shader.key >> 32) as u32,
            shader.key as u32,
            texture.tex_id(),
            commit_bits(elem.current_commit()),
            alpha.to_bits(),
            (scale.x as f32).to_bits(),
            (area.size.w as f32).to_bits(),
            (area.size.h as f32).to_bits(),
        ];
        if let Some((_, texture, commit)) = &backdrop {
            bits.extend([texture.tex_id(), commit_bits(*commit)]);
        }
        for uniform in &uniforms {
            push_uniform_bits(&mut bits, uniform);
        }

        let mut textures = HashMap::from([(String::from(shader.format.texture_name()), texture)]);
        if let (Some(name), Some((_, texture, _))) =
            (shader.format.backdrop_texture_name(), backdrop)
        {
            textures.insert(String::from(name), texture);
        }

        let mut last = self.last.borrow_mut();
        let (id, commit) = match &mut *last {
            Some((id, commit, last_bits)) => {
                if *last_bits != bits {
                    commit.increment();
                    *last_bits = bits;
                }
                (id.clone(), *commit)
            }
            None => {
                let id = Id::new();
                let commit = CommitCounter::default();
                *last = Some((id.clone(), commit, bits));
                (id, commit)
            }
        };

        let elem = ShaderRenderElement::new(
            ProgramType::Window(shader.key),
            area.size,
            None,
            scale.x as f32,
            alpha,
            Rc::from(uniforms),
            textures,
            Kind::Unspecified,
        )
        .with_id(id, commit)
        .with_location(location + area.loc);
        let elem = match (
            framebuffer_backdrop_size,
            shader.format.backdrop_texture_name(),
        ) {
            (Some(size), Some(name)) => elem.with_framebuffer_backdrop(FramebufferBackdrop {
                textures: vec![String::from(name)],
                size,
                sharp_textures: Vec::new(),
                blur: None,
            }),
            _ => elem,
        };

        // We're drawing the shader, not the offscreen itself.
        data.id = elem.id().clone();

        Ok((elem, data))
    }
}

/// Area that the shader draws, relative to the tile.
///
/// `contents` is the area of the tile contents. The ShojiWM effect format draws over them, the
/// other formats can draw around them too.
fn draw_area(
    shader: &CompiledShader,
    params: &Params,
    contents: Rectangle<f64, Logical>,
    scale: Scale<f64>,
) -> Rectangle<f64, Logical> {
    let mut area = contents;
    if shader.format.has_margin() && params.margin > 0. {
        let margin = Point::from((params.margin, params.margin));
        let margin = margin.to_physical_precise_round(scale).to_logical(scale);
        area.loc -= margin;
        area.size += margin.upscale(2.).to_size();
    }
    area
}

impl CloseShader {
    /// Draws the snapshot of a closing window through the shader.
    ///
    /// `tex_offset` is the position of the texture relative to the tile, and `location` is where
    /// the tile should end up.
    pub fn render(
        &self,
        texture: &GlesTexture,
        tex_offset: Point<f64, Logical>,
        location: Point<f64, Logical>,
        scale: Scale<f64>,
        progress: f32,
    ) -> ShaderRenderElement {
        let tex_size = texture.size();
        let tex_size_logical = tex_size.to_logical(1, Transform::Normal).to_f64();
        let contents = Rectangle::new(tex_offset, tex_size_logical.downscale(scale.x));

        let mut params = self.params.clone();
        params.close_progress = progress;

        let area = draw_area(&self.shader, &params, contents, scale);
        // The snapshot has the backdrop in the texture already.
        let uniforms = uniforms(
            &self.shader,
            &params,
            area,
            tex_offset,
            tex_size,
            None,
            scale,
        );

        ShaderRenderElement::new(
            ProgramType::Window(self.shader.key),
            area.size,
            None,
            scale.x as f32,
            1.,
            Rc::from(uniforms),
            HashMap::from([(
                String::from(self.shader.format.texture_name()),
                texture.clone(),
            )]),
            Kind::Unspecified,
        )
        .with_location(location + area.loc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_format() {
        let niri = "vec4 window_color(vec3 coords_geo, vec3 size_geo) { return vec4(1.0); }";
        let shoji = "vec4 shader_main (EffectContext effect) {\n return texture2D(tex, effect.texture_uv);\n}";
        assert_eq!(Format::detect(niri), Ok(Format::Niri));
        assert_eq!(Format::detect(shoji), Ok(Format::ShojiWm));

        // Comments and longer names don't count.
        let commented = "// vec4 shader_main(EffectContext e)\n/* window_color( */ void f() {}";
        assert!(Format::detect(commented).is_err());
        assert!(Format::detect("vec4 my_window_color(vec3 a) { return vec4(0.0); }").is_err());
        assert!(Format::detect(&format!("{niri}\n{shoji}")).is_err());

        let legacy = "vec4 shader_main(vec2 uv, vec2 rect_size) { return texture2D(tex, uv); }";
        assert_eq!(Format::detect(legacy), Ok(Format::ShojiWmLegacy));

        let paint = "vec4 paint_main(PaintContext ctx) { return shoji_default_paint(ctx); }";
        assert_eq!(Format::detect(paint), Ok(Format::ShojiWmPaint));
        let err = Format::detect(&format!("{paint}\n{shoji}")).unwrap_err();
        assert!(err.contains("shader_main() and paint_main()"), "{err}");
    }

    #[test]
    fn strips_comments_keeping_lines() {
        let code = "a // b\nc /* d\ne */ f\n";
        assert_eq!(strip_comments(code), "a \nc \n f\n");
    }

    #[test]
    fn assembled_code_counts_lines_from_user_code() {
        let src = assemble("line one\nline two", Format::Niri);
        let (before, _) = src.split_once("line one").unwrap();
        assert!(before.ends_with("#line 1\n"));
        assert!(src.starts_with("#version 100\n"));
    }

    #[test]
    fn user_uniform_types() {
        assert_eq!(user_uniform_value(&[1.]).type_(), UniformType::_1f);
        assert_eq!(user_uniform_value(&[1., 2., 3.]).type_(), UniformType::_3f);
        assert_eq!(
            user_uniform_value(&[1., 2., 3., 4.]).type_(),
            UniformType::_4f
        );
    }
}
