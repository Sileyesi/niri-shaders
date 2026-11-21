// Waves that run out from the pointer while it is over the window.

// Height of the waves, in logical pixels.
uniform float amplitude;
// Distance between the waves, in logical pixels.
uniform float wavelength;
// How fast the waves run, in logical pixels per second.
uniform float speed;

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    if (niri_cursor.x < 0.0) {
        return niri_sample(coords_geo);
    }

    vec2 px = coords_geo.xy * size_geo.xy;
    vec2 d = px - niri_cursor * size_geo.xy;
    float r = length(d);

    float phase = (r - niri_time * speed) / wavelength * 6.2832;
    float falloff = exp(-r / 250.0);
    vec2 dir = r > 0.001 ? d / r : vec2(0.0);
    vec2 moved = px + dir * sin(phase) * amplitude * falloff;

    vec4 color = niri_sample(vec3(moved / size_geo.xy, 1.0));
    // Light on the crests.
    color.rgb += vec3(0.1) * max(cos(phase), 0.0) * falloff * color.a;
    return color;
}
