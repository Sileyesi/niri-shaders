
void main() {
    vec3 coords_geo = niri_input_to_geo * vec3(niri_v_coords, 1.0);
    vec2 texture_uv = (niri_geo_to_backdrop * coords_geo).xy;

    EffectContext effect = make_effect_context(
        texture_uv,
        niri_shoji_texture_size,
        niri_shoji_content_rect,
        niri_shoji_frame_rect
    );

    // The effect draws what's behind the window, inside its frame, and the window goes on top.
    vec4 behind = shader_main(effect) * effect_frame_coverage(effect);
    vec4 window = texture2D(niri_tex, (niri_geo_to_tex * coords_geo).xy);
    vec4 color = window + behind * (1.0 - window.a);

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
