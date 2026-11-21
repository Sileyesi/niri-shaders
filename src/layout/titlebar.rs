//! Title bars of floating windows, with the window buttons on the left and the title in the
//! middle, like on macOS.

use std::cell::RefCell;

use niri_config::CornerRadius;
use pangocairo::cairo::{self, ImageSurface};
use pangocairo::pango::{Alignment, EllipsizeMode, FontDescription, Weight};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::{Logical, Point, Size, Transform};

use crate::render_helpers::primary_gpu_texture::PrimaryGpuTextureRenderElement;
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::texture::{TextureBuffer, TextureRenderElement};

/// Height of the title bar.
pub const TITLEBAR_HEIGHT: f64 = 32.;

/// Diameter of a window button.
const BUTTON_SIZE: f64 = 12.;

/// Distance between the centers of the window buttons, which is also the width of their hit areas.
const BUTTON_STEP: f64 = 20.;

/// Center of the first window button from the left edge.
const FIRST_BUTTON_X: f64 = 20.;

const FONT: &str = "sans";
const FONT_SIZE: f64 = 13.;

const CLOSE_COLOR: (f64, f64, f64) = (1.0, 0.373, 0.341);
const MINIMIZE_COLOR: (f64, f64, f64) = (0.996, 0.737, 0.180);
const MAXIMIZE_COLOR: (f64, f64, f64) = (0.157, 0.784, 0.251);

/// Part of the title bar under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TitleBarPart {
    /// The title, which moves the window.
    Title,
    Minimize,
    Maximize,
    Close,
}

/// What the title bar looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleBarState {
    pub title: String,
    pub active: bool,
    pub maximized: bool,
    pub hovered: Option<TitleBarPart>,
    /// Radius of the top corners.
    pub radius: CornerRadius,
}

/// Draws the title bar of a tile, and keeps the texture while it doesn't change.
#[derive(Debug, Default)]
pub struct TitleBar {
    cached: RefCell<Option<(Key, TextureBuffer<GlesTexture>)>>,
}

#[derive(Debug, PartialEq)]
struct Key {
    state: TitleBarState,
    width: i32,
    scale: u64,
}

impl TitleBarPart {
    /// The part at `pos`, relative to the title bar of this width.
    pub fn at(pos: Point<f64, Logical>, width: f64) -> Option<Self> {
        if pos.x < 0. || pos.y < 0. || width < pos.x || TITLEBAR_HEIGHT <= pos.y {
            return None;
        }

        // The buttons go close, minimize, maximize from the left.
        let button = ((pos.x - (FIRST_BUTTON_X - BUTTON_STEP / 2.)) / BUTTON_STEP).floor();
        Some(match button {
            0. => Self::Close,
            1. => Self::Minimize,
            2. => Self::Maximize,
            _ => Self::Title,
        })
    }
}

impl TitleBar {
    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        state: &TitleBarState,
        width: f64,
        scale: f64,
        location: Point<f64, Logical>,
        alpha: f32,
    ) -> Option<PrimaryGpuTextureRenderElement> {
        let size =
            Size::<f64, Logical>::from((width, TITLEBAR_HEIGHT)).to_physical_precise_round(scale);
        if size.w <= 0 || size.h <= 0 {
            return None;
        }

        let key = Key {
            state: state.clone(),
            width: size.w,
            scale: scale.to_bits(),
        };

        let mut cached = self.cached.borrow_mut();
        if cached.as_ref().is_none_or(|(k, _)| *k != key) {
            *cached = None;
            match draw(state, size.w, size.h, scale) {
                Ok(data) => {
                    match TextureBuffer::from_memory(
                        renderer.as_gles_renderer(),
                        &data,
                        Fourcc::Argb8888,
                        (size.w, size.h),
                        false,
                        scale,
                        Transform::Normal,
                        Vec::new(),
                    ) {
                        Ok(buffer) => *cached = Some((key, buffer)),
                        Err(err) => warn!("error uploading the title bar: {err:?}"),
                    }
                }
                Err(err) => warn!("error drawing the title bar: {err:?}"),
            }
        }

        let (_, buffer) = cached.as_ref()?;
        let elem = TextureRenderElement::from_texture_buffer(
            buffer.clone(),
            location,
            alpha,
            None,
            None,
            Kind::Unspecified,
        );
        Some(PrimaryGpuTextureRenderElement(elem))
    }
}

