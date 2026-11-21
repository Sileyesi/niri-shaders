//! Shader pipelines, set with the `shader-pipeline` layer and window rules.
//!
//! The surface (a layer surface or a tile) is rendered to a texture, the passes run on textures
//! that keep their contents between frames (states), and a last pass draws the surface. This is the
//! shape of the layer effects of ShojiWM configs, like the water on the wallpaper of
//! liquid-desktop, and the shaders are in the ShojiWM format.
//!
//! ShojiWM configs feed such shaders from TypeScript. niri feeds the same uniforms itself, when a
//! shader declares them and the config doesn't set them: `time`, `dt`, the windows on the output
//! (`window_rects`, `window_motion`, `window_weight`, `window_count`), the pointer
//! (`pointer_state`), splashes when windows open and close (`impulses`, `impulse_power`) and
//! `presence`, which fades the effect in while things move and out once they settle. For a window,
//! the "output" is the window itself, and there are no other windows.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context as _;
use glam::Mat3;
use niri_config::shader_pipeline::{is_backdrop, SHARP_BACKDROP_TEXTURE, SURFACE_TEXTURE};
use niri_config::{PipelinePass, ShaderInput, ShaderPipeline, ShaderSource};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element as _, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture, Uniform};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind as _, Color32F, Frame as _, Offscreen as _, Renderer as _};
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use super::blur::BlurOptions;
use super::shader_element::{FramebufferBackdrop, ShaderRenderElement};
use super::shaders::{mat3_uniform, ProgramType};
use super::window_shader::{self, push_uniform_bits, Backdrop, CompiledShader};

const MAX_WINDOWS: usize = 12;
const MAX_IMPULSES: usize = 8;

/// How long the effect keeps running after the last disturbance, as in liquid-desktop's water.ts.
const SETTLE: Duration = Duration::from_millis(9000);
/// The end of the settle time, over which `presence` fades out.
const PRESENCE_FADE_SECS: f32 = 2.5;
/// How long `presence` takes to fade in.
const PRESENCE_RISE_SECS: f32 = 0.45;
/// The step of the simulation is clamped to this, in seconds per frame.
const DT_MIN: f32 = 0.002;
const DT_MAX: f32 = 0.017;
/// Clamp of the window and pointer velocities, in screens per second.
const WINDOW_MOTION_MAX: f32 = 2.5;
const POINTER_MOTION_MAX: f32 = 3.5;
/// Splash strengths, negative plunges the surface down.
const ENTER_POWER: f32 = -1.;
const EXIT_POWER: f32 = 0.75;
const DROP_POWER: f32 = -1.3;

/// What niri feeds the pipeline about the output, every frame.
#[derive(Debug, Default)]
pub struct Inputs {
    /// Windows on the output by id, in output-local logical coordinates.
    pub windows: Vec<(u64, Rectangle<f64, Logical>)>,
    /// Pointer in output-local logical coordinates, if it's on the output.
    pub pointer: Option<Point<f64, Logical>>,
    pub output_size: Size<f64, Logical>,
}

#[derive(Debug)]
struct TrackedWindow {
    id: u64,
    /// x0, y0, x1, y1, from 0 to 1 over the output.
    rect: [f32; 4],
    motion: [f32; 2],
    seen: bool,
}

/// The uniforms that niri drives, like water.ts does for liquid-desktop.
#[derive(Debug)]
struct Simulation {
    updated_at: Option<Duration>,
    /// Seconds per frame, last frame.
    frame_dt: f32,
    time: f32,
    windows: Vec<TrackedWindow>,
    /// Whether windows have been seen once, so that the ones there at the start don't splash.
    started: bool,
    impulses: [[f32; 4]; MAX_IMPULSES],
    impulse_power: [f32; MAX_IMPULSES],
    next_impulse: usize,
    pointer: Option<[f32; 2]>,
    pointer_state: [f32; 4],
    active_at: Option<Duration>,
    presence: f32,
}

