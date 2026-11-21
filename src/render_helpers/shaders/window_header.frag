#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

varying vec2 niri_v_coords;
uniform vec2 niri_size;
uniform float niri_scale;
uniform float niri_alpha;

uniform mat3 niri_input_to_geo;
uniform vec2 niri_geo_size;
uniform mat3 niri_geo_to_tex;
uniform mat3 niri_geo_to_backdrop;

uniform float niri_time;
uniform float niri_focused;
uniform vec2 niri_cursor;
uniform float niri_close_progress;
uniform float niri_random_seed;
uniform vec4 niri_corner_radius;
uniform float niri_border_width;
uniform vec4 niri_border_color;
uniform float niri_focus_time;
