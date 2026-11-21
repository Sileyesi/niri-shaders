// An animated rainbow ring around the window that glows outward. Brighter on the focused window.

// Width of the ring, in logical pixels.
uniform float width;
// How far the glow reaches, in logical pixels. Keep it below the margin of the shader.
uniform float glow;
// Turns of the rainbow per second.
uniform float speed;

vec3 hsv2rgb(vec3 c) {
    vec3 p = abs(fract(c.xxx + vec3(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 px = coords_geo.xy * size_geo.xy;
    vec4 window = niri_sample(coords_geo);

    // Distance to the rounded edge of the tile, positive outside of it.
    float d = shoji_rrect_sdf(px, vec4(vec2(0.0), size_geo.xy), niri_corner_radius);

    // The ring goes right outside of the tile, so it doesn't cover the window.
    float ring = shoji_coverage((abs(d - width * 0.5) - width * 0.5) * niri_scale);
    float halo = d > 0.0 ? exp(-max(d - width, 0.0) * 3.0 / max(glow, 1.0)) * 0.6 : 0.0;

    // The colors go around the window and move.
    vec2 c = px - size_geo.xy * 0.5;
    float hue = fract(atan(c.y, c.x) / 6.2832 + 0.5 + niri_time * speed);
    vec3 rgb = hsv2rgb(vec3(hue, 0.8, 1.0));

    float brightness = mix(0.45, 1.0, niri_focused) * (0.85 + 0.15 * sin(niri_time * 3.0));
    vec4 neon = vec4(rgb, 1.0) * max(ring, halo) * brightness;

    return window + neon * (1.0 - window.a);
}