impl Default for Simulation {
    fn default() -> Self {
        Self {
            updated_at: None,
            frame_dt: 0.,
            time: 0.,
            windows: Vec::new(),
            started: false,
            impulses: [[0., 0., 0., -1.]; MAX_IMPULSES],
            impulse_power: [0.; MAX_IMPULSES],
            next_impulse: 0,
            pointer: None,
            pointer_state: [-10., -10., 0., 0.],
            active_at: None,
            presence: 0.,
        }
    }
}

impl Simulation {
    fn update(&mut self, now: Duration, inputs: &Inputs) {
        let dt = self
            .updated_at
            .map_or(0., |at| now.saturating_sub(at).as_secs_f32().min(0.1));
        if self.updated_at == Some(now) {
            return;
        }
        self.updated_at = Some(now);
        self.frame_dt = dt;
        self.time = window_shader::time_uniform(now).max(0.001);

        let size = inputs.output_size;
        if size.w <= 0. || size.h <= 0. {
            return;
        }
        let to_uv = |rect: Rectangle<f64, Logical>| {
            [
                (rect.loc.x / size.w) as f32,
                (rect.loc.y / size.h) as f32,
                ((rect.loc.x + rect.size.w) / size.w) as f32,
                ((rect.loc.y + rect.size.h) / size.h) as f32,
            ]
        };
        let center = |r: [f32; 4]| [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5];
        let splash_radius = |rect: Rectangle<f64, Logical>| {
            (0.5 * rect.size.w.min(rect.size.h) / size.h).clamp(0.05, 0.35) as f32
        };

        let mut moved = false;
        for window in &mut self.windows {
            window.seen = false;
        }
        for (id, rect) in &inputs.windows {
            let uv = to_uv(*rect);
            match self.windows.iter_mut().find(|w| w.id == *id) {
                Some(window) => {
                    let (from, to) = (center(window.rect), center(uv));
                    window.motion = if dt > 0. {
                        [
                            ((to[0] - from[0]) / dt).clamp(-WINDOW_MOTION_MAX, WINDOW_MOTION_MAX),
                            ((to[1] - from[1]) / dt).clamp(-WINDOW_MOTION_MAX, WINDOW_MOTION_MAX),
                        ]
                    } else {
                        [0., 0.]
                    };
                    moved |= window.motion != [0., 0.] || window.rect != uv;
                    window.rect = uv;
                    window.seen = true;
                }
                None => {
                    if self.started {
                        let c = center(uv);
                        self.splash(c, splash_radius(*rect), ENTER_POWER);
                    }
                    self.windows.push(TrackedWindow {
                        id: *id,
                        rect: uv,
                        motion: [0., 0.],
                        seen: true,
                    });
                }
            }
        }
        let gone: Vec<_> = self
            .windows
            .iter()
            .filter(|w| !w.seen)
            .map(|w| {
                (
                    center(w.rect),
                    (w.rect[3] - w.rect[1]).min(w.rect[2] - w.rect[0]),
                )
            })
            .collect();
        for (c, extent) in gone {
            self.splash(c, (0.5 * extent).clamp(0.05, 0.35), EXIT_POWER);
        }
        self.windows.retain(|w| w.seen);
        self.started = true;

        let pointer = inputs
            .pointer
            .map(|p| [(p.x / size.w) as f32, (p.y / size.h) as f32]);
        self.pointer_state = match (pointer, self.pointer) {
            (Some(p), Some(prev)) if dt > 0. => {
                let vx = ((p[0] - prev[0]) / dt).clamp(-POINTER_MOTION_MAX, POINTER_MOTION_MAX);
                let vy = ((p[1] - prev[1]) / dt).clamp(-POINTER_MOTION_MAX, POINTER_MOTION_MAX);
                moved |= vx.abs() + vy.abs() > 0.02;
                [p[0], p[1], vx, vy]
            }
            (Some(p), _) => [p[0], p[1], 0., 0.],
            (None, _) => [-10., -10., 0., 0.],
        };
        self.pointer = pointer;

        if moved {
            self.active_at = Some(now);
        }

        // Fade in quickly while active, and out over the end of the settle time.
        let remaining = self.active_at.map_or(Duration::ZERO, |at| {
            SETTLE.saturating_sub(now.saturating_sub(at))
        });
        let target = (remaining.as_secs_f32() / PRESENCE_FADE_SECS).min(1.);
        self.presence = if target > self.presence {
            (self.presence + dt / PRESENCE_RISE_SECS).min(target)
        } else {
            target
        };
    }

