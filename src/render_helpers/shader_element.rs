use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::rc::Rc;
use std::sync::Once;

use glam::{Mat3, Vec2};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    ffi, link_program, Capability, GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform,
    UniformDesc, UniformName, UniformType,
};
use smithay::backend::renderer::utils::{CommitCounter, OpaqueRegions};
use smithay::backend::renderer::{
    DebugFlags, Frame as _, FrameContext, Offscreen as _, Texture as _,
};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::blur::{Blur, BlurOptions};
use super::renderer::AsGlesFrame;
use super::resources::Resources;
use super::shaders::{ProgramType, Shaders};
use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};

/// Renders a shader with optional texture input, on the primary GPU.
#[derive(Debug, Clone)]
pub struct ShaderRenderElement {
    program: ProgramType,
    id: Id,
    commit_counter: CommitCounter,
    area: Rectangle<f64, Logical>,
    opaque_regions: Vec<Rectangle<f64, Logical>>,
    // Should only be used for visual improvements, i.e. corner radius anti-aliasing.
    scale: f32,
    alpha: f32,
    additional_uniforms: Rc<[Uniform<'static>]>,
    textures: HashMap<String, GlesTexture>,
    kind: Kind,
    framebuffer_backdrop: Option<FramebufferBackdrop>,
}

/// Backdrop that a [`ShaderRenderElement`] takes from the framebuffer under it when drawing.
///
/// This is for what an offscreen texture can't have, like the windows under a floating window.
#[derive(Debug, Clone)]
pub struct FramebufferBackdrop {
    /// Names of the textures that get the backdrop, blurred if it has blur.
    pub textures: Vec<String>,
    /// Names of the textures that get the backdrop without the blur.
    pub sharp_textures: Vec<String>,
    /// Size of the backdrop texture that the uniforms expect. It covers the whole element, the
    /// right way up.
    pub size: Size<i32, Buffer>,
    /// Blur for the captured backdrop, for surfaces that blur what's under them themselves rather
    /// than over a background effect drawn under them. Such a surface doesn't blur its whole
    /// rectangle, only what its shader draws.
    pub blur: Option<BlurOptions>,
}

/// What a [`ShaderRenderElement`] captured from the framebuffer.
#[derive(Debug, Default)]
struct CapturedBackdrop {
    texture: Option<GlesTexture>,
    blur: Option<Blur>,
    /// The blurred backdrop, when the backdrop has blur.
    blurred: Option<GlesTexture>,
}

#[derive(Debug, Clone)]
pub struct ShaderProgram(Rc<ShaderProgramInner>);

#[derive(Debug)]
struct ShaderProgramInner {
    normal: ShaderProgramInternal,
    debug: ShaderProgramInternal,
    uniform_tint: ffi::types::GLint,
    /// Whether to skip uniforms that the program doesn't have, rather than fail drawing.
    ///
    /// This is for user shaders, where the uniforms are the ones active after linking, and the
    /// driver drops the ones that the shader declares but doesn't use.
    lenient: bool,
}

#[derive(Debug)]
struct ShaderProgramInternal {
    program: ffi::types::GLuint,
    uniform_tex_matrix: ffi::types::GLint,
    uniform_matrix: ffi::types::GLint,
    uniform_size: ffi::types::GLint,
    uniform_scale: ffi::types::GLint,
    uniform_alpha: ffi::types::GLint,
    attrib_vert: ffi::types::GLint,
    attrib_vert_position: ffi::types::GLint,
    additional_uniforms: HashMap<String, UniformDesc>,
    texture_uniforms: HashMap<String, ffi::types::GLint>,
}

impl PartialEq for ShaderProgram {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

unsafe fn compile_program(
    gl: &ffi::Gles2,
    src: &str,
    additional_uniforms: &[UniformName<'_>],
    texture_uniforms: &[&str],
    // destruction_callback_sender: Sender<CleanupResource>,
) -> Result<ShaderProgram, GlesError> {
    let shader = format!("#version 100\n{src}");
    let program = unsafe { link_program(gl, include_str!("shaders/texture.vert"), &shader)? };
    let debug_shader = format!("#version 100\n#define DEBUG_FLAGS\n{src}");
    let debug_program =
        unsafe { link_program(gl, include_str!("shaders/texture.vert"), &debug_shader)? };

    let vert = c"vert";
    let vert_position = c"vert_position";
    let matrix = c"matrix";
    let tex_matrix = c"tex_matrix";
    let size = c"niri_size";
    let scale = c"niri_scale";
    let alpha = c"niri_alpha";
    let tint = c"niri_tint";

    Ok(ShaderProgram(Rc::new(ShaderProgramInner {
        normal: ShaderProgramInternal {
            program,
            uniform_matrix: gl.GetUniformLocation(program, matrix.as_ptr()),
            uniform_tex_matrix: gl.GetUniformLocation(program, tex_matrix.as_ptr()),
            uniform_size: gl.GetUniformLocation(program, size.as_ptr()),
            uniform_scale: gl.GetUniformLocation(program, scale.as_ptr()),
            uniform_alpha: gl.GetUniformLocation(program, alpha.as_ptr()),
            attrib_vert: gl.GetAttribLocation(program, vert.as_ptr()),
            attrib_vert_position: gl.GetAttribLocation(program, vert_position.as_ptr()),
            additional_uniforms: additional_uniforms
                .iter()
                .map(|uniform| {
                    let name =
                        CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(program, name.as_ptr());
                    (
                        uniform.name.clone().into_owned(),
                        UniformDesc {
                            location,
                            type_: uniform.type_,
                        },
                    )
                })
                .collect(),
            texture_uniforms: texture_uniforms
                .iter()
                .map(|name_| {
                    let name = CString::new(name_.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(program, name.as_ptr());
                    (name_.to_string(), location)
                })
                .collect(),
        },
        debug: ShaderProgramInternal {
            program: debug_program,
            uniform_matrix: gl.GetUniformLocation(debug_program, matrix.as_ptr()),
            uniform_tex_matrix: gl.GetUniformLocation(debug_program, tex_matrix.as_ptr()),
            uniform_size: gl.GetUniformLocation(debug_program, size.as_ptr()),
            uniform_scale: gl.GetUniformLocation(debug_program, scale.as_ptr()),
            uniform_alpha: gl.GetUniformLocation(debug_program, alpha.as_ptr()),
            attrib_vert: gl.GetAttribLocation(debug_program, vert.as_ptr()),
            attrib_vert_position: gl.GetAttribLocation(debug_program, vert_position.as_ptr()),
            additional_uniforms: additional_uniforms
                .iter()
                .map(|uniform| {
                    let name =
                        CString::new(uniform.name.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(debug_program, name.as_ptr());
                    (
                        uniform.name.clone().into_owned(),
                        UniformDesc {
                            location,
                            type_: uniform.type_,
                        },
                    )
                })
                .collect(),
            texture_uniforms: texture_uniforms
                .iter()
                .map(|name_| {
                    let name = CString::new(name_.as_bytes()).expect("Interior null in name");
                    let location = gl.GetUniformLocation(debug_program, name.as_ptr());
                    (name_.to_string(), location)
                })
                .collect(),
        },
        uniform_tint: gl.GetUniformLocation(debug_program, tint.as_ptr()),
        lenient: false,
    })))
}

/// Uniforms that [`ShaderRenderElement`] sets by itself.
const ELEMENT_UNIFORMS: [&str; 6] = [
    "matrix",
    "tex_matrix",
    "niri_size",
    "niri_scale",
    "niri_alpha",
    "niri_tint",
];

unsafe fn program_internal(
    gl: &ffi::Gles2,
    program: ffi::types::GLuint,
    additional_uniforms: &[UniformName<'_>],
    texture_uniforms: &[&str],
) -> ShaderProgramInternal {
    let location = |name: &str| {
        let name = CString::new(name.as_bytes()).expect("Interior null in name");
        unsafe { gl.GetUniformLocation(program, name.as_ptr()) }
    };

    ShaderProgramInternal {
        program,
        uniform_matrix: location("matrix"),
        uniform_tex_matrix: location("tex_matrix"),
        uniform_size: location("niri_size"),
        uniform_scale: location("niri_scale"),
        uniform_alpha: location("niri_alpha"),
        attrib_vert: unsafe { gl.GetAttribLocation(program, c"vert".as_ptr()) },
        attrib_vert_position: unsafe { gl.GetAttribLocation(program, c"vert_position".as_ptr()) },
        additional_uniforms: additional_uniforms
            .iter()
            .map(|uniform| {
                let desc = UniformDesc {
                    location: location(&uniform.name),
                    type_: uniform.type_,
                };
                (uniform.name.clone().into_owned(), desc)
            })
            .collect(),
        texture_uniforms: texture_uniforms
            .iter()
            .map(|name| (name.to_string(), location(name)))
            .collect(),
    }
}

/// Returns the shader info log, or the program info log if `program` is true.
unsafe fn info_log(gl: &ffi::Gles2, object: ffi::types::GLuint, program: bool) -> String {
    unsafe {
        let mut max_len = 0;
        if program {
            gl.GetProgramiv(object, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);
        } else {
            gl.GetShaderiv(object, ffi::INFO_LOG_LENGTH, &mut max_len as *mut _);
        }

        let mut log = vec![0u8; max_len.max(1) as usize];
        let mut len = 0;
        if program {
            gl.GetProgramInfoLog(object, max_len, &mut len, log.as_mut_ptr() as *mut _);
        } else {
            gl.GetShaderInfoLog(object, max_len, &mut len, log.as_mut_ptr() as *mut _);
        }
        log.truncate(len.max(0) as usize);

        String::from_utf8_lossy(&log).trim_end().to_owned()
    }
}

unsafe fn compile_shader_with_log(
    gl: &ffi::Gles2,
    variant: ffi::types::GLuint,
    src: &str,
) -> Result<ffi::types::GLuint, String> {
    unsafe {
        let shader = gl.CreateShader(variant);
        if shader == 0 {
            return Err(String::from("error creating a shader object"));
        }

        gl.ShaderSource(
            shader,
            1,
            &src.as_ptr() as *const *const u8 as *const *const ffi::types::GLchar,
            &(src.len() as i32) as *const _,
        );
        gl.CompileShader(shader);

        let mut status = ffi::FALSE as i32;
        gl.GetShaderiv(shader, ffi::COMPILE_STATUS, &mut status as *mut _);
        if status == ffi::FALSE as i32 {
            let log = info_log(gl, shader, false);
            gl.DeleteShader(shader);
            return Err(log);
        }

        Ok(shader)
    }
}

/// Like smithay's `link_program()`, but returns the compiler or linker log on errors.
unsafe fn link_program_with_log(
    gl: &ffi::Gles2,
    vert_src: &str,
    frag_src: &str,
) -> Result<ffi::types::GLuint, String> {
    unsafe {
        let vert = compile_shader_with_log(gl, ffi::VERTEX_SHADER, vert_src)?;
        let frag = match compile_shader_with_log(gl, ffi::FRAGMENT_SHADER, frag_src) {
            Ok(frag) => frag,
            Err(err) => {
                gl.DeleteShader(vert);
                return Err(err);
            }
        };

        let program = gl.CreateProgram();
        gl.AttachShader(program, vert);
        gl.AttachShader(program, frag);
        gl.LinkProgram(program);
        gl.DetachShader(program, vert);
        gl.DetachShader(program, frag);
        gl.DeleteShader(vert);
        gl.DeleteShader(frag);

        let mut status = ffi::FALSE as i32;
        gl.GetProgramiv(program, ffi::LINK_STATUS, &mut status as *mut _);
        if status == ffi::FALSE as i32 {
            let log = info_log(gl, program, true);
            gl.DeleteProgram(program);
            return Err(log);
        }

        Ok(program)
    }
}

/// Returns the active uniforms of a linked program, other than the ones that
/// [`ShaderRenderElement`] sets by itself, and the names of its samplers.
///
/// Elements of arrays come one by one, as `name[0]`, `name[1]` and so on.
unsafe fn active_uniforms(
    gl: &ffi::Gles2,
    program: ffi::types::GLuint,
) -> (Vec<UniformName<'static>>, Vec<String>) {
    let mut rv = Vec::new();
    let mut samplers = Vec::new();

    unsafe {
        let mut count = 0;
        gl.GetProgramiv(program, ffi::ACTIVE_UNIFORMS, &mut count as *mut _);
        let mut max_len = 0;
        gl.GetProgramiv(
            program,
            ffi::ACTIVE_UNIFORM_MAX_LENGTH,
            &mut max_len as *mut _,
        );

        let mut buf = vec![0u8; max_len.max(1) as usize];
        for i in 0..count.max(0) as u32 {
            let mut len = 0;
            let mut size = 0;
            let mut type_ = 0;
            gl.GetActiveUniform(
                program,
                i,
                max_len,
                &mut len,
                &mut size,
                &mut type_,
                buf.as_mut_ptr() as *mut _,
            );
            let Ok(name) = std::str::from_utf8(&buf[..len.max(0) as usize]) else {
                continue;
            };
            if ELEMENT_UNIFORMS.contains(&name) {
                continue;
            }
            if type_ == ffi::SAMPLER_2D {
                samplers.push(name.to_owned());
                continue;
            }

            let type_ = match type_ {
                ffi::FLOAT => UniformType::_1f,
                ffi::FLOAT_VEC2 => UniformType::_2f,
                ffi::FLOAT_VEC3 => UniformType::_3f,
                ffi::FLOAT_VEC4 => UniformType::_4f,
                ffi::INT | ffi::BOOL => UniformType::_1i,
                ffi::FLOAT_MAT2 => UniformType::Matrix2x2,
                ffi::FLOAT_MAT3 => UniformType::Matrix3x3,
                ffi::FLOAT_MAT4 => UniformType::Matrix4x4,
                _ => continue,
            };

            // Drivers report arrays by their first element, `name[0]`.
            match name.strip_suffix("[0]") {
                Some(base) => {
                    for i in 0..size.max(1) {
                        rv.push(UniformName::new(format!("{base}[{i}]"), type_));
                    }
                }
                None if size == 1 => rv.push(UniformName::new(name.to_owned(), type_)),
                None => (),
            }
        }
    }

    (rv, samplers)
}

impl ShaderProgram {
    pub fn compile(
        renderer: &mut GlesRenderer,
        src: &str,
        additional_uniforms: &[UniformName<'_>],
        texture_uniforms: &[&str],
    ) -> Result<Self, GlesError> {
        renderer.with_context(move |gl| unsafe {
            compile_program(gl, src, additional_uniforms, texture_uniforms)
        })?
    }

    /// Compiles a shader that comes from the user.
    ///
    /// Unlike [`compile()`](Self::compile()), this returns the compiler log on errors and
    /// registers the uniforms that are active after linking, instead of a fixed list. When
    /// drawing, uniforms that the program doesn't have, or that have a different type, are
    /// skipped. `src` must start with the `#version` line.
    pub fn compile_user(
        renderer: &mut GlesRenderer,
        src: &str,
        texture_uniforms: &[&str],
    ) -> Result<Self, String> {
        let debug_src = match src.split_once('\n') {
            Some((version, rest)) => format!("{version}\n#define DEBUG_FLAGS\n{rest}"),
            None => src.to_owned(),
        };
        let vert = include_str!("shaders/texture.vert");

        let res = renderer.with_context(move |gl| unsafe {
            let program = link_program_with_log(gl, vert, src)?;
            let debug_program = match link_program_with_log(gl, vert, &debug_src) {
                Ok(program) => program,
                Err(err) => {
                    gl.DeleteProgram(program);
                    return Err(err);
                }
            };

            let (uniforms, samplers) = active_uniforms(gl, program);
            let mut textures: Vec<&str> = texture_uniforms.to_vec();
            textures.extend(samplers.iter().map(String::as_str));
            textures.sort_unstable();
            textures.dedup();
            Ok(ShaderProgram(Rc::new(ShaderProgramInner {
                normal: program_internal(gl, program, &uniforms, &textures),
                debug: program_internal(gl, debug_program, &uniforms, &textures),
                uniform_tint: gl.GetUniformLocation(debug_program, c"niri_tint".as_ptr()),
                lenient: true,
            })))
        });

        match res {
            Ok(res) => res,
            Err(err) => Err(format!("{err:?}")),
        }
    }

    /// Returns whether the shader uses this uniform.
    ///
    /// Drivers drop uniforms that don't affect the result, so this is false for a uniform that the
    /// shader declares but doesn't actually use.
    pub fn uses_uniform(&self, name: &str) -> bool {
        let program = &self.0.normal;
        match name {
            "niri_size" => program.uniform_size != -1,
            "niri_scale" => program.uniform_scale != -1,
            "niri_alpha" => program.uniform_alpha != -1,
            _ => program
                .additional_uniforms
                .get(name)
                .is_some_and(|desc| desc.location != -1),
        }
    }

    /// Returns the type of a uniform that the shader uses.
    pub fn uniform_type(&self, name: &str) -> Option<UniformType> {
        let desc = self.0.normal.additional_uniforms.get(name)?;
        (desc.location != -1).then_some(desc.type_)
    }

    pub fn destroy(self, renderer: &mut GlesRenderer) -> Result<(), GlesError> {
        renderer.with_context(move |gl| unsafe {
            gl.DeleteProgram(self.0.normal.program);
            gl.DeleteProgram(self.0.debug.program);
        })
    }
}

impl ShaderRenderElement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program: ProgramType,
        size: Size<f64, Logical>,
        opaque_regions: Option<Vec<Rectangle<f64, Logical>>>,
        // Should only be used for visual improvements, i.e. corner radius anti-aliasing.
        scale: f32,
        alpha: f32,
        additional_uniforms: Rc<[Uniform<'static>]>,
        textures: HashMap<String, GlesTexture>,
        kind: Kind,
    ) -> Self {
        Self {
            program,
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            area: Rectangle::from_size(size),
            opaque_regions: opaque_regions.unwrap_or_default(),
            scale,
            alpha,
            additional_uniforms,
            textures,
            kind,
            framebuffer_backdrop: None,
        }
    }

    pub fn empty(program: ProgramType, kind: Kind) -> Self {
        Self {
            program,
            id: Id::new(),
            commit_counter: CommitCounter::default(),
            area: Rectangle::default(),
            opaque_regions: vec![],
            scale: 1.,
            alpha: 1.,
            additional_uniforms: Rc::new([]),
            textures: HashMap::new(),
            kind,
            framebuffer_backdrop: None,
        }
    }

    pub fn damage_all(&mut self) {
        self.commit_counter.increment();
    }

    pub fn update(
        &mut self,
        size: Size<f64, Logical>,
        opaque_regions: Option<Vec<Rectangle<f64, Logical>>>,
        scale: f32,
        alpha: f32,
        uniforms: Rc<[Uniform<'static>]>,
        textures: HashMap<String, GlesTexture>,
    ) {
        self.area.size = size;
        self.opaque_regions = opaque_regions.unwrap_or_default();
        self.scale = scale;
        self.alpha = alpha;
        self.additional_uniforms = uniforms;
        self.textures = textures;

        self.commit_counter.increment();
    }

    pub fn with_location(mut self, location: Point<f64, Logical>) -> Self {
        self.area.loc = location;
        self
    }

    /// Takes the id and commit of an element drawn before, so that it isn't damaged unless the
    /// commit changes.
    pub fn with_id(mut self, id: Id, commit_counter: CommitCounter) -> Self {
        self.id = id;
        self.commit_counter = commit_counter;
        self
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = alpha;
        self
    }

    /// Makes the element take its backdrop from the framebuffer under it.
    pub fn with_framebuffer_backdrop(mut self, backdrop: FramebufferBackdrop) -> Self {
        self.framebuffer_backdrop = Some(backdrop);
        self
    }
}

impl Element for ShaderRenderElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit_counter
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(Size::from((1., 1.)))
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.area.to_physical_precise_round(scale)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.opaque_regions
            .iter()
            .map(|region| region.to_physical_precise_down(scale))
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        self.kind
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.framebuffer_backdrop.is_some()
    }
}

impl RenderElement<GlesRenderer> for ShaderRenderElement {
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        let Some(backdrop) = &self.framebuffer_backdrop else {
            return Ok(());
        };
        let _span = tracy_client::span!("ShaderRenderElement::capture_framebuffer");

        let captured = cache.get_or_insert::<RefCell<CapturedBackdrop>, _>(Default::default);
        let mut captured = captured.borrow_mut();
        // Dropped before blurring again: the blur renders into a texture that must have no other
        // references.
        captured.blurred = None;

        if !frame.capabilities().contains(&Capability::Blit) {
            static WARN: Once = Once::new();
            WARN.call_once(|| {
                warn!("shader backdrops of floating windows disabled: renderer does not support framebuffer blitting");
            });
            captured.texture = None;
            return Ok(());
        }

        // A blit can mirror the framebuffer contents, but not turn them.
        let transform = frame.transformation();
        let (mirror_x, mirror_y) = match transform {
            Transform::Normal => (false, false),
            Transform::Flipped => (true, false),
            Transform::Flipped180 => (false, true),
            Transform::_180 => (true, true),
            _ => {
                static WARN: Once = Once::new();
                WARN.call_once(|| {
                    warn!("shader backdrops of floating windows disabled on rotated outputs");
                });
                captured.texture = None;
                return Ok(());
            }
        };

        let output_rect = Rectangle::from_size(frame.output_size());
        let size = backdrop.size;
        if captured.texture.as_ref().is_some_and(|t| t.size() != size) {
            captured.texture = None;
        }
        let texture = match &captured.texture {
            Some(texture) => texture.clone(),
            None => {
                let mut guard = frame.renderer();
                let texture = guard.as_mut().create_buffer(Fourcc::Abgr8888, size)?;
                captured.texture.insert(texture).clone()
            }
        };

        // Only the part of the element on the output can be captured, the rest stays clear.
        let blit = dst.intersection(output_rect).map(|clamped| {
            let from = transform.transform_rect_in(clamped, &output_rect.size);

            let rel = Rectangle::new(clamped.loc - dst.loc, clamped.size);
            let sx = size.w as f64 / dst.size.w.max(1) as f64;
            let sy = size.h as f64 / dst.size.h.max(1) as f64;
            let (mut x0, mut x1) = (
                (rel.loc.x as f64 * sx).round() as i32,
                ((rel.loc.x + rel.size.w) as f64 * sx).round() as i32,
            );
            let (mut y0, mut y1) = (
                (rel.loc.y as f64 * sy).round() as i32,
                ((rel.loc.y + rel.size.h) as f64 * sy).round() as i32,
            );
            if mirror_x {
                std::mem::swap(&mut x0, &mut x1);
            }
            if mirror_y {
                std::mem::swap(&mut y0, &mut y1);
            }
            (from, [x0, y0, x1, y1])
        });

        // Like in FramebufferEffectElement, renderer.with_context() would reset the binding of
        // the frame that we blit from.
        frame.with_context(|gl| unsafe {
            while gl.GetError() != ffi::NO_ERROR {}

            let mut current_fbo = 0i32;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current_fbo as *mut _);

            // Clear and BlitFramebuffer are affected by the scissor test, we don't want that.
            gl.Disable(ffi::SCISSOR_TEST);

            let mut fbo = 0;
            gl.GenFramebuffers(1, &mut fbo as *mut _);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                texture.tex_id(),
                0,
            );

            gl.ClearColor(0., 0., 0., 0.);
            gl.Clear(ffi::COLOR_BUFFER_BIT);

            if let Some((from, to)) = blit {
                gl.BlitFramebuffer(
                    from.loc.x,
                    from.loc.y,
                    from.loc.x + from.size.w,
                    from.loc.y + from.size.h,
                    to[0],
                    to[1],
                    to[2],
                    to[3],
                    ffi::COLOR_BUFFER_BIT,
                    ffi::LINEAR,
                );
            }

            // Restore state set by GlesFrame that we just modified.
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current_fbo as u32);
            gl.Enable(ffi::SCISSOR_TEST);

            gl.DeleteFramebuffers(1, &mut fbo as *mut _);

            if gl.GetError() != ffi::NO_ERROR {
                Err(GlesError::BlitError)
            } else {
                Ok(())
            }
        })??;

        if let Some(options) = backdrop.blur {
            let mut guard = frame.renderer();
            let renderer = guard.as_mut();
            let captured = &mut *captured;
            if captured.blur.is_none() {
                captured.blur = Blur::new(renderer);
            }
            if let Some(blur) = &mut captured.blur {
                let res = blur
                    .prepare_textures(
                        |fourcc, size| renderer.create_buffer(fourcc, size),
                        &texture,
                        options,
                    )
                    .and_then(|()| blur.render(renderer, &texture, options));
                match res {
                    Ok(blurred) => captured.blurred = Some(blurred),
                    Err(err) => warn!("error blurring the shader backdrop: {err:?}"),
                }
            }
        }

        Ok(())
    }

    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dest: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("ShaderRenderElement::draw");

