// Rain on the glass behind a semitransparent window: drops land at random and send out rings that
// bend the blurred backdrop and catch the light. The window itself stays sharp on top.
//
// The preset turns on blur behind the window. Give the window a see-through background, for
// example kitty with `background_opacity 0.4`.

// Size of the cells that drops land in, in logical pixels. Smaller is denser rain.
uniform float cell_px;
// Seconds between two drops in the same cell.
uniform float period;
// Part of the cells that get drops, from 0 to 1.
uniform float chance;
// How fast the rings spread, in logical pixels per second.
uniform float ring_speed;
// Distance between the waves of a ring, in logical pixels.
uniform float wavelength;
// How far the backdrop bends under a wave, in logical pixels.
uniform float refraction;
// How bright the light on the waves is.
uniform float glint;
// Brightness of the backdrop under the water.
uniform float tint;

float rain_hash(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Slope of the water surface at `p`, summed over the drops in this cell and two around it.
vec2 rain_slope(vec2 p) {
    float size = max(cell_px, 8.0);
    float life = max(period, 0.1);
    float k = 6.2832 / max(wavelength, 1.0);
    vec2 cell = floor(p / size);
    vec2 slope = vec2(0.0);

    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            vec2 id = cell + vec2(float(x), float(y));

            // Each cell gets a drop every `period` seconds, at its own moment, and every drop
            // lands somewhere else in the cell.
            float clock = niri_time / life + rain_hash(id * 1.37);
            float n = floor(clock);
            if (rain_hash(id + n * 3.17) > chance) {
                continue;
            }
            float age = fract(clock);
            vec2 spot = (id + vec2(rain_hash(id + n * 7.31), rain_hash(id - n * 5.13))) * size;

            vec2 d = p - spot;
            float r = length(d);
            // Distance behind the front of the ring.
            float behind = age * life * ring_speed - r;
            // A few waves right behind the front.
            float width = 1.2 * max(wavelength, 1.0);
            float train = exp(-behind * behind / (2.0 * width * width)) * step(-wavelength, behind);
            // Weaker as it grows, gone before it leaves the cells around.
            float fade = (1.0 - age) * (1.0 - age) * smoothstep(size * 2.5, size * 1.5, r);
            float wave = cos(behind * k) * train * fade;

            slope += (r > 0.001 ? d / r : vec2(0.0)) * wave;
        }
    }

    return slope;
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 p = coords_geo.xy * size_geo.xy;
    vec2 slope = rain_slope(p);

    // The backdrop, bent by the waves.
    vec2 bent = (p - slope * refraction) / size_geo.xy;
    vec4 backdrop = niri_sample_backdrop(vec3(bent, 1.0));
    backdrop.rgb *= tint;

    // Waves that face the light, coming from the top left, catch it.
    float facing = clamp(dot(slope, normalize(vec2(0.6, 0.8))), 0.0, 1.0);
    backdrop.rgb += vec3(0.85, 0.92, 1.0) * glint * pow(facing, 1.5) * backdrop.a;

    // Only inside the tile, and the window goes on top.
    backdrop *= shoji_coverage(niri_tile_sdf(coords_geo, size_geo) * niri_scale);
    vec4 window = niri_sample(coords_geo);
    return window + backdrop * (1.0 - window.a);
}