    fn splash(&mut self, pos: [f32; 2], radius: f32, power: f32) {
        self.impulses[self.next_impulse] = [pos[0], pos[1], radius, self.time];
        self.impulse_power[self.next_impulse] = power;
        self.next_impulse = (self.next_impulse + 1) % MAX_IMPULSES;
        if let Some(now) = self.updated_at {
            self.active_at = Some(now);
        }
    }

    /// Whether the effect is running or still fading out.
    fn is_active(&self) -> bool {
        self.presence > 0.
            || self.active_at.is_some_and(|at| {
                self.updated_at
                    .is_some_and(|now| now.saturating_sub(at) < SETTLE)
            })
    }

    /// The uniforms that niri drives, for a pass that runs `repeat` times a frame.
    /// How many times a frame the passes run, so that a long frame at a low refresh rate is split
    /// into steps the simulation can take, and it runs at the same speed at any refresh rate.
    fn steps(&self) -> u32 {
        (self.frame_dt / DT_MAX).ceil().clamp(1., 4.) as u32
    }

    fn uniforms(&self, repeat: u32) -> Vec<Uniform<'static>> {
        let steps = self.steps();
        let dt =
            self.frame_dt.clamp(DT_MIN, DT_MAX * steps as f32) / (repeat.max(1) * steps) as f32;
        let mut rv = vec![
            Uniform::new("time", self.time),
            Uniform::new("niri_time", self.time),
            Uniform::new("dt", dt),
            Uniform::new("presence", self.presence),
            Uniform::new("pointer_state", self.pointer_state),
            Uniform::new("window_count", self.windows.len().min(MAX_WINDOWS) as f32),
        ];
        for i in 0..MAX_WINDOWS {
            let (rect, motion, weight) = match self.windows.get(i) {
                Some(w) => (w.rect, w.motion, 1f32),
                None => ([2., 2., 2., 2.], [0., 0.], 0.),
            };
            rv.push(Uniform::new(format!("window_rects[{i}]"), rect));
            rv.push(Uniform::new(format!("window_motion[{i}]"), motion));
            rv.push(Uniform::new(format!("window_weight[{i}]"), weight));
        }
        for i in 0..MAX_IMPULSES {
            rv.push(Uniform::new(format!("impulses[{i}]"), self.impulses[i]));
            rv.push(Uniform::new(
                format!("impulse_power[{i}]"),
                self.impulse_power[i],
            ));
        }
        rv
    }
}

/// A texture to read from and one to render into, swapped after every pass.
#[derive(Debug)]
struct PingPong {
    front: GlesTexture,
    back: GlesTexture,
    size: Size<i32, Physical>,
}

/// State of the pipeline of a layer surface or a tile.
#[derive(Debug, Default)]
pub struct SurfacePipeline {
    sim: Simulation,
    /// Whether the pipeline only runs while things move, because its draw pass fades with
    /// `presence`. Other pipelines run whenever the surface is drawn.
    gated: Cell<bool>,
    /// The surface, rendered at exactly its size, so that all textures line up.
    surface: RefCell<Option<GlesTexture>>,
    /// What's behind the surface, at the same size.
    backdrop: RefCell<Option<GlesTexture>>,
    states: RefCell<HashMap<String, PingPong>>,
    /// When the passes ran last, so that they run once a frame however many times the surface is
    /// drawn.
    stepped_at: Cell<Option<Duration>>,
    /// What the surface was drawn from when the passes ran last. Passes that depend only on the
    /// surface, like the shape of liquid glass, don't run again while it stays the same.
    surface_key: RefCell<Option<Vec<SurfaceElementKey>>>,
    /// Hash of the passes that ran last.
    passes_hash: Cell<u64>,
    /// What the backdrop was drawn from last, to draw it again only when it changes.
    backdrop_key: RefCell<Option<Vec<SurfaceElementKey>>>,
    /// Bumped whenever a texture that the draw pass reads changes.
    generation: Cell<u32>,
    /// Id of the draw element with its commit and what it was drawn with. A stable id lets the
    /// damage tracker skip the element while it doesn't change, and keep what an element with a
    /// backdrop from the framebuffer captured.
    last: RefCell<Option<(Id, CommitCounter, Vec<u32>)>>,
}

