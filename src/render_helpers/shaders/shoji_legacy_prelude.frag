uniform sampler2D niri_tex;

// The older ShojiWM shader API, `vec4 shader_main(vec2 uv, vec2 rect_size)`. There `tex` is the
// input of the effect, which for a window is its backdrop, `uv` goes from 0 to 1 over the effect's
// rect and `rect_size` is that rect in physical pixels. niri gives such a shader what is behind the
// window (niri's blur, when the window has it) over the window frame, and draws the window on top.
uniform sampler2D tex;

// The texture with the backdrop is bigger than the frame, so map the coordinates.
vec4 niri_legacy_texture2D(sampler2D sampler, vec2 uv) {
    return texture2D(sampler, (niri_geo_to_backdrop * vec3(uv, 1.0)).xy);
}

#define texture2D(sampler, uv) niri_legacy_texture2D(sampler, uv)
