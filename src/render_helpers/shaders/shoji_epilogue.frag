
void main() {
    vec3 coords_geo = niri_input_to_geo * vec3(niri_v_coords, 1.0);
    vec2 texture_uv = (niri_geo_to_tex * coords_geo).xy;

    EffectContext effect = make_effect_context(
        texture_uv,
        niri_shoji_texture_size,
        niri_shoji_content_rect,
        niri_shoji_frame_rect
    );

    vec4 color = shader_main(effect);

    // Outside of the window frame, keep what niri drew there, like the shadow and the focus ring.
    color = mix(texture2D(tex, texture_uv), color, effect_frame_coverage(effect));

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
