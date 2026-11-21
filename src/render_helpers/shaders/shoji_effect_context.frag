// Filled in by niri for make_effect_context(), in physical pixels of the texture.
uniform vec2 niri_shoji_texture_size;
uniform vec4 niri_shoji_content_rect;
uniform vec4 niri_shoji_frame_rect;

// The rest is the EffectContext prelude from ShojiWM (https://github.com/bea4dev/ShojiWM),
// src/shojiwm_lib/src/backend/shader_effect.rs, effect_context_shader_prelude().
// MIT, Copyright (c) 2026 bea4dev.

// The shape of what the effect belongs to. Programs that do not know it
// leave these at zero (no radius, border or clip; scale 1).
uniform vec4 effect_frame_radius_phy_px;
uniform vec4 effect_frame_border_phy_px;
uniform vec4 effect_clip_rect_phy_px;
uniform vec4 effect_clip_radius_phy_px;
uniform float effect_has_clip;
uniform float effect_scale;

struct EffectContext {
    vec2 texture_uv;
    vec2 texture_size_phy_px;
    vec4 content_rect_phy_px;
    vec4 frame_rect_phy_px;
    vec4 frame_radius_phy_px;   // top-left, top-right, bottom-right, bottom-left
    vec4 frame_border_phy_px;   // top, right, bottom, left
    vec4 clip_rect_phy_px;      // in the texture, like frame_rect_phy_px
    vec4 clip_radius_phy_px;
    bool has_clip;
    float scale;                // physical pixels per logical pixel
    // Former names of the size/rect fields above (same values).
    vec2 texture_size_px;
    vec4 content_rect_px;
    vec4 frame_rect_px;
};

EffectContext make_effect_context(
    vec2 texture_uv,
    vec2 texture_size_phy_px,
    vec4 content_rect_phy_px,
    vec4 frame_rect_phy_px
) {
    return EffectContext(
        texture_uv,
        texture_size_phy_px,
        content_rect_phy_px,
        frame_rect_phy_px,
        effect_frame_radius_phy_px,
        effect_frame_border_phy_px,
        effect_clip_rect_phy_px,
        effect_clip_radius_phy_px,
        effect_has_clip > 0.5,
        effect_scale > 0.0 ? effect_scale : 1.0,
        texture_size_phy_px,
        content_rect_phy_px,
        frame_rect_phy_px
    );
}

// Signed distance to a rounded rect (negative inside). `rect` is
// (x, y, width, height); `radius` is top-left, top-right, bottom-right,
// bottom-left.
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

// Pixel coverage of a signed distance, antialiased over one physical pixel.
float shoji_coverage(float sdf) {
    return clamp(0.5 - sdf, 0.0, 1.0);
}

vec2 effect_texture_phy_px(EffectContext effect) {
    return effect.texture_uv * effect.texture_size_phy_px;
}

vec2 effect_content_phy_px(EffectContext effect) {
    return effect_texture_phy_px(effect) - effect.content_rect_phy_px.xy;
}

vec2 effect_content_uv(EffectContext effect) {
    return effect_content_phy_px(effect) / max(effect.content_rect_phy_px.zw, vec2(1.0));
}

vec2 effect_texture_uv_from_content_phy_px(EffectContext effect, vec2 content_phy_px) {
    return (effect.content_rect_phy_px.xy + content_phy_px) /
        max(effect.texture_size_phy_px, vec2(1.0));
}

vec2 effect_frame_phy_px(EffectContext effect) {
    return effect_texture_phy_px(effect) - effect.frame_rect_phy_px.xy;
}

vec2 effect_frame_uv(EffectContext effect) {
    return effect_frame_phy_px(effect) / max(effect.frame_rect_phy_px.zw, vec2(1.0));
}

// Signed distance to the frame's rounded rect (negative inside).
float effect_frame_sdf(EffectContext effect) {
    return shoji_rrect_sdf(
        effect_texture_phy_px(effect),
        effect.frame_rect_phy_px,
        effect.frame_radius_phy_px
    );
}

float effect_frame_coverage(EffectContext effect) {
    return shoji_coverage(effect_frame_sdf(effect));
}

// Signed distance to the inner edge of the frame's border.
float effect_frame_inner_sdf(EffectContext effect) {
    vec4 b = effect.frame_border_phy_px;
    vec4 r = effect.frame_radius_phy_px;
    vec4 rect = effect.frame_rect_phy_px;
    vec4 inner = vec4(rect.xy + vec2(b.w, b.x), max(rect.zw - vec2(b.y + b.w, b.x + b.z), vec2(0.0)));
    vec4 inner_radius = max(r - vec4(max(b.x, b.w), max(b.x, b.y), max(b.z, b.y), max(b.z, b.w)), vec4(0.0));
    return shoji_rrect_sdf(effect_texture_phy_px(effect), inner, inner_radius);
}

// Signed distance to the clip ShojiWM applies (very negative without one).
float effect_clip_sdf(EffectContext effect) {
    if (!effect.has_clip) {
        return -1.0e6;
    }
    return shoji_rrect_sdf(
        effect_texture_phy_px(effect),
        effect.clip_rect_phy_px,
        effect.clip_radius_phy_px
    );
}

float effect_clip_coverage(EffectContext effect) {
    return shoji_coverage(effect_clip_sdf(effect));
}

// Former helper names.
vec2 effect_texture_px(EffectContext effect) {
    return effect_texture_phy_px(effect);
}

vec2 effect_content_px(EffectContext effect) {
    return effect_content_phy_px(effect);
}

vec2 effect_texture_uv_from_content_px(EffectContext effect, vec2 content_px) {
    return effect_texture_uv_from_content_phy_px(effect, content_px);
}

vec2 effect_frame_px(EffectContext effect) {
    return effect_frame_phy_px(effect);
}
