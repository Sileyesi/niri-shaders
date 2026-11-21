// A sci-fi hologram: a blue tint, running bands, noise, flickering edges and rows that jump
// sideways now and then.

// Color of the hologram.
uniform vec3 color;

float hash(vec2 p) {
    p = fract(p * vec2(0.1031, 0.1030));
    p += dot(p, p.yx + 33.33);
    return fract((p.x + p.y) * p.x);
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 px = coords_geo.xy * size_geo.xy;

    // Now and then a row of pixels jumps sideways.
    float row = floor(px.y / 3.0);
    float tick = floor(niri_time * 15.0);
    float jump = step(0.992, hash(vec2(row * 0.37, tick))) * (hash(vec2(row, tick + 1.0)) - 0.5) * 30.0;
    vec4 tex = niri_sample(vec3((px + vec2(jump, 0.0)) / size_geo.xy, 1.0));
    float luma = dot(tex.rgb, vec3(0.299, 0.587, 0.114)) / max(tex.a, 0.001);

    // Thin lines and a wide band, both running down.
    float lines = 0.7 + 0.3 * sin(px.y * 0.9 - niri_time * 8.0);
    float band = fract(coords_geo.y * 0.8 - niri_time * 0.3);
    band = smoothstep(0.0, 0.15, band) * (1.0 - smoothstep(0.15, 0.3, band));

    vec3 rgb = color * (0.3 + 1.3 * luma) * lines + color * band * 0.35;
    rgb += (hash(floor(px * niri_scale) + fract(niri_time * 7.0) * 113.0) - 0.5) * 0.15;
    float alpha = tex.a * (0.45 + 0.35 * luma + 0.2 * band);

    // Edges that glow and flicker.
    float d = niri_tile_sdf(coords_geo, size_geo);
    float flicker = 0.6 + 0.4 * sin(niri_time * 31.0 + coords_geo.y * 25.0) * hash(vec2(tick, 3.0));
    float edge = exp(min(d, 0.0) / 5.0) * flicker;
    rgb += color * edge;
    alpha = max(alpha, edge * 0.8);

    // The whole picture flickers a little.
    alpha *= 0.9 + 0.1 * hash(vec2(tick, 7.0));
    alpha *= shoji_coverage(d * niri_scale);

    return vec4(clamp(rgb, 0.0, 1.0) * alpha, alpha);
}
