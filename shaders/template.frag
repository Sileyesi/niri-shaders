// Template for a niri window shader. Copy it, change window_color() and point a window rule at
// the copy:
//
//     window-rule {
//         match app-id="kitty"
//         shader "~/.config/niri/shaders/my-shader.frag" {
//             uniform "strength" 0.5
//         }
//     }
//
// The shader draws the whole tile: the window with its border, focus ring and shadow, as niri
// would draw them. Save the file and the window changes within a second. See docs/SHADERS.md.
//
// niri declares these before your code, you don't need to:
//
//   sampler2D niri_tex      The tile as niri would draw it. Colors are premultiplied by alpha.
//   mat3 niri_geo_to_tex    Turns coords_geo into texture coordinates for niri_tex.
//   vec2 niri_geo_size      Size of the tile in logical pixels, same as size_geo.xy.
//   vec2 niri_size          Size of the drawn area in physical pixels.
//   float niri_scale        Physical pixels per logical pixel.
//   float niri_alpha        Opacity that niri applies after your shader, you don't need it.
//   float niri_time         Seconds since niri started (wraps every hour). Using it makes niri
//                           redraw the window every frame.
//   float niri_focused      1.0 for the focused window, 0.0 for the others, eased in between.
//   vec2 niri_cursor        Pointer over the window, from (0, 0) at the top left corner to
//                           (1, 1) at the bottom right, or (-1, -1) when it's somewhere else.
//   float niri_close_progress  0.0, going to 1.0 while the window closes. Using it makes the
//                           shader draw the close animation instead of the default one.
//   float niri_random_seed  A random number from 0 to 1, different for every window.
//   vec4 niri_corner_radius Corner radii of the tile in logical pixels: top left, top right,
//                           bottom right, bottom left.
//   float niri_border_width Width of the border in logical pixels, 0 without one.
//
// Helpers:
//
//   vec4 niri_sample(vec3 coords_geo)                The tile color at a point, premultiplied.
//   float niri_tile_sdf(vec3 coords_geo, vec3 size_geo)
//                                                    Distance to the rounded edge of the tile in
//                                                    logical pixels, negative inside.
//   float shoji_rrect_sdf(vec2 p, vec4 rect, vec4 radius)
//                                                    Distance to any rounded rect.
//   float shoji_coverage(float sdf)                  Antialiased coverage of a distance.

// Your own parameters. Set them with `uniform "strength" 0.5` in the window rule. Use float,
// vec2, vec3 or vec4 for 1 to 4 numbers. Ones that the rule doesn't set are 0.
uniform float strength;

// coords_geo is the position of the pixel within the tile: (0, 0) is the top left corner and
// (1, 1) the bottom right one. With a `margin` in the rule, the shader can draw outside of that
// range too. size_geo.xy is the size of the tile in logical pixels.
//
// Return the color of the pixel, premultiplied by alpha.
vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec4 color = niri_sample(coords_geo);

    // Example: fade the colors toward gray on windows out of focus.
    float gray = dot(color.rgb, vec3(0.2126, 0.7152, 0.0722));
    float amount = strength * (1.0 - niri_focused);
    color.rgb = mix(color.rgb, vec3(gray), amount);

    return color;
}
