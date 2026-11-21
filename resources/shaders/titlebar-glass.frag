// The liquid glass under the title bars of floating windows: the lens of island-glass.frag (the
// island-refract stage of ShojiWM's true liquid glass) over what's really under the title bar.
// niri gives it the backdrop blurred as `tex` and sharp as `sharp_tex`: the middle of the glass is
// mostly clear and it frosts toward the edge.
//
// The glass goes on down into the window, so the lens bends only along the top and the sides, and
// the title bar looks like the top of one pane of glass rather than a pill of its own.

// The glass in physical pixels of the texture, and its corner radii (top-left, top-right,
// bottom-right, bottom-left).
uniform vec4 glass_rect;
uniform vec4 glass_radius;
uniform float rim_width_px;
uniform float refraction_px;
uniform float chromatic_shift_px;
uniform float highlight_strength;
uniform float edge_softness_px;
// RGB of the tint over the glass, and how much of it.
uniform vec4 glass_tint;
uniform sampler2D sharp_tex;
// How frosted the middle and the edge are, from 0 (clear) to 1 (all blur).
uniform float center_frost;
uniform float edge_frost;

float distanceAt(vec2 p, float rounding) {
    return -shoji_rrect_sdf(p, glass_rect, max(glass_radius, vec4(rounding)));
}

// Circular lens profile from Aghajari's Liquid Glass article, as in island-refract.frag:
// https://www.aghajari.com/publications/liquid-glass/
float circularLens(float distancePx, float widthPx) {
    float x = 1.0 - clamp(distancePx / widthPx, 0.0, 1.0);
    float epsilon = clamp(edge_softness_px / widthPx, 0.0001, 0.5);
    float top = sqrt(1.0 + epsilon);
    return (top - sqrt(max(1.0 - x * x, 0.0) + epsilon)) / (top - sqrt(epsilon));
}

vec3 backdropAt(vec2 uv, vec2 size, float frost) {
    uv = clamp(uv, 0.5 / size, 1.0 - 0.5 / size);
    return mix(texture2D(sharp_tex, uv).rgb, texture2D(tex, uv).rgb, frost);
}

vec4 shader_main(EffectContext effect) {
    vec2 size = effect.texture_size_phy_px;
    vec2 p = effect.texture_uv * size;
    float distance = distanceAt(p, 0.0);
    float mask = shoji_coverage(-distance);
    if (mask <= 0.001)
        return vec4(0.0);

    float rimWidth = max(rim_width_px, 1.0);
    // The bend follows a rounder copy of the shape, so that it turns smoothly around the corners.
    float rounding = rimWidth * 0.5;
    vec2 e = vec2(1.5, 0.0);
    vec2 gradient = vec2(distanceAt(p + e.xy, rounding) - distanceAt(p - e.xy, rounding),
                         distanceAt(p + e.yx, rounding) - distanceAt(p - e.yx, rounding)) / 3.0;
    float magnitude = length(gradient);
    vec2 inward = gradient / max(magnitude, 0.0001);
    float coherence = smoothstep(0.15, 0.85, magnitude);

    float d = max(distance, 0.0);
    float profile = 0.5 * (circularLens(d - 0.25, rimWidth) + circularLens(d + 0.25, rimWidth));
    vec2 offset = inward * profile * coherence * min(refraction_px, rimWidth) / size;
    float chromatic = circularLens(d, max(rimWidth * 0.5, 1.0));
    vec2 dispersion = inward * coherence * chromatic * chromatic_shift_px / size;

    vec2 uv = effect.texture_uv;
    float frost = mix(center_frost, edge_frost, profile);
    vec3 color = vec3(backdropAt(uv + offset + dispersion, size, frost).r,
                      backdropAt(uv + offset, size, frost).g,
                      backdropAt(uv + offset - dispersion, size, frost).b);
    color = mix(color, glass_tint.rgb, clamp(glass_tint.a, 0.0, 1.0));

    // Light from the upper left on the rim, screened so that it never clips to a flat line.
    float light = dot(-inward, normalize(vec2(-0.45, -0.89)));
    float glint = 0.15 + 0.85 * pow(max(light, 0.0), 1.5) + 0.4 * pow(max(-light, 0.0), 1.5);
    float band = exp(-d / 1.6);
    float shine = clamp(highlight_strength * glint * coherence * (1.1 * band + 0.25 * profile),
                        0.0, 1.0);
    color = 1.0 - (1.0 - color) * (1.0 - shine * vec3(0.94, 0.97, 1.0));

    return vec4(clamp(color, 0.0, 1.0) * mask, mask);
}
