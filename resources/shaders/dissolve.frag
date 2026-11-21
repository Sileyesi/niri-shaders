// From ShojiWM (https://github.com/bea4dev/ShojiWM), MIT, Copyright (c) 2026 bea4dev
// Original: examples/output-overlay/output-dissolve.frag
// Changed for niri: the window dissolves in random blocks with a glowing edge instead of fading.
// niri drives `progress` with the close animation, so this plays when the window closes.
uniform float progress;

float hash(vec2 p) {
    p = fract(p * vec2(0.1031, 0.1030));
    p += dot(p, p.yx + 33.33);
    return fract((p.x + p.y) * p.x);
}

vec4 shader_main(EffectContext effect) {
    vec4 color = texture2D(tex, effect.texture_uv);

    // Blocks of a few logical pixels each get a random threshold.
    vec2 cell = floor(effect_texture_phy_px(effect) / (3.0 * effect.scale));
    float threshold = hash(cell);

    // Go a bit past 1 so that every block is gone at the end.
    float p = clamp(progress, 0.0, 1.0) * 1.1;
    float keep = smoothstep(p - 0.1, p, threshold);

    // Blocks that are about to go glow.
    float glow = keep * (1.0 - smoothstep(p, p + 0.12, threshold)) * step(0.001, p);
    color.rgb += vec3(1.0, 0.45, 0.1) * glow * color.a;

    return color * keep;
}