/// What a surface element looked like, to tell whether the surface changed.
type SurfaceElementKey = (Id, CommitCounter, Rectangle<i32, Physical>, u32);

impl SurfacePipeline {
    pub fn update(&mut self, now: Duration, inputs: &Inputs) {
        self.sim.update(now, inputs);
    }

    /// Drops a stone into the effect at a point on the output, in logical coordinates.
    pub fn drop_at(&mut self, pos: Point<f64, Logical>, output_size: Size<f64, Logical>) {
        if output_size.w <= 0. || output_size.h <= 0. {
            return;
        }
        let uv = [
            (pos.x / output_size.w) as f32,
            (pos.y / output_size.h) as f32,
        ];
        self.sim
            .splash(uv, 0.03 + fastrand::f32() * 0.02, DROP_POWER);
    }

    /// Whether the pipeline is running while things move, and needs redraws.
    pub fn needs_redraws(&self) -> bool {
        self.gated.get() && self.sim.is_active()
    }

    /// Renders the surface through the pipeline, and returns the element of the draw pass, or
    /// `None` if the pipeline has nothing to show right now.
    ///
    /// `elements` are the surface's own, with the surface at (0, 0), and `backdrop` what's behind
    /// it. A backdrop from the framebuffer goes only to the draw pass, blurred with
    /// `backdrop_blur`. `location` is where the surface goes.
    #[allow(clippy::too_many_arguments)]
    pub fn render<E: RenderElement<GlesRenderer>, B: RenderElement<GlesRenderer>>(
        &self,
        renderer: &mut GlesRenderer,
        config: &ShaderPipeline,
        elements: &[E],
        backdrop: Backdrop<B>,
        backdrop_blur: Option<BlurOptions>,
        size: Size<f64, Logical>,
        scale: Scale<f64>,
        location: Point<f64, Logical>,
        alpha: f32,
        now: Duration,
    ) -> anyhow::Result<Option<ShaderRenderElement>> {
        let _span = tracy_client::span!("SurfacePipeline::render");

        let draw = compile_pass(renderer, &config.draw)?;
        let gated = draw.program.uses_uniform("presence");
        self.gated.set(gated);
        if gated && !self.sim.is_active() {
            return Ok(None);
        }

        let passes = config
            .passes
            .iter()
            .map(|pass| compile_pass(renderer, pass))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let size_px = size.to_physical_precise_round(scale);
        if size_px.w <= 0 || size_px.h <= 0 {
            anyhow::bail!("the surface is empty");
        }

        // The passes can be skipped when the surface didn't change and none of them changes over
        // time or reads the backdrop.
        let key = elements_key(elements, scale);
        // The passes themselves go in the key too, for when the config changes.
        let passes_hash = {
            let mut hasher = DefaultHasher::new();
            for (pass, compiled) in config.passes.iter().zip(&passes) {
                compiled.key.hash(&mut hasher);
                pass.target.hash(&mut hasher);
                pass.input.hash(&mut hasher);
                pass.repeat.hash(&mut hasher);
                pass.textures.hash(&mut hasher);
                for (name, values) in &pass.uniforms {
                    name.hash(&mut hasher);
                    values.iter().for_each(|x| x.to_bits().hash(&mut hasher));
                }
            }
            hasher.finish()
        };
        let dynamic_names: Vec<String> = self
            .sim
            .uniforms(1)
            .into_iter()
            .map(|uniform| uniform.name.into_owned())
            .collect();
        let static_passes = config.passes.iter().zip(&passes).all(|(pass, compiled)| {
            pass.clock_uniforms.is_empty()
                && !pass.input.as_deref().is_some_and(is_backdrop)
                && !pass.textures.iter().any(|(_, from)| is_backdrop(from))
                && !dynamic_names
                    .iter()
                    .any(|name| compiled.program.uses_uniform(name))
        });
        let surface_unchanged = self
            .surface
            .borrow()
            .as_ref()
            .is_some_and(|texture| texture_size(texture) == size_px)
            && self.surface_key.borrow().as_ref() == Some(&key);
        let passes_unchanged = self.passes_hash.replace(passes_hash) == passes_hash;
        let skip_passes = static_passes && surface_unchanged && passes_unchanged;

        let surface = if surface_unchanged {
            self.surface.borrow().clone().unwrap()
        } else {
            self.bump_generation();
            render_exact(&self.surface, renderer, elements, size_px, scale)?
        };
        *self.surface_key.borrow_mut() = Some(key);
        let from_framebuffer = matches!(backdrop, Backdrop::Framebuffer);
        let backdrop = match backdrop {
            Backdrop::Elements(elements) => {
                let key = elements_key(elements, scale);
                let cached = self.backdrop.borrow().clone().filter(|texture| {
                    texture_size(texture) == size_px
                        && self.backdrop_key.borrow().as_ref() == Some(&key)
                });
                let texture = match cached {
                    Some(texture) => texture,
                    None => {
                        self.bump_generation();
                        render_exact(&self.backdrop, renderer, elements, size_px, scale)?
                    }
                };
                *self.backdrop_key.borrow_mut() = Some(key);
                Some(texture)
            }
            Backdrop::None | Backdrop::Framebuffer => {
                *self.backdrop_key.borrow_mut() = None;
                None
            }
        };
        let sources = Sources {
            surface: &surface,
            backdrop: backdrop.as_ref(),
        };

        let states_ready = self.ensure_states(renderer, config, size_px)?;

        if self.stepped_at.get() != Some(now) && !(skip_passes && states_ready) {
            self.stepped_at.set(Some(now));
            self.bump_generation();
            for (pass, compiled) in config.passes.iter().zip(&passes) {
                for _ in 0..pass.repeat * self.sim.steps() {
                    self.run_pass(renderer, pass, compiled, &sources, scale)?;
                }
            }
        }

        // The draw pass gets the surface as `tex` unless it says otherwise.
        let states = self.states.borrow();
        let input = config.draw.input.as_deref().unwrap_or(SURFACE_TEXTURE);
        let mut textures = HashMap::new();
        if let Some(texture) = sources.resolve(input, &states) {
            textures.insert(String::from("tex"), texture);
        }
        textures.extend(pass_textures(&config.draw, &sources, &states));

        // The textures that the element takes from the framebuffer, blurred and not.
        let mut framebuffer_textures = Vec::new();
        let mut sharp_textures = Vec::new();
        if from_framebuffer {
            let tex = String::from("tex");
            let reads = [(&tex, input)].into_iter().chain(
                config
                    .draw
                    .textures
                    .iter()
                    .map(|(name, from)| (name, from.as_str())),
            );
            for (name, from) in reads {
                if from == SHARP_BACKDROP_TEXTURE {
                    sharp_textures.push(name.clone());
                } else if is_backdrop(from) {
                    framebuffer_textures.push(name.clone());
                }
            }
        }

        let mut uniforms = context_uniforms(size_px, size, scale.x as f32);
        uniforms.extend(self.sim.uniforms(1));
        uniforms.extend(config_uniforms(&config.draw, self.sim.time));

        // What the element is drawn with. Uniforms that the shader doesn't read, like the time for
        // a shader that doesn't change over time, don't count.
        let mut bits = vec![
            (draw.key >> 32) as u32,
            draw.key as u32,
            self.generation.get(),
            alpha.to_bits(),
            (scale.x as f32).to_bits(),
            size_px.w as u32,
            size_px.h as u32,
            u32::from(from_framebuffer),
        ];
        bits.extend(textures.values().map(|texture| texture.tex_id()));
        for uniform in &uniforms {
            if draw.program.uses_uniform(&uniform.name) {
                push_uniform_bits(&mut bits, uniform);
            }
        }
        let (id, commit) = {
            let mut last = self.last.borrow_mut();
            match &mut *last {
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
            }
        };

        let elem = ShaderRenderElement::new(
            ProgramType::Window(draw.key),
            size,
            None,
            scale.x as f32,
            alpha,
            Rc::from(uniforms),
            textures,
            Kind::Unspecified,
        )
        .with_id(id, commit)
        .with_location(location);
        if framebuffer_textures.is_empty() && sharp_textures.is_empty() {
            return Ok(Some(elem));
        }
        Ok(Some(elem.with_framebuffer_backdrop(FramebufferBackdrop {
            textures: framebuffer_textures,
            sharp_textures,
            size: size_px.to_logical(1).to_buffer(1, Transform::Normal),
            blur: backdrop_blur,
        })))
    }

