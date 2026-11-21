use niri_config::utils::MergeWith as _;
use niri_config::{Config, LayerRule, ShaderPipeline};
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::element::Kind;
use smithay::desktop::{LayerSurface, PopupKind, PopupManager};
use smithay::utils::{Logical, Point, Rectangle, Scale, Size};
use smithay::wayland::compositor::{remove_pre_commit_hook, HookId};
use smithay::wayland::shell::wlr_layer::{ExclusiveZone, Layer};

use super::ResolvedLayerRules;
use crate::animation::Clock;
use crate::layout::shadow::Shadow;
use crate::niri_render_elements;
use crate::render_helpers::background_effect::BackgroundEffectElement;
use crate::render_helpers::blur::BlurOptions;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shader_pipeline::{self, SurfacePipeline};
use crate::render_helpers::shadow::ShadowRenderElement;
use crate::render_helpers::solid_color::{SolidColorBuffer, SolidColorRenderElement};
use crate::render_helpers::surface::push_elements_from_surface_tree;
use crate::render_helpers::window_shader::Backdrop;
use crate::render_helpers::xray::XrayPos;
use crate::render_helpers::{background_effect, RenderCtx};
use crate::utils::{baba_is_float_offset, round_logical_in_physical};

#[derive(Debug)]
pub struct MappedLayer {
    /// The surface itself.
    surface: LayerSurface,

    /// Pre-commit hook that we have on all mapped layer surfaces.
    pre_commit_hook: HookId,

    /// Up-to-date rules.
    rules: ResolvedLayerRules,

    /// Whether to recompute layer rules on the next commit.
    ///
    /// Set in the pre-commit hook when the layer changes; consumed in the commit handler.
    recompute_rules_on_commit: bool,

    /// Buffer to draw instead of the surface when it should be blocked out.
    block_out_buffer: SolidColorBuffer,

    /// The shadow around the surface.
    shadow: Shadow,

    /// The blur config, passed for background effect rendering.
    blur_config: niri_config::Blur,

    /// The view size for the layer surface's output.
    view_size: Size<f64, Logical>,

    /// Scale of the output the layer surface is on (and rounds its sizes to).
    scale: f64,

    /// Clock for driving animations.
    clock: Clock,

    /// State of the shader pipeline, if the rules give the surface one.
    pipeline: Option<SurfacePipeline>,
}

niri_render_elements! {
    LayerSurfaceRenderElement<R> => {
        Wayland = WaylandSurfaceRenderElement<R>,
        SolidColor = SolidColorRenderElement,
        Shadow = ShadowRenderElement,
        BackgroundEffect = BackgroundEffectElement,
        Shader = ShaderRenderElement,
    }
}

impl MappedLayer {
    pub fn new(
        surface: LayerSurface,
        pre_commit_hook: HookId,
        rules: ResolvedLayerRules,
        view_size: Size<f64, Logical>,
        scale: f64,
        clock: Clock,
        config: &Config,
    ) -> Self {
        let mut shadow_config = config.layout.shadow;
        // Shadows for layer surfaces need to be explicitly enabled.
        shadow_config.on = false;
        shadow_config.merge_with(&rules.shadow);

        Self {
            surface,
            pre_commit_hook,
            recompute_rules_on_commit: false,
            block_out_buffer: SolidColorBuffer::new((0., 0.), [0., 0., 0., 1.]),
            view_size,
            scale,
            shadow: Shadow::new(shadow_config),
            blur_config: config.blur,
            pipeline: rules
                .shader_pipeline
                .as_ref()
                .map(|_| SurfacePipeline::default()),
            rules,
            clock,
        }
    }

    pub fn update_config(&mut self, config: &Config) {
        let mut shadow_config = config.layout.shadow;
        // Shadows for layer surfaces need to be explicitly enabled.
        shadow_config.on = false;
        shadow_config.merge_with(&self.rules.shadow);
        self.shadow.update_config(shadow_config);

        self.blur_config = config.blur;
    }

    pub fn update_shaders(&mut self) {
        self.shadow.update_shaders();
    }

    pub fn update_sizes(&mut self, view_size: Size<f64, Logical>, scale: f64) {
        self.view_size = view_size;
        self.scale = scale;
    }

    pub fn update_render_elements(&mut self, size: Size<f64, Logical>) {
        // Round to physical pixels.
        let size = size
            .to_physical_precise_round(self.scale)
            .to_logical(self.scale);

        self.block_out_buffer.resize(size);

        let radius = self.rules.geometry_corner_radius.unwrap_or_default();
        // FIXME: is_active based on keyboard focus?
        self.shadow
            .update_render_elements(size, true, radius, self.scale, 1.);
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.rules.baba_is_float
            || self
                .pipeline
                .as_ref()
                .is_some_and(SurfacePipeline::needs_redraws)
    }

    /// Updates what the shader pipeline knows about the output, once a frame.
    pub fn update_pipeline(&mut self, inputs: &shader_pipeline::Inputs) {
        if let Some(pipeline) = &mut self.pipeline {
            pipeline.update(self.clock.now(), inputs);
        }
    }

