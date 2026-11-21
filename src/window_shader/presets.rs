//! Shaders that come with niri and can be used as `shader "preset:name"`.

/// A shader built into niri.
#[derive(Debug)]
pub struct Preset {
    pub name: &'static str,
    pub code: &'static str,
    /// Defaults of the shader's own uniforms, which the config can override.
    pub uniforms: &'static [(&'static str, &'static [f32])],
    /// How far around the tile the shader draws.
    pub margin: f64,
    /// Whether the shader is made to go over a blurred background, which niri then turns on unless
    /// the window rules say otherwise.
    pub wants_blur: bool,
    /// Whether `cycle-window-shader` goes through it. Presets that only do something when the
    /// window closes are left out.
    pub cycle: bool,
}

macro_rules! preset {
    ($name:literal $(, $field:ident: $value:expr)* $(,)?) => {
        Preset {
            name: $name,
            code: include_str!(concat!("../../resources/shaders/", $name, ".frag")),
            $($field: $value,)*
            ..Preset::DEFAULT
        }
    };
}

impl Preset {
    const DEFAULT: Preset = Preset {
        name: "",
        code: "",
        uniforms: &[],
        margin: 0.,
        wants_blur: false,
        cycle: true,
    };
}

/// All presets, in the order that `cycle-window-shader` goes through them.
pub static PRESETS: &[Preset] = &[
    // The options of IslandGlassOptions in ShojiWM's default config, distances in physical pixels.
    preset!(
        "island-glass",
        uniforms: &[
            ("rim_width_px", &[80.]),
            ("refraction_px", &[80.]),
            ("chromatic_shift_px", &[1.9]),
            ("highlight_strength", &[0.6]),
            ("highlight_width_px", &[2.]),
            ("blur_mix", &[1.]),
            ("edge_softness_px", &[2.]),
            ("bevel_width_px", &[10.]),
            ("bevel_shadow", &[0.]),
            ("normal_smoothing_px", &[3.]),
            ("debug_view", &[0.]),
        ],
        wants_blur: true,
    ),
    preset!(
        "rain",
        uniforms: &[
            ("cell_px", &[60.]),
            ("period", &[2.4]),
            ("chance", &[0.72]),
            ("ring_speed", &[75.]),
            ("wavelength", &[12.]),
            ("refraction", &[5.]),
            ("glint", &[0.6]),
            ("tint", &[0.94]),
        ],
        wants_blur: true,
    ),
    preset!(
        "thaw",
        uniforms: &[
            ("crystal_px", &[140.]),
            ("refraction_px", &[70.]),
            ("thaw_seconds", &[1.2]),
        ],
        margin: 48.,
        wants_blur: true,
    ),
    preset!("crt", uniforms: &[("intensity", &[0.7]), ("tint", &[1., 1., 1.])]),
    preset!("hologram", uniforms: &[("color", &[0.3, 0.75, 1.])]),
    preset!(
        "neon-edge",
        uniforms: &[("width", &[3.]), ("glow", &[18.]), ("speed", &[0.25])],
        margin: 32.,
    ),
    preset!("glitch", uniforms: &[("strength", &[1.])]),
    preset!("matrix", uniforms: &[("density", &[0.7])]),
    preset!(
        "ripple",
        uniforms: &[("amplitude", &[5.]), ("wavelength", &[44.]), ("speed", &[160.])],
    ),
    preset!("dissolve", cycle: false),
];

pub fn find_preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.name == name)
}
