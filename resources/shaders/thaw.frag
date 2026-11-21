// Ice on every window out of focus. Give a window the focus and the ice melts away from the
// pointer, crystal by crystal; take the focus away and it freezes over again from the edges. The
// pointer over a frozen window is warm and melts a spot under it. Close the window and it freezes
// at once, then shatters.
//
// What makes it work is what a window shader knows here and nowhere else: whether the window has
// the focus and since when, where the pointer is over it, what's behind it, and how far its close
// animation is.
//
// The preset turns on blur behind the window and draws 48 logical pixels around it, for the
// shards. Give the window a see-through background, for example kitty with
// `background_opacity 0`, and a slower close animation to watch it shatter.

// Size of the crystals, in logical pixels.
uniform float crystal_px;
// How far a crystal bends what's behind it, in logical pixels.
uniform float refraction_px;
// Seconds the ice takes to melt off the whole window.
uniform float thaw_seconds;

// A file outside the presets gets 0 for the uniforms that the config doesn't set.
float thaw_or(float value, float fallback) {
    return value > 0.0 ? value : fallback;
}

float thaw_hash(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

vec2 thaw_hash2(vec2 p) {
    vec3 q = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.xx + q.yz) * q.zy);
}

float thaw_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(thaw_hash(i), thaw_hash(i + vec2(1.0, 0.0)), u.x),
               mix(thaw_hash(i + vec2(0.0, 1.0)), thaw_hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

// Feathery frost: thin ridges of noise at three scales, from 0 to about 0.9.
float thaw_frost(vec2 p) {
    float sum = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 3; i++) {
        sum += amp * (1.0 - abs(2.0 * thaw_noise(p) - 1.0));
        p = p * 2.1 + vec2(5.2, 1.3);
        amp *= 0.5;
    }
    return sum;
}

// The crystal that `g` is in, with `g` in crystals: its id, its center, and how far `g` is from
// the border with the next crystal, where the crack is.
void thaw_crystal(vec2 g, out vec2 id, out vec2 center, out float border) {
    vec2 cell = floor(g);
    float d1 = 8.0;
    float d2 = 8.0;
    id = cell;
    center = cell;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 c = cell + vec2(float(x), float(y));
            vec2 point = c + 0.15 + 0.7 * thaw_hash2(c);
            float d = length(g - point);
            if (d < d1) {
                d2 = d1;
                d1 = d;
                id = c;
                center = point;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }

    border = d2 - d1;
}

// Where the melting starts: the pointer, or the middle of the window when it's elsewhere.
vec2 thaw_origin(vec2 size) {
    return niri_cursor.x >= 0.0 ? niri_cursor * size : 0.5 * size;
}

// How far the melting has gone from its origin, in logical pixels.
float thaw_front(vec2 size, float crystal) {
    // A window that never had the focus is frozen. One that has it without a time, like a window
    // that opened with it, has melted already.
    if (niri_focus_time < 0.0) {
        return niri_focused > 0.5 ? 1e5 : -1e4;
    }
    float speed = (length(size) + 2.0 * crystal) / thaw_or(thaw_seconds, 1.2);
    return niri_focus_time * speed;
}

// How much ice is on the crystal with this id and center, from 0 to 1.
float thaw_ice(vec2 id, vec2 center, vec2 size, float crystal) {
    // Melting, crystal by crystal, behind a front that runs out from the origin since the focus.
    float melt_at = distance(center, thaw_origin(size)) + crystal * thaw_hash(id + 7.7);
    float melted = clamp((thaw_front(size, crystal) - melt_at) / (0.6 * crystal), 0.0, 1.0);

    // Freezing from the edges, while the focus goes away.
    float edge = min(min(center.x, size.x - center.x), min(center.y, size.y - center.y));
    float reach = (1.0 - niri_focused) * (0.5 * min(size.x, size.y) + 1.5 * crystal);
    float freeze_at = max(edge, 0.0) + 0.5 * crystal * thaw_hash(id + 3.1);
    float frozen = clamp((reach - freeze_at) / (0.5 * crystal), 0.0, 1.0);

    float ice = max(frozen, 1.0 - melted);

    // The pointer is warm.
    if (niri_cursor.x >= 0.0) {
        float warm = (1.3 * crystal - distance(center, niri_cursor * size)) / (0.6 * crystal);
        ice *= 1.0 - 0.9 * clamp(warm, 0.0, 1.0);
    }

    // Closing freezes the window at once, before it shatters.
    return max(ice, smoothstep(0.0, 0.15, niri_close_progress));
}

