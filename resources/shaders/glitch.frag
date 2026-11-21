// Digital glitches: every few seconds the window tears into shifted rows with split color
// channels and a few inverted blocks.

// How strong the glitches are.
uniform float strength;

float hash(vec2 p) {
    p = fract(p * vec2(0.1031, 0.1030));
    p += dot(p, p.yx + 33.33);
    return fract((p.x + p.y) * p.x);
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 uv = coords_geo.xy;
    float seed = niri_random_seed * 97.0;

    // Time goes in slots of 0.1 s, and about one slot in 30 glitches.
    float slot = floor(niri_time * 10.0);
    float burst = step(0.967, hash(vec2(slot, seed)));
    float amount = burst * strength * (0.4 + 0.6 * hash(vec2(slot, seed + 1.0)));

    // Bands of rows shifted sideways.
    float rows = mix(6.0, 40.0, hash(vec2(slot, seed + 2.0)));
    float band = floor(uv.y * rows);
    float tear = (hash(vec2(band, slot + seed)) - 0.5) * step(0.55, hash(vec2(band + 5.0, slot)));
    vec2 shifted = uv + vec2(tear * 0.12 * amount, 0.0);

    // Color channels split apart, a little even between glitches.
    vec2 split = vec2((1.0 + 14.0 * amount) / size_geo.x, 0.0);
    vec4 g = niri_sample(vec3(shifted, 1.0));
    vec4 r = niri_sample(vec3(shifted + split, 1.0));
    vec4 b = niri_sample(vec3(shifted - split, 1.0));
    vec4 color = vec4(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));

    // A few blocks get inverted.
    float block = step(0.975, hash(floor(uv * vec2(10.0, 30.0)) + slot)) * burst;
    color.rgb = mix(color.rgb, vec3(color.a) - color.rgb, block);

    return color;
}
