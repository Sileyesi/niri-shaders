uniform sampler2D niri_tex;
uniform sampler2D niri_backdrop;

// Signed distance to a rounded rect (negative inside). `rect` is (x, y, width, height); `radius`
// is top-left, top-right, bottom-right, bottom-left. Same as in the ShojiWM prelude.
float shoji_rrect_sdf(vec2 p, vec4 rect, vec4 radius) {
    vec2 half_size = rect.zw * 0.5;
    vec2 q = p - rect.xy - half_size;
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

// Pixel coverage of a signed distance, antialiased over one pixel.
float shoji_coverage(float sdf) {
    return clamp(0.5 - sdf, 0.0, 1.0);
}

// Color of the window at a point in the geometry coordinates, premultiplied.
vec4 niri_sample(vec3 coords_geo) {
    return texture2D(niri_tex, (niri_geo_to_tex * coords_geo).xy);
}

// What's behind the window at a point in the geometry coordinates: niri's blur, when the window has
// it. A shader that uses this gets niri_tex without the blur, and draws the backdrop itself.
vec4 niri_sample_backdrop(vec3 coords_geo) {
    return texture2D(niri_backdrop, (niri_geo_to_backdrop * coords_geo).xy);
}

// Signed distance to the rounded edge of the tile, in logical pixels.
float niri_tile_sdf(vec3 coords_geo, vec3 size_geo) {
    return shoji_rrect_sdf(coords_geo.xy * size_geo.xy, vec4(vec2(0.0), size_geo.xy),
                           niri_corner_radius);
}