    fn bump_generation(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    fn ensure_states(
        &self,
        renderer: &mut GlesRenderer,
        config: &ShaderPipeline,
        size_px: Size<i32, Physical>,
    ) -> anyhow::Result<bool> {
        let mut states = self.states.borrow_mut();
        let count = states.len();
        states.retain(|name, _| config.states.iter().any(|state| state.name == *name));
        let mut unchanged = states.len() == count;

        for state in &config.states {
            let size = Size::from((
                ((size_px.w as f64 * state.scale).round() as i32).max(1),
                ((size_px.h as f64 * state.scale).round() as i32).max(1),
            ));
            if states.get(&state.name).is_some_and(|s| s.size == size) {
                continue;
            }

            // Like `resize: "clear"` in ShojiWM, a new size starts from nothing.
            let mut create = || -> anyhow::Result<GlesTexture> {
                let buffer_size = size.to_logical(1).to_buffer(1, Transform::Normal);
                let mut texture: GlesTexture = renderer
                    .create_buffer(Fourcc::Abgr16161616f, buffer_size)
                    .context("error creating a state texture")?;
                {
                    let mut target = renderer.bind(&mut texture)?;
                    let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
                    frame.clear(Color32F::TRANSPARENT, &[Rectangle::from_size(size)])?;
                    let _sync = frame.finish()?;
                }
                Ok(texture)
            };
            let front = create()?;
            let back = create()?;
            states.insert(state.name.clone(), PingPong { front, back, size });
            unchanged = false;
        }

        Ok(unchanged)
    }

    fn run_pass(
        &self,
        renderer: &mut GlesRenderer,
        pass: &PipelinePass,
        compiled: &CompiledShader,
        sources: &Sources,
        scale: Scale<f64>,
    ) -> anyhow::Result<()> {
        let mut states = self.states.borrow_mut();

        // Checked when parsing the config.
        let target = pass.target.as_deref().unwrap();
        let size = states[target].size;

        // A pass gets its target as `tex` unless it says otherwise, and renders the next value of
        // the target.
        let input = pass.input.as_deref().unwrap_or(target);
        let mut textures = HashMap::new();
        if let Some(texture) = sources.resolve(input, &states) {
            textures.insert(String::from("tex"), texture);
        }
        textures.extend(pass_textures(pass, sources, &states));

        let geo_size = size.to_f64().to_logical(1.);
        let mut uniforms = context_uniforms(size, geo_size, scale.x as f32);
        uniforms.extend(self.sim.uniforms(pass.repeat));
        uniforms.extend(config_uniforms(pass, self.sim.time));

        let elem = ShaderRenderElement::new(
            ProgramType::Window(compiled.key),
            geo_size,
            None,
            1.,
            1.,
            Rc::from(uniforms),
            textures,
            Kind::Unspecified,
        );

        let state = states.get_mut(target).unwrap();
        let full = Rectangle::from_size(size);
        {
            let mut target = renderer.bind(&mut state.back)?;
            let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
            frame.clear(Color32F::TRANSPARENT, &[full])?;
            RenderElement::<GlesRenderer>::draw(
                &elem,
                &mut frame,
                elem.src(),
                full,
                &[full],
                &[],
                None,
            )?;
            let _sync = frame.finish()?;
        }
        std::mem::swap(&mut state.front, &mut state.back);

        Ok(())
    }
}

/// The textures that passes can read besides the states.
struct Sources<'a> {
    surface: &'a GlesTexture,
    backdrop: Option<&'a GlesTexture>,
}