        let frame = frame.as_gles_frame();

        // The textures of the element, and the backdrop it captured from the framebuffer.
        let mut textures: Vec<(&str, GlesTexture)> = self
            .textures
            .iter()
            .map(|(name, texture)| (name.as_str(), texture.clone()))
            .collect();
        if let Some(backdrop) = &self.framebuffer_backdrop {
            let captured = cache
                .and_then(|cache| cache.get::<RefCell<CapturedBackdrop>>())
                .and_then(|captured| {
                    let captured = captured.borrow();
                    let sharp = captured.texture.clone()?;
                    Some((
                        captured.blurred.clone().unwrap_or_else(|| sharp.clone()),
                        sharp,
                    ))
                });
            if let Some((texture, sharp)) = captured {
                for name in &backdrop.textures {
                    textures.push((name, texture.clone()));
                }
                for name in &backdrop.sharp_textures {
                    textures.push((name, sharp.clone()));
                }
            }
        }

        let Some(shader) = Shaders::get_from_frame(frame).program(self.program) else {
            return Ok(());
        };

        let Some(resources) = Resources::get(frame) else {
            return Ok(());
        };
        let mut resources = resources.borrow_mut();

        let supports_instancing = frame.capabilities().contains(&Capability::Instancing);

        // prepare the vertices
        resources.vertices.clear();
        if supports_instancing {
            resources.vertices.extend(damage.iter().flat_map(|rect| {
                let dest_size = dest.size;

                let rect_constrained_loc = rect
                    .loc
                    .constrain(Rectangle::from_extremities((0, 0), dest_size.to_point()));
                let rect_clamped_size = rect.size.clamp(
                    (0, 0),
                    (dest_size.to_point() - rect_constrained_loc).to_size(),
                );

                let rect = Rectangle::new(rect_constrained_loc, rect_clamped_size);
                [
                    rect.loc.x as f32,
                    rect.loc.y as f32,
                    rect.size.w as f32,
                    rect.size.h as f32,
                ]
            }));
        } else {
            resources.vertices.extend(damage.iter().flat_map(|rect| {
                let dest_size = dest.size;

                let rect_constrained_loc = rect
                    .loc
                    .constrain(Rectangle::from_extremities((0, 0), dest_size.to_point()));
                let rect_clamped_size = rect.size.clamp(
                    (0, 0),
                    (dest_size.to_point() - rect_constrained_loc).to_size(),
                );

                let rect = Rectangle::new(rect_constrained_loc, rect_clamped_size);
                // Add the 4 f32s per damage rectangle for each of the 6 vertices.
                (0..6).flat_map(move |_| {
                    [
                        rect.loc.x as f32,
                        rect.loc.y as f32,
                        rect.size.w as f32,
                        rect.size.h as f32,
                    ]
                })
            }));
        }

