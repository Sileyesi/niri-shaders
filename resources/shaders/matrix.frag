// The window turns into green phosphor, with columns of characters falling over it.

// How many of the columns have rain, from 0 to 1.
uniform float density;

float hash(vec2 p) {
    p = fract(p * vec2(0.1031, 0.1030));
    p += dot(p, p.yx + 33.33);
    return fract((p.x + p.y) * p.x);
}

// A made-up character: a 5 × 7 grid of dots with a gap around it.
float glyph(vec2 cell_uv, float id) {
    vec2 g = floor(cell_uv * vec2(6.0, 8.0));
    if (g.x < 1.0 || g.y < 1.0) {
        return 0.0;
    }
    return step(0.5, hash(g + vec2(id * 7.13, id * 3.71)));
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 px = coords_geo.xy * size_geo.xy;
    vec4 tex = niri_sample(coords_geo);
    float luma = dot(tex.rgb, vec3(0.299, 0.587, 0.114)) / max(tex.a, 0.001);

    // The window in green phosphor.
    vec3 base = vec3(0.1, 1.0, 0.35) * (0.12 + 0.88 * luma);

    // Columns of characters, each with its own speed and trail.
    vec2 cell_size = vec2(10.0, 14.0);
    vec2 cell = floor(px / cell_size);
    vec2 cell_uv = fract(px / cell_size);

    float has_rain = step(1.0 - density, hash(vec2(cell.x, 4.0)));
    float speed = mix(5.0, 16.0, hash(vec2(cell.x, 1.0)));
    float trail_len = mix(6.0, 22.0, hash(vec2(cell.x, 3.0)));
    float cycle = size_geo.y / cell_size.y + trail_len + 8.0;
    float head = niri_time * speed + hash(vec2(cell.x, 2.0)) * cycle;
    float behind = mod(head - cell.y, cycle);
    float trail = behind < trail_len ? 1.0 - behind / trail_len : 0.0;

    // Characters change now and then, the head is almost white.
    float id = floor(hash(cell) * 50.0 + niri_time * mix(1.0, 8.0, hash(cell + 5.0)));
    float rain = glyph(cell_uv, id) * trail * has_rain;
    vec3 rain_rgb = mix(vec3(0.15, 1.0, 0.35), vec3(0.8, 1.0, 0.85), step(behind, 1.0));

    float rain_a = rain * 0.95;
    vec3 rgb = base * tex.a * (1.0 - rain_a) + rain_rgb * rain_a;
    float alpha = tex.a + rain_a * (1.0 - tex.a);

    float coverage = shoji_coverage(niri_tile_sdf(coords_geo, size_geo) * niri_scale);
    return vec4(rgb, alpha) * coverage;
}