    /// Drops a stone into the shader pipeline at a point in output-local coordinates.
    pub fn drop_into_pipeline(
        &mut self,
        pos: Point<f64, Logical>,
        output_size: Size<f64, Logical>,
    ) -> bool {
        match &mut self.pipeline {
            Some(pipeline) => {
                pipeline.drop_at(pos, output_size);
                true
            }
            None => false,
        }
    }

    pub fn surface(&self) -> &LayerSurface {
        &self.surface
    }

    pub fn rules(&self) -> &ResolvedLayerRules {
        &self.rules
    }

    /// Recomputes the resolved layer rules and returns whether they changed.
    pub fn recompute_layer_rules(&mut self, rules: &[LayerRule], is_at_startup: bool) -> bool {
        let new_rules = ResolvedLayerRules::compute(rules, &self.surface, is_at_startup);
        if new_rules == self.rules {
            return false;
        }

        self.rules = new_rules;
        match (&self.rules.shader_pipeline, &self.pipeline) {
            (Some(_), None) => self.pipeline = Some(SurfacePipeline::default()),
            (None, Some(_)) => self.pipeline = None,
            _ => (),
        }
        true
    }

    pub fn set_recompute_rules_on_commit(&mut self) {
        self.recompute_rules_on_commit = true;
    }

    pub fn take_recompute_rules_on_commit(&mut self) -> bool {
        std::mem::take(&mut self.recompute_rules_on_commit)
    }

    pub fn place_within_backdrop(&self) -> bool {
        if !self.rules.place_within_backdrop {
            return false;
        }

        if self.surface.layer() != Layer::Background {
            return false;
        }

        let state = self.surface.cached_state();
        if state.exclusive_zone != ExclusiveZone::DontCare {
            return false;
        }

        true
    }

    pub fn bob_offset(&self) -> Point<f64, Logical> {
        if !self.rules.baba_is_float {
            return Point::from((0., 0.));
        }

        let y = baba_is_float_offset(self.clock.now_unadjusted(), self.view_size.h);
        let y = round_logical_in_physical(self.scale, y);
        Point::from((0., y))
    }