        if resources.vertices.is_empty() {
            return Ok(());
        }

        // dest position and scale
        let mut matrix = Mat3::from_translation(Vec2::new(dest.loc.x as f32, dest.loc.y as f32));

        let scale = src.size.to_f64() / dest.size.to_f64();
        let tex_matrix = Mat3::from_scale(Vec2::new(scale.x as f32, scale.y as f32));
        let tex_matrix =
            Mat3::from_translation(Vec2::new(src.loc.x as f32, src.loc.y as f32)) * tex_matrix;

        //apply output transformation
        matrix = Mat3::from_cols_array(frame.projection()) * matrix;

        let has_debug = !frame.debug_flags().is_empty();
        let has_tint = frame.debug_flags().contains(DebugFlags::TINT);

        // render
        let span_loc = smithay::gpu_span_location!("draw shader");
        frame.with_profiled_context(span_loc, move |gl| -> Result<(), GlesError> {
            let program = if has_debug {
                &shader.0.debug
            } else {
                &shader.0.normal
            };

            unsafe {
                for (i, (_, texture)) in textures.iter().enumerate() {
                    gl.ActiveTexture(ffi::TEXTURE0 + i as u32);
                    gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_S,
                        ffi::CLAMP_TO_BORDER as i32,
                    );
                    gl.TexParameteri(
                        ffi::TEXTURE_2D,
                        ffi::TEXTURE_WRAP_T,
                        ffi::CLAMP_TO_BORDER as i32,
                    );
                }

                gl.UseProgram(program.program);

                for (i, (name, _)) in textures.iter().enumerate() {
                    // User shaders don't have to use every texture they are given.
                    let location = match program.texture_uniforms.get(*name) {
                        Some(location) => *location,
                        None if shader.0.lenient => continue,
                        None => program.texture_uniforms[*name],
                    };
                    gl.Uniform1i(location, i as i32);
                }

                gl.UniformMatrix3fv(
                    program.uniform_matrix,
                    1,
                    ffi::FALSE,
                    matrix.as_ref().as_ptr(),
                );
                gl.UniformMatrix3fv(
                    program.uniform_tex_matrix,
                    1,
                    ffi::FALSE,
                    tex_matrix.as_ref().as_ptr(),
                );
                gl.Uniform2f(program.uniform_size, dest.size.w as f32, dest.size.h as f32);
                gl.Uniform1f(program.uniform_scale, self.scale);
                gl.Uniform1f(program.uniform_alpha, self.alpha);

                let tint = if has_tint { 1.0f32 } else { 0.0f32 };
                if has_debug {
                    gl.Uniform1f(shader.0.uniform_tint, tint);
                }

                for uniform in &*self.additional_uniforms {
                    let desc = match program.additional_uniforms.get(&*uniform.name) {
                        Some(desc) => desc,
                        None if shader.0.lenient => continue,
                        None => {
                            return Err(GlesError::UnknownUniform(
                                uniform.name.clone().into_owned(),
                            ))
                        }
                    };
                    if shader.0.lenient && !uniform.value.matches(&desc.type_) {
                        continue;
                    }
                    uniform.value.set(gl, desc)?;
                }

                gl.EnableVertexAttribArray(program.attrib_vert as u32);
                gl.BindBuffer(ffi::ARRAY_BUFFER, resources.vbos[0]);
                gl.VertexAttribPointer(
                    program.attrib_vert as u32,
                    2,
                    ffi::FLOAT,
                    ffi::FALSE,
                    0,
                    std::ptr::null(),
                );

                // vert_position
                gl.EnableVertexAttribArray(program.attrib_vert_position as u32);
                gl.BindBuffer(ffi::ARRAY_BUFFER, resources.vbos[1]);
                gl.BufferData(
                    ffi::ARRAY_BUFFER,
                    (std::mem::size_of::<ffi::types::GLfloat>() * resources.vertices.len())
                        as isize,
                    resources.vertices.as_ptr() as *const _,
                    ffi::STREAM_DRAW,
                );

                gl.VertexAttribPointer(
                    program.attrib_vert_position as u32,
                    4,
                    ffi::FLOAT,
                    ffi::FALSE,
                    0,
                    std::ptr::null(),
                );

                let damage_len = damage.len() as i32;
                if supports_instancing {
                    gl.VertexAttribDivisor(program.attrib_vert as u32, 0);
                    gl.VertexAttribDivisor(program.attrib_vert_position as u32, 1);
                    gl.DrawArraysInstanced(ffi::TRIANGLE_STRIP, 0, 4, damage_len);
                } else {
                    // When we have more than 10 rectangles, draw them in batches of 10.
                    for i in 0..(damage_len - 1) / 10 {
                        gl.DrawArrays(ffi::TRIANGLES, 0, 6);

                        // Set damage pointer to the next 10 rectangles.
                        let offset =
                            (i + 1) as usize * 6 * 4 * std::mem::size_of::<ffi::types::GLfloat>();
                        gl.VertexAttribPointer(
                            program.attrib_vert_position as u32,
                            4,
                            ffi::FLOAT,
                            ffi::FALSE,
                            0,
                            offset as *const _,
                        );
                    }

                    // Draw the up to 10 remaining rectangles.
                    let count = ((damage_len - 1) % 10 + 1) * 6;
                    gl.DrawArrays(ffi::TRIANGLES, 0, count);
                }

                gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
                gl.ActiveTexture(ffi::TEXTURE0);
                gl.BindTexture(ffi::TEXTURE_2D, 0);
                gl.DisableVertexAttribArray(program.attrib_vert as u32);
                gl.DisableVertexAttribArray(program.attrib_vert_position as u32);
            }

            Ok(())
        })??;

        Ok(())
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for ShaderRenderElement {
    fn capture_framebuffer(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::capture_framebuffer(self, frame, src, dst, cache)?;
        Ok(())
    }

    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();

        RenderElement::<GlesRenderer>::draw(self, frame, src, dst, damage, opaque_regions, cache)?;

        Ok(())
    }

    fn underlying_storage(
        &self,
        _renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        // If scanout for things other than Wayland buffers is implemented, this will need to take
        // the target GPU into account.
        None
    }
}
