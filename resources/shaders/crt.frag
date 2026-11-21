// An old CRT monitor: curved glass, scanlines, a shadow mask, a vignette, a rolling bar and a
// little flicker.

// How strong the effect is, from 0 to 1.
uniform float intensity;
// Color of the phosphor, multiplied with the window.
uniform vec3 tint;

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    // Bend the picture like the glass of a CRT.
    vec2 c = coords_geo.xy * 2.0 - 1.0;
    c *= 1.0 + 0.08 * intensity * dot(c.yx, c.yx);
    vec2 uv = c * 0.5 + 0.5;

    // A little color fringing.
    vec2 shift = vec2(0.7 * intensity / size_geo.x, 0.0);
    vec4 color = niri_sample(vec3(uv, 1.0));
    color.r = niri_sample(vec3(uv + shift, 1.0)).r;
    color.b = niri_sample(vec3(uv - shift, 1.0)).b;

    vec2 px = uv * size_geo.xy * niri_scale;

    // Scanlines, one dark line every 3 physical pixels.
    float scan = 0.5 + 0.5 * cos(px.y * 6.2832 / 3.0);
    color.rgb *= mix(1.0, 0.55 + 0.45 * scan, intensity);

    // Shadow mask of red, green and blue stripes.
    float column = mod(floor(px.x), 3.0);
    vec3 mask = vec3(column == 0.0, column == 1.0, column == 2.0);
    color.rgb *= mix(vec3(1.0), 0.7 + 0.6 * mask, 0.35 * intensity);

    // Darker toward the edges of the tube.
    float vignette = 16.0 * uv.x * uv.y * (1.0 - uv.x) * (1.0 - uv.y);
    color.rgb *= mix(1.0, pow(clamp(vignette, 0.0, 1.0), 0.3), intensity);

    // A brighter bar that rolls down, and flicker.
    float roll = fract(uv.y - niri_time * 0.12);
    color.rgb *= 1.0 + 0.06 * intensity * smoothstep(0.0, 0.04, roll) * (1.0 - smoothstep(0.04, 0.2, roll));
    color.rgb *= 1.0 - 0.04 * intensity * (0.5 + 0.5 * sin(niri_time * 55.0));

    color.rgb = min(color.rgb * tint * 1.15, vec3(color.a));

    // The bent edges, antialiased.
    vec2 edge = min(uv, 1.0 - uv) * size_geo.xy * niri_scale;
    return color * clamp(min(edge.x, edge.y), 0.0, 1.0);
}
