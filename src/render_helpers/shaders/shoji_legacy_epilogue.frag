
#undef texture2D

float niri_legacy_frame_sdf(vec2 p, vec2 size, vec4 radius) {
    vec2 half_size = size * 0.5;
    vec2 q = p - half_size;
    float r;
    if (q.x >= 0.0) {
        r = q.y >= 0.0 ? radius.z : radius.y;
    } else {
        r = q.y >= 0.0 ? radius.w : radius.x;
    }
    r = min(r, min(half_size.x, half_size.y));
    vec2 d = abs(q) - (half_size - vec2(r));
    return min(max(d.x, d.y), 0.0) + length(max(d, 0.0)) - r;
}

void main() {
    vec3 coords_geo = niri_input_to_geo * vec3(niri_v_coords, 1.0);

    vec4 effect = shader_main(coords_geo.xy, niri_geo_size * niri_scale);

    // The effect goes behind the window, inside its rounded frame.
    vec4 window = texture2D(niri_tex, (niri_geo_to_tex * coords_geo).xy);
    float sdf = niri_legacy_frame_sdf(coords_geo.xy * niri_geo_size, niri_geo_size,
                                      niri_corner_radius);
    float frame = clamp(0.5 - sdf * niri_scale, 0.0, 1.0);
    vec4 color = window + effect * frame * (1.0 - window.a);

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