// The window at `p`, in logical pixels, under `ice` of the crystal `id`, over what's behind it.
vec4 thaw_look(vec2 p, vec2 size, vec2 id, float border, float ice, vec2 extra_bend) {
    float crystal = thaw_or(crystal_px, 140.0);

    // Every crystal bends the light its own way, what's behind more than the window.
    vec2 facet = 2.0 * thaw_hash2(id + 1.3) - 1.0;
    vec2 bend = facet * thaw_or(refraction_px, 70.0) * ice + extra_bend;
    vec4 back = niri_sample_backdrop(vec3(clamp((p + bend) / size, 0.0, 1.0), 1.0));
    vec4 window = niri_sample(vec3(clamp((p + 0.2 * bend) / size, 0.0, 1.0), 1.0));
    vec4 color = window + back * (1.0 - window.a);

    // Cold blue ice, brighter on the crystals that face the light from the top left.
    float light = dot(facet, normalize(vec2(-0.6, -0.8)));
    color.rgb *= mix(vec3(1.0), vec3(0.78, 0.9, 1.05) * (0.95 + 0.2 * light), ice);

    // White frost, thicker toward the edges of the window, and thin cracks between the crystals
    // that break off here and there.
    float edge = min(min(p.x, size.x - p.x), min(p.y, size.y - p.y));
    float frost = ice * (0.1 + 0.45 * thaw_frost(p / (0.3 * crystal)));
    frost += ice * 0.5 * (1.0 - smoothstep(0.0, 0.5 * crystal, edge)) * thaw_frost(p / 24.0);
    float crack = ice * (1.0 - smoothstep(0.0, 0.025, border));
    crack *= smoothstep(0.35, 0.6, thaw_noise(p / (0.4 * crystal) + 3.7));
    float white = clamp(0.6 * frost + 0.55 * crack, 0.0, 0.9);
    return vec4(vec3(0.9, 0.96, 1.0) * white, white) + color * (1.0 - white);
}

vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec2 size = size_geo.xy;
    vec2 p = coords_geo.xy * size;
    float crystal = thaw_or(crystal_px, 140.0);
    // Every window has its own crystals.
    vec2 offset = vec2(97.0, 57.0) * niri_random_seed;
    float shatter = clamp((niri_close_progress - 0.15) / 0.85, 0.0, 1.0);

    if (shatter <= 0.0) {
        vec2 id;
        vec2 center;
        float border;
        thaw_crystal(p / crystal + offset, id, center, border);
        float ice = thaw_ice(id, (center - offset) * crystal, size, crystal);

        // A ring of meltwater runs ahead of the melting.
        vec2 from_origin = p - thaw_origin(size);
        float behind = thaw_front(size, crystal) - length(from_origin);
        float ring = exp(-behind * behind / 300.0) * niri_focused;
        ring *= 1.0 - smoothstep(0.8, 1.3, niri_focus_time / thaw_or(thaw_seconds, 1.2));
        vec2 push = from_origin / max(length(from_origin), 1.0) * 10.0 * ring;

        vec4 color = thaw_look(p, size, id, border, ice, push);
        color.rgb += vec3(0.85, 0.93, 1.0) * 0.3 * ring * color.a;
        return color * shoji_coverage(niri_tile_sdf(coords_geo, size_geo) * niri_scale);
    }

    // Shattering: every crystal shrinks, flies away from the middle and falls. Each pixel looks
    // for the crystal that ended up over it.
    vec2 cell = floor(p / crystal + offset);
    float shrink = 1.0 - 0.6 * shatter;
    float fade = 1.0 - smoothstep(0.55, 1.0, shatter);

    for (int y = -2; y <= 2; y++) {
        for (int x = -2; x <= 2; x++) {
            vec2 c = cell + vec2(float(x), float(y));
            vec2 center = (c + 0.15 + 0.7 * thaw_hash2(c) - offset) * crystal;
            vec2 away = center - 0.5 * size;
            away /= max(length(away), 1.0);
            vec2 move = away * (0.3 + 0.4 * thaw_hash(c + 9.1)) * crystal * shatter
                + vec2(0.0, 0.9) * crystal * shatter * shatter;

            // Where this pixel was on the crystal before it moved.
            vec2 q = center + (p - center - move) / shrink;
            vec2 q_id;
            vec2 q_center;
            float q_border;
            thaw_crystal(q / crystal + offset, q_id, q_center, q_border);
            if (q_id == c) {
                float inside = shoji_coverage(niri_tile_sdf(vec3(q / size, 1.0), size_geo) * niri_scale);
                return thaw_look(q, size, c, q_border, 1.0, vec2(0.0)) * inside * fade;
            }
        }
    }

    return vec4(0.0);
}