    pub fn render_normal<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        if ctx.target.should_hide(self.rules.block_out_from) {
            return;
        }

        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);

        let bob_offset = self.bob_offset();
        let location = location + bob_offset;
        let xray_pos = xray_pos.offset(bob_offset);

        let surface = self.surface.wl_surface();

        let should_block_out = ctx.target.should_block_out(self.rules.block_out_from);
        if should_block_out {
            // Round to physical pixels.
            let location = location.to_physical_precise_round(scale).to_logical(scale);

            // FIXME: take geometry-corner-radius into account.
            let elem = SolidColorRenderElement::from_buffer(
                &self.block_out_buffer,
                location,
                alpha,
                Kind::Unspecified,
            );
            push(elem.into());
        } else {
            let pipeline = self.render_pipeline(ctx.r(), ns, location, xray_pos, alpha);

            // The pipeline draws instead of the surface, or behind it.
            let replaced = pipeline.as_ref().is_some_and(|(_, behind)| !behind);
            if !replaced {
                // Layer surfaces don't have extra geometry like windows.
                let buf_pos = location;

                push_elements_from_surface_tree(
                    ctx.renderer,
                    surface,
                    buf_pos.to_physical_precise_round(scale),
                    scale,
                    alpha,
                    Kind::ScanoutCandidate,
                    &mut |elem| push(elem.into()),
                );
            }
            if let Some((elem, _)) = pipeline {
                push(elem.into());
            }
        }

        let location = location.to_physical_precise_round(scale).to_logical(scale);
        self.shadow
            .render(ctx.renderer, location, &mut |elem| push(elem.into()));

        // A pipeline that reads the backdrop draws it itself.
        if self.pipeline_uses_backdrop() && !should_block_out {
            return;
        }

        let geometry = Rectangle::new(location, self.block_out_buffer.size());
        let surface_off = Point::new(0., 0.); // No geometry on layer surfaces.
        let surface_anim_scale = Scale::from(1.);
        let radius = self.rules.geometry_corner_radius.unwrap_or_default();
        background_effect::render_for_tile(
            ctx.as_gles(),
            ns,
            geometry,
            self.scale,
            false,
            surface,
            surface_off,
            surface_anim_scale,
            self.blur_config,
            radius,
            self.rules.background_effect,
            should_block_out,
            xray_pos,
            &mut |elem| push(elem.into()),
        );
    }

    fn pipeline_uses_backdrop(&self) -> bool {
        self.pipeline.is_some()
            && self
                .rules
                .shader_pipeline
                .as_ref()
                .is_some_and(ShaderPipeline::uses_backdrop)
    }

    /// Renders the surface through its shader pipeline, and returns the element with whether it
    /// goes behind the surface.
    ///
    /// Returns `None` when there's no pipeline, it has nothing to show right now, or it fails, in
    /// which case the surface should be rendered as usual.
    fn render_pipeline<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        alpha: f32,
    ) -> Option<(ShaderRenderElement, bool)> {
        let (Some(pipeline), Some(config)) = (&self.pipeline, &self.rules.shader_pipeline) else {
            return None;
        };

        let scale = Scale::from(self.scale);
        let size = self.block_out_buffer.size();
        let mut ctx = ctx.as_gles();

        let mut elements = Vec::new();
        push_elements_from_surface_tree(
            ctx.renderer,
            self.surface.wl_surface(),
            Point::from((0, 0)),
            scale,
            1.,
            Kind::Unspecified,
            &mut |elem| elements.push(elem),
        );

        // Top and overlay surfaces are over windows, which xray doesn't show. Their pipelines take
        // the real backdrop from the framebuffer, like floating windows, and blur it themselves,
        // only where they draw.
        let effect = self.rules.background_effect;
        let from_framebuffer = config.uses_backdrop()
            && effect.xray.is_none()
            && matches!(self.surface.layer(), Layer::Top | Layer::Overlay);
        let backdrop_blur =
            (from_framebuffer && effect.blur == Some(true) && !self.blur_config.off)
                .then(|| BlurOptions::from(self.blur_config));

        // What's behind the surface, with the surface at (0, 0).
        let mut backdrop = Vec::new();
        if config.uses_backdrop() && !from_framebuffer {
            background_effect::render_for_tile(
                ctx.r(),
                ns,
                Rectangle::from_size(size),
                self.scale,
                false,
                self.surface.wl_surface(),
                Point::new(0., 0.),
                Scale::from(1.),
                self.blur_config,
                self.rules.geometry_corner_radius.unwrap_or_default(),
                self.rules.background_effect,
                false,
                xray_pos,
                &mut |elem| backdrop.push(elem),
            );
        }

        let location = location.to_physical_precise_round(scale).to_logical(scale);
        let res = pipeline.render(
            ctx.renderer,
            config,
            &elements,
            if from_framebuffer {
                Backdrop::Framebuffer
            } else if config.uses_backdrop() {
                Backdrop::Elements(&backdrop[..])
            } else {
                Backdrop::None
            },
            backdrop_blur,
            size,
            scale,
            location,
            alpha,
            self.clock.now(),
        );
        match res {
            Ok(elem) => elem.map(|elem| (elem, config.behind)),
            Err(err) => {
                warn!("error rendering the layer shader pipeline: {err:?}");
                None
            }
        }
    }

    pub fn render_popups<R: NiriRenderer>(
        &self,
        mut ctx: RenderCtx<R>,
        ns: Option<usize>,
        location: Point<f64, Logical>,
        xray_pos: XrayPos,
        push: &mut dyn FnMut(LayerSurfaceRenderElement<R>),
    ) {
        if ctx.target.should_block_out(self.rules.block_out_from) {
            return;
        }

        let scale = Scale::from(self.scale);
        let alpha = self.rules.opacity.unwrap_or(1.).clamp(0., 1.);

        let bob_offset = self.bob_offset();
        let location = location + bob_offset;
        let xray_pos = xray_pos.offset(bob_offset);

        let surface = self.surface.wl_surface();
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let popup_rules = match popup {
                PopupKind::Xdg(_) => self.rules.popups,
                // IME popups aren't affected by rules for regular popups.
                PopupKind::InputMethod(_) => niri_config::ResolvedPopupsRules::default(),
            };
            let alpha = match popup {
                PopupKind::Xdg(_) => alpha * popup_rules.opacity.unwrap_or(1.).clamp(0., 1.),
                // IME popups aren't affected by the layer opacity.
                PopupKind::InputMethod(_) => 1.,
            };

            let surface = popup.wl_surface();
            let popup_geo = popup.geometry();
            let surface_loc = location + (offset - popup_geo.loc).to_f64();

            push_elements_from_surface_tree(
                ctx.renderer,
                surface,
                surface_loc.to_physical_precise_round(scale),
                scale,
                alpha,
                Kind::ScanoutCandidate,
                &mut |elem| push(elem.into()),
            );

            let geometry = Rectangle::new(location + offset.to_f64(), popup_geo.size.to_f64());
            let surface_off = popup_geo.loc.upscale(-1).to_f64();
            let surface_anim_scale = Scale::from(1.);
            let mut effect = popup_rules.background_effect;
            // Default xray to false for pop-ups since they're always on top of something.
            if effect.xray.is_none() {
                effect.xray = Some(false);
            }
            let xray_pos = xray_pos.offset(offset.to_f64());
            background_effect::render_for_tile(
                ctx.as_gles(),
                ns,
                geometry,
                self.scale,
                false,
                surface,
                surface_off,
                surface_anim_scale,
                self.blur_config,
                popup_rules.geometry_corner_radius.unwrap_or_default(),
                effect,
                false,
                xray_pos,
                &mut |elem| push(elem.into()),
            );
        }
    }
}

impl Drop for MappedLayer {
    fn drop(&mut self) {
        remove_pre_commit_hook(self.surface.wl_surface(), &self.pre_commit_hook);
    }
}
