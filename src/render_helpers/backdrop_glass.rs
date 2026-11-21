//! Liquid glass over what's really under a rectangle, like the title bars of floating windows.
//!
//! The element takes the framebuffer under it, blurs it, and bends it with the lens of the true
//! liquid glass along the edges of a rounded rect.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use niri_config::{CornerRadius, ShaderInput};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::backend::renderer::gles::{GlesRenderer, Uniform};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::utils::{Logical, Point, Rectangle, Scale, Transform};

use super::blur::BlurOptions;
use super::shader_element::{FramebufferBackdrop, ShaderRenderElement};
use super::shader_pipeline::context_uniforms;
use super::shaders::ProgramType;
use super::window_shader::{self, push_uniform_bits};

const TITLEBAR_GLASS: &str = include_str!("../../resources/shaders/titlebar-glass.frag");

/// What the glass looks like.
#[derive(Debug, Clone, Copy)]
pub struct GlassStyle {
    pub rim_width: f32,
    pub refraction: f32,
    pub chromatic_shift: f32,
    pub highlight: f32,
    /// RGB and amount of the tint.
    pub tint: [f32; 4],
}

#[derive(Debug, Default)]
pub struct BackdropGlass {
    /// Id of the element with its commit and what it was drawn with, so that the damage tracker
    /// keeps what the element captured while nothing changes.
    last: RefCell<Option<(Id, CommitCounter, Vec<u32>)>>,
}

impl BackdropGlass {
    /// Returns the glass over `area`, with the glass itself going on `open_bottom` past the bottom
    /// of the area, where the lens doesn't bend.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        renderer: &mut GlesRenderer,
        area: Rectangle<f64, Logical>,
        radius: CornerRadius,
        open_bottom: f64,
        style: GlassStyle,
        blur: BlurOptions,
        scale: Scale<f64>,
    ) -> Option<ShaderRenderElement> {
        let size_px = area.size.to_physical_precise_round(scale);
        if size_px.w <= 0 || size_px.h <= 0 {
            return None;
        }

        // The error is logged once by compile().
        let shader = window_shader::compile(
            renderer,
            "titlebar-glass",
            TITLEBAR_GLASS,
            ShaderInput::Window,
        )
        .ok()?;

        let s = scale.x as f32;
        let glass_rect = [
            0.,
            0.,
            size_px.w as f32,
            size_px.h as f32 + (open_bottom * scale.y) as f32,
        ];
        let glass_radius = [
            radius.top_left * s,
            radius.top_right * s,
            radius.bottom_right * s,
            radius.bottom_left * s,
        ];
        let mut uniforms = context_uniforms(size_px, area.size, s);
        uniforms.extend([
            Uniform::new("glass_rect", glass_rect),
            Uniform::new("glass_radius", glass_radius),
            Uniform::new("rim_width_px", style.rim_width * s),
            Uniform::new("refraction_px", style.refraction * s),
            Uniform::new("chromatic_shift_px", style.chromatic_shift * s),
            Uniform::new("highlight_strength", style.highlight),
            Uniform::new("edge_softness_px", 2. * s),
            Uniform::new("glass_tint", style.tint),
            Uniform::new("center_frost", 0.6f32),
            Uniform::new("edge_frost", 0.95f32),
        ]);

        let mut bits = vec![(shader.key >> 32) as u32, shader.key as u32];
        for uniform in &uniforms {
            push_uniform_bits(&mut bits, uniform);
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
            ProgramType::Window(shader.key),
            area.size,
            None,
            s,
            1.,
            Rc::from(uniforms),
            HashMap::new(),
            Kind::Unspecified,
        )
        .with_id(id, commit)
        .with_location(area.loc)
        .with_framebuffer_backdrop(FramebufferBackdrop {
            textures: vec![String::from("tex")],
            sharp_textures: vec![String::from("sharp_tex")],
            size: size_px.to_logical(1).to_buffer(1, Transform::Normal),
            blur: Some(blur),
        });
        Some(elem)
    }
}

/// Moves `area` to whole physical pixels, like the title bar texture over the glass.
pub fn round_area(area: Rectangle<f64, Logical>, scale: Scale<f64>) -> Rectangle<f64, Logical> {
    let loc: Point<f64, Logical> = area.loc.to_physical_precise_round(scale).to_logical(scale);
    Rectangle::new(loc, area.size)
}