/// Draws the title bar in physical pixels, and returns the ARGB data.
fn draw(state: &TitleBarState, w: i32, h: i32, scale: f64) -> anyhow::Result<Vec<u8>> {
    let surface = ImageSurface::create(cairo::Format::ARgb32, w, h)?;
    let cr = cairo::Context::new(&surface)?;
    let (wf, hf) = (f64::from(w), f64::from(h));
    let pi = std::f64::consts::PI;

    // The background, with the top corners rounded like the window.
    let tl = f64::from(state.radius.top_left) * scale;
    let tr = f64::from(state.radius.top_right) * scale;
    cr.new_path();
    cr.move_to(0., hf);
    cr.line_to(0., tl);
    if tl > 0. {
        cr.arc(tl, tl, tl, pi, 1.5 * pi);
    }
    cr.line_to(wf - tr, 0.);
    if tr > 0. {
        cr.arc(wf - tr, tr, tr, 1.5 * pi, 2. * pi);
    }
    cr.line_to(wf, hf);
    cr.close_path();
    // The liquid glass under the title bar shows through: this only darkens it a little for the
    // title, and the glass lights its own edge.
    if state.active {
        cr.set_source_rgba(0.08, 0.09, 0.12, 0.16);
    } else {
        cr.set_source_rgba(0.08, 0.08, 0.10, 0.30);
    }
    cr.fill_preserve()?;
    cr.clip();

    // A soft sheen at the top and a thin line where the window starts.
    let line = scale.round().max(1.);
    let highlight = cairo::LinearGradient::new(0., 0., 0., hf);
    highlight.add_color_stop_rgba(0., 1., 1., 1., 0.07);
    highlight.add_color_stop_rgba(0.6, 1., 1., 1., 0.);
    cr.set_source(&highlight)?;
    cr.paint()?;
    cr.set_source_rgba(0., 0., 0., 0.22);
    cr.rectangle(0., hf - line, wf, line);
    cr.fill()?;

    // The buttons. They have their colors on the active window, or when the pointer is on them.
    let hovered = matches!(
        state.hovered,
        Some(TitleBarPart::Close | TitleBarPart::Minimize | TitleBarPart::Maximize)
    );
    let lit = state.active || hovered;
    let radius = BUTTON_SIZE / 2. * scale;
    let cy = hf / 2.;
    let center = |i: f64| ((FIRST_BUTTON_X + BUTTON_STEP * i) * scale).round();
    let buttons = [
        (CLOSE_COLOR, lit),
        (MINIMIZE_COLOR, lit),
        (MAXIMIZE_COLOR, lit),
    ];
    for (i, (color, colored)) in buttons.into_iter().enumerate() {
        let cx = center(i as f64);
        cr.new_path();
        cr.arc(cx, cy, radius, 0., 2. * pi);
        if colored {
            cr.set_source_rgb(color.0, color.1, color.2);
        } else {
            cr.set_source_rgba(1., 1., 1., 0.16);
        }
        cr.fill_preserve()?;
        cr.set_source_rgba(0., 0., 0., 0.18);
        cr.set_line_width(line * 0.5);
        cr.stroke()?;
    }

    // Glyphs on hover, like on macOS.
    if hovered {
        let glyph = 0.28 * BUTTON_SIZE * scale;
        cr.set_source_rgba(0., 0., 0., 0.55);
        cr.set_line_width((1.1 * scale).max(1.));
        cr.set_line_cap(cairo::LineCap::Round);

        let cx = center(0.);
        cr.move_to(cx - glyph, cy - glyph);
        cr.line_to(cx + glyph, cy + glyph);
        cr.move_to(cx + glyph, cy - glyph);
        cr.line_to(cx - glyph, cy + glyph);
        cr.stroke()?;

        // A dash.
        let cx = center(1.);
        cr.move_to(cx - glyph * 1.2, cy);
        cr.line_to(cx + glyph * 1.2, cy);
        cr.stroke()?;

        // Two triangles in opposite corners, pointing out, or in when the window fills the
        // screen.
        let cx = center(2.);
        let g = glyph * 1.15;
        let (near, far) = if state.maximized {
            (0.15, 1.)
        } else {
            (1., 0.15)
        };
        for sign in [-1., 1.] {
            let corner = (cx + sign * g * near, cy + sign * g * near);
            cr.move_to(corner.0, corner.1);
            cr.line_to(cx + sign * g * far, cy + sign * g * near);
            cr.line_to(cx + sign * g * near, cy + sign * g * far);
            cr.close_path();
        }
        cr.fill()?;
    }

    // The title, in the middle, clear of the buttons on both sides.
    let side = ((FIRST_BUTTON_X + BUTTON_STEP * 3.) * scale).round();
    let max_width = wf - side * 2.;
    if max_width > 0. && !state.title.is_empty() {
        let mut font = FontDescription::from_string(FONT);
        font.set_absolute_size(FONT_SIZE * scale * f64::from(pangocairo::pango::SCALE));
        font.set_weight(Weight::Semibold);

        let layout = pangocairo::functions::create_layout(&cr);
        layout.context().set_round_glyph_positions(false);
        layout.set_font_description(Some(&font));
        layout.set_ellipsize(EllipsizeMode::End);
        layout.set_alignment(Alignment::Center);
        layout.set_width((max_width * f64::from(pangocairo::pango::SCALE)) as i32);
        layout.set_text(&state.title);

        let (_, text_h) = layout.pixel_size();
        let y = ((hf - f64::from(text_h)) / 2.).round();
        // A shadow under the title keeps it readable over a bright backdrop.
        cr.move_to(side, y + line);
        cr.set_source_rgba(0., 0., 0., 0.35);
        pangocairo::functions::show_layout(&cr, &layout);
        cr.move_to(side, y);
        let alpha = if state.active { 0.95 } else { 0.55 };
        cr.set_source_rgba(1., 1., 1., alpha);
        pangocairo::functions::show_layout(&cr, &layout);
    }

    drop(cr);
    let data = surface.take_data()?.to_vec();
    Ok(data)
}
