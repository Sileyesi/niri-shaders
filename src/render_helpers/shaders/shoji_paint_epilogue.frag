
void main() {
    PaintContext ctx = shoji_make_paint_context();
    vec3 coords_geo = niri_input_to_geo * vec3(niri_v_coords, 1.0);
    vec4 tile = texture2D(niri_tex, (niri_geo_to_tex * coords_geo).xy);

    // Like in ShojiWM.
    vec4 paint = paint_main(ctx) * (shoji_opacity * shoji_clip_coverage(ctx.frag_phy_px));

    // The paint replaces niri's border. The window shows through the hole over it, and what niri
    // drew outside of the frame, like the shadow and the focus ring, stays under it.
    vec4 client = tile * shoji_inner_coverage(ctx);
    vec4 outside = tile * (1.0 - shoji_outer_coverage(ctx));
    vec4 color = shoji_over(client, shoji_over(paint, outside));

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