impl Sources<'_> {
    fn resolve(&self, name: &str, states: &HashMap<String, PingPong>) -> Option<GlesTexture> {
        match name {
            SURFACE_TEXTURE => Some(self.surface.clone()),
            name if is_backdrop(name) => self.backdrop.cloned(),
            name => Some(states.get(name)?.front.clone()),
        }
    }
}

fn elements_key<E: RenderElement<GlesRenderer>>(
    elements: &[E],
    scale: Scale<f64>,
) -> Vec<SurfaceElementKey> {
    elements
        .iter()
        .map(|elem| {
            (
                elem.id().clone(),
                elem.current_commit(),
                elem.geometry(scale),
                elem.alpha().to_bits(),
            )
        })
        .collect()
}

/// Renders the elements to a texture of exactly `size_px`, kept in `cell` between frames.
fn render_exact(
    cell: &RefCell<Option<GlesTexture>>,
    renderer: &mut GlesRenderer,
    elements: &[impl RenderElement<GlesRenderer>],
    size_px: Size<i32, Physical>,
    scale: Scale<f64>,
) -> anyhow::Result<GlesTexture> {
    let mut cell = cell.borrow_mut();
    let texture = match &mut *cell {
        Some(texture) if texture_size(texture) == size_px => texture,
        _ => {
            let buffer_size = size_px.to_logical(1).to_buffer(1, Transform::Normal);
            let texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, buffer_size)
                .context("error creating the surface texture")?;
            cell.insert(texture)
        }
    };

    let full = Rectangle::from_size(size_px);
    {
        let mut target = renderer.bind(texture)?;
        let mut frame = renderer.render(&mut target, size_px, Transform::Normal)?;
        frame.clear(Color32F::TRANSPARENT, &[full])?;
        // Elements come front to back.
        for elem in elements.iter().rev() {
            let dst = elem.geometry(scale);
            let damage = [Rectangle::from_size(dst.size)];
            elem.draw(&mut frame, elem.src(), dst, &damage, &[], None)?;
        }
        let _sync = frame.finish()?;
    }

    Ok(texture.clone())
}

fn texture_size(texture: &GlesTexture) -> Size<i32, Physical> {
    use smithay::backend::renderer::Texture as _;
    let size = texture.size();
    Size::from((size.w, size.h))
}

fn compile_pass(
    renderer: &mut GlesRenderer,
    pass: &PipelinePass,
) -> anyhow::Result<CompiledShader> {
    let name = pass.source.name();
    let code = match &pass.source {
        ShaderSource::File { code, .. } => code.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?,
        ShaderSource::Inline(code) => code,
        ShaderSource::Preset(_) => anyhow::bail!("presets can't be pipeline passes"),
    };
    // The error with the compiler log is logged once by compile().
    window_shader::compile(renderer, &name, code, ShaderInput::Window)
        .map_err(|_| anyhow::anyhow!("error compiling {name}"))
}

fn pass_textures(
    pass: &PipelinePass,
    sources: &Sources,
    states: &HashMap<String, PingPong>,
) -> Vec<(String, GlesTexture)> {
    pass.textures
        .iter()
        .filter_map(|(name, from)| Some((name.clone(), sources.resolve(from, states)?)))
        .collect()
}

/// The uniforms set in the config, which go last so that they win over niri's.
fn config_uniforms(pass: &PipelinePass, time: f32) -> Vec<Uniform<'static>> {
    let values = pass
        .uniforms
        .iter()
        .map(|(name, values)| match *values.as_slice() {
            [x] => Uniform::new(name.clone(), x),
            [x, y] => Uniform::new(name.clone(), [x, y]),
            [x, y, z] => Uniform::new(name.clone(), [x, y, z]),
            [x, y, z, w, ..] => Uniform::new(name.clone(), [x, y, z, w]),
            [] => Uniform::new(name.clone(), 0f32),
        });
    let clock = pass
        .clock_uniforms
        .iter()
        .map(move |(name, rate)| Uniform::new(name.clone(), time * rate));
    values.chain(clock).collect()
}

/// Uniforms for a ShojiWM-format pass over a whole texture of `size_px`.
pub fn context_uniforms(
    size_px: Size<i32, Physical>,
    geo_size: Size<f64, Logical>,
    scale: f32,
) -> Vec<Uniform<'static>> {
    let rect = [0., 0., size_px.w as f32, size_px.h as f32];
    vec![
        mat3_uniform("niri_input_to_geo", Mat3::IDENTITY),
        mat3_uniform("niri_geo_to_tex", Mat3::IDENTITY),
        Uniform::new("niri_geo_size", [geo_size.w as f32, geo_size.h as f32]),
        Uniform::new("niri_shoji_texture_size", [rect[2], rect[3]]),
        Uniform::new("niri_shoji_content_rect", rect),
        Uniform::new("niri_shoji_frame_rect", rect),
        Uniform::new("effect_frame_radius_phy_px", [0f32; 4]),
        Uniform::new("effect_frame_border_phy_px", [0f32; 4]),
        Uniform::new("effect_clip_rect_phy_px", [0f32; 4]),
        Uniform::new("effect_clip_radius_phy_px", [0f32; 4]),
        Uniform::new("effect_has_clip", 0f32),
        Uniform::new("effect_scale", scale),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(windows: Vec<(u64, Rectangle<f64, Logical>)>) -> Inputs {
        Inputs {
            windows,
            pointer: None,
            output_size: Size::from((1000., 500.)),
        }
    }

    #[test]
    fn windows_splash_when_they_come_and_go() {
        let mut sim = Simulation::default();
        let rect = Rectangle::new(Point::from((100., 100.)), Size::from((200., 100.)));

        // Windows that are there from the start don't splash.
        sim.update(Duration::from_millis(100), &inputs(vec![(1, rect)]));
        assert_eq!(sim.impulse_power, [0.; MAX_IMPULSES]);
        assert!(!sim.is_active());

        sim.update(
            Duration::from_millis(116),
            &inputs(vec![(1, rect), (2, rect)]),
        );
        assert_eq!(sim.impulse_power[0], ENTER_POWER);
        assert_eq!(sim.impulses[0][..2], [0.2, 0.3]);
        assert!(sim.is_active());

        sim.update(Duration::from_millis(132), &inputs(vec![(2, rect)]));
        assert_eq!(sim.impulse_power[1], EXIT_POWER);
    }

    #[test]
    fn moving_windows_have_motion_and_settle() {
        let mut sim = Simulation::default();
        let at = |x| Rectangle::new(Point::from((x, 0.)), Size::from((100., 100.)));

        sim.update(Duration::from_millis(0), &inputs(vec![(1, at(0.))]));
        sim.update(Duration::from_millis(100), &inputs(vec![(1, at(100.))]));
        // 100 px of 1000 in 0.1 s.
        assert!((sim.windows[0].motion[0] - 1.).abs() < 1e-4);
        assert!(sim.is_active());

        sim.update(Duration::from_millis(20_000), &inputs(vec![(1, at(100.))]));
        assert_eq!(sim.presence, 0.);
        assert!(!sim.is_active());
    }

    #[test]
    fn dt_is_split_between_repeats() {
        let mut sim = Simulation::default();
        sim.update(Duration::from_millis(0), &inputs(vec![]));
        sim.update(Duration::from_millis(16), &inputs(vec![]));
        let uniforms = sim.uniforms(4);
        let dt = uniforms.iter().find(|u| u.name == "dt").unwrap();
        assert_eq!(
            dt.value,
            smithay::backend::renderer::gles::UniformValue::_1f(0.004)
        );
    }
}
