# Window shaders

This fork of niri can draw any window through a fragment shader.
You point a window rule at a `.frag` file or one of the built-in presets, and the window is drawn by your shader, together with its border, focus ring, shadow and rounded corners.
Save the file and the window changes within a second, no restart needed.

## Putting a shader on a window

Add `shader` to a [window rule](./wiki/Configuration:-Window-Rules.md):

```kdl
window-rule {
    match app-id="kitty"
    shader "~/.config/niri/shaders/crt.frag" {
        uniform "intensity" 0.7
        uniform "tint" 0.2 1.0 0.6
    }
}
```

The argument can be one of three things:

- a path to a file: `shader "~/.config/niri/shaders/crt.frag"`. Relative paths are relative to the config file. `shader path="..."` works too, like `custom-shader` in animations.
- a preset built into niri: `shader "preset:island-glass"`.
- the code itself, as a raw string: `shader r"vec4 window_color(vec3 coords_geo, vec3 size_geo) { ... }"`.

Inside the block:

- `uniform "name" 1.0` sets a uniform of your shader. One to four numbers become a `float`, `vec2`, `vec3` or `vec4`.
- `uniform "name" time-scale=0.25` ties a `float` uniform to niri's clock: it's the time in seconds times the scale, and the window is redrawn every frame.
- `margin 32` lets the shader draw that many logical pixels outside of the window, for glows. For the niri and ShojiWM paint formats.
- `input "backdrop"` gives a ShojiWM effect shader what's behind the window instead of the window, see [below](#shaders-from-shojiwm).

When several rules match a window, the last `shader` wins, with its own uniforms.
`exclude` works as usual, so you can give every window a shader and leave some out.

To find the app id of a window, run `niri msg windows`.

## Presets

| Preset | What it does | Uniforms and defaults |
|---|---|---|
| `island-glass` | The rim of ShojiWM's island glass: a thick lens along the edge with dispersion and light. Its distance field, 15 passes in ShojiWM, is exact for a window. | `rim_width_px 80`, `refraction_px 80`, `chromatic_shift_px 1.9`, `highlight_strength 0.6`, `highlight_width_px 2`, `edge_softness_px 2`, `bevel_width_px 10`, `bevel_shadow 0`, `debug_view 0` |
| `rain` | Raindrops ringing on the blurred glass behind a transparent window, under sharp text. | `cell_px 60`, `period 2.4`, `chance 0.72`, `ring_speed 75`, `wavelength 12`, `refraction 5`, `glint 0.6`, `tint 0.94` |
| `thaw` | Ice on every window out of focus. The focus melts it from the pointer, crystal by crystal, the pointer over a frozen window melts a spot, and closing freezes the window and shatters it. Margin 48. Give it a slower `window-close` animation. | `crystal_px 140`, `refraction_px 70`, `thaw_seconds 1.2` |
| `crt` | Curved glass, scanlines, shadow mask, vignette, flicker. | `intensity 0.7`, `tint 1 1 1` |
| `hologram` | Blue tint, running bands, noise, flickering edges, rows that jump. | `color 0.3 0.75 1` |
| `neon-edge` | Rainbow ring around the window that glows outward. Margin 32. | `width 3`, `glow 18`, `speed 0.25` |
| `glitch` | Every few seconds: torn rows, split color channels, inverted blocks. | `strength 1` |
| `matrix` | Green phosphor with falling characters. | `density 0.7` |
| `ripple` | Waves running out from the pointer. | `amplitude 5`, `wavelength 44`, `speed 160` |
| `dissolve` | When the window closes, it falls apart into glowing blocks. | |

`island-glass`, `rain` and `thaw` turn on `background-effect { blur true; }` for their window unless the rule sets `blur` itself.
The window must be semitransparent to show the blur, for example kitty with `background_opacity 0.6`.
The blur is niri's xray blur, so behind the window you see the blurred wallpaper, not other windows.

The preset sources are in [`resources/shaders/`](../resources/shaders/), and are good starting points for your own.

## Writing a shader

Copy [`shaders/template.frag`](../shaders/template.frag); it describes every uniform in a comment.
A shader is GLSL ES 1.00, like the [custom animation shaders](./wiki/Configuration:-Animations.md#custom-shader).
It defines one function:

```glsl
vec4 window_color(vec3 coords_geo, vec3 size_geo) {
    vec4 color = niri_sample(coords_geo);
    return color * vec4(1.0, 0.8, 0.8, 1.0);
}
```

`coords_geo` is the position of the pixel in the tile: (0, 0) is the top left corner and (1, 1) the bottom right one.
With a `margin` it goes outside of that range.
`size_geo.xy` is the size of the tile in logical pixels.
The tile is the window with its border; the focus ring and the shadow are around it.
Return the color premultiplied by alpha, the way it comes out of `niri_tex`.

The names are the same as in the animation shaders, so code moves between them easily:

| Uniform | |
|---|---|
| `sampler2D niri_tex` | The tile as niri would draw it, premultiplied. |
| `mat3 niri_geo_to_tex` | `(niri_geo_to_tex * coords_geo).xy` is the texture coordinate. |
| `vec2 niri_geo_size` | Size of the tile in logical pixels. |
| `vec2 niri_size` | Size of the drawn area in physical pixels. |
| `float niri_scale` | Physical pixels per logical pixel. |
| `float niri_alpha` | Opacity that niri applies to your result. |
| `float niri_time` | Seconds since niri started, wraps every hour. |
| `float niri_focused` | 1 for the focused window, 0 for the others, eased over 250 ms. |
| `vec2 niri_cursor` | Pointer over the window, 0 to 1 like `coords_geo`, or -1 when it's elsewhere. |
| `float niri_close_progress` | 0, going to 1 while the window closes. |
| `float niri_random_seed` | A random number per window. |
| `vec4 niri_corner_radius` | Corner radii of the tile: top left, top right, bottom right, bottom left. |
| `float niri_border_width` | Border width, 0 without a border. |
| `vec4 niri_border_color` | Color of the border (or the focus ring without a border), premultiplied, between the inactive and active ones by focus. |
| `float niri_focus_time` | Seconds since the window got the focus, up to 2, or -1 if it never had it. The window is redrawn every frame for those 2 seconds. |
| `sampler2D niri_backdrop` | What's behind the window: niri's blur, when the window has blur. Read it with `niri_sample_backdrop(coords_geo)`. A shader that uses it gets `niri_tex` without the blur, and draws the backdrop itself, like the `rain` preset. |

Helper functions: `niri_sample(coords_geo)`, `niri_tile_sdf(coords_geo, size_geo)` (distance to the rounded edge of the tile, negative inside), `shoji_rrect_sdf(p, rect, radius)` and `shoji_coverage(sdf)`.

Any other `uniform float`, `vec2`, `vec3` or `vec4` you declare is yours to set from the config.
Uniforms that the config doesn't set are 0, except `time`, which niri sets to `niri_time`, and `progress`, which follows the close animation.

## Shaders from ShojiWM

Shaders from [ShojiWM](https://github.com/bea4dev/ShojiWM) and its configs work unchanged.
niri tells the format by the entry point:

| Entry point | Format |
|---|---|
| `vec4 shader_main(EffectContext effect)` | ShojiWM effect. Gets the `EffectContext` prelude, and its input texture is `tex`. |
| `vec4 shader_main(vec2 uv, vec2 rect_size)` | The older ShojiWM effect API. `tex` is what's behind the window, and the result goes behind it. |
| `vec4 paint_main(PaintContext ctx)` | ShojiWM decoration. Gets the `PaintContext` prelude and paints the frame of the window: niri's border is replaced by what it paints, and the window shows through the hole. |

```kdl
window-rule {
    match app-id="kitty"
    shader "~/shojiwm/noise_salt.frag" {
        uniform "noise_amount" 0.04
    }
}
```

A ShojiWM effect gets the window as `tex`, like `layerSource()`.
With `input "backdrop"` it gets what's behind the window instead, like `backdropSource()`, and the window goes on top of what it draws.
Most effects in ShojiWM configs, like liquid glass behind terminals, are of this kind; turn on blur for the window to give them the blurred backdrop:

```kdl
window-rule {
    match app-id="kitty"
    background-effect {
        blur true
    }
    shader "~/shojiwm/liquid-glass.frag" {
        input "backdrop"
        uniform "glass_radius_px" 10.0
    }
}
```

niri fills the context like this:

- `texture_uv`, `texture_size_phy_px`: the texture with the whole tile, or the backdrop with `input "backdrop"`.
- `frame_rect_phy_px`: the tile, `content_rect_phy_px`: the window inside the border.
- `frame_radius_phy_px`: corner radius of the tile, `frame_border_phy_px`: border width.
- `has_clip`, `clip_rect_phy_px`, `clip_radius_phy_px`: the window rounded corners when `clip-to-geometry true` is on.
- `scale`: scale of the output.

The result is drawn inside the frame shape; outside of it niri keeps the shadow and focus ring as they were.
For `PaintContext`, the node is the tile, its border is niri's border with its colors, and the hole is the window.
`niri_time`, `niri_focused`, `niri_cursor` and the rest work in all formats.

Uniforms that a ShojiWM config animates from TypeScript are tied to the clock in the config: `uniform "phase_01" time-scale=0.25`.
`time` is set to the clock unless the config sets it, and a `progress` that the config doesn't set follows the close animation, so ShojiWM's dissolve and fade effects play when the window closes.

[`examples/shojiwm-configs/`](../examples/shojiwm-configs/) runs the shaders of five ShojiWM configs by bea4dev, rainy-terminal, liquid-terminal, frozen-desktop, liquid-desktop and true-liquid-glass, in niri.
[niri-showcase](https://github.com/Sileyesi/niri-showcase) puts it all together in a desktop session with a QuickShell bar, and [SHELLS.md](SHELLS.md) is about other shells.

## Shader pipelines

Layer effects of ShojiWM, with several stages and textures that keep their contents between frames, are a `shader-pipeline` in a [layer rule](./wiki/Configuration:-Layer-Rules.md).
This is the water on the wallpaper of liquid-desktop:

```kdl
layer-rule {
    match layer="background"

    shader-pipeline {
        state "flow" scale=0.5
        pass "water-flow.frag" target="flow" repeat=4
        draw "water-surface.frag" {
            texture "flow" from="flow"
            texture "sharp" from="surface"
            uniform "emboss" 5
        }
    }
}
```

- `state "name" scale=0.5` is a 16-bit float texture at that part of the surface's size, which keeps its contents between frames.
- `pass "file.frag" target="name"` renders into a state every frame, `repeat` times. It gets the state as `tex`, or what `input="name"` says.
- `draw "file.frag"` draws the surface. It gets the surface as `tex`, or what `input="name"` says. With `behind=true` it goes behind the surface rather than in place of it.
- `texture "sampler" from="name"` gives a pass another texture: a state, `surface` for what the surface draws, or `backdrop` for niri's blur behind the surface.
- `uniform` works as in window shaders.

Passes are ShojiWM effect shaders.
When they declare these uniforms and the config doesn't set them, niri feeds them the way liquid-desktop's `water.ts` does:

| Uniform | |
|---|---|
| `float time`, `float dt` | Seconds on niri's clock, and seconds per frame, clamped from 2 to 17 ms and split between the `repeat` runs of the pass. |
| `vec4 window_rects[12]`, `vec2 window_motion[12]`, `float window_weight[12]`, `float window_count` | The windows on the output, from 0 to 1 over it, with their velocity in screens per second. |
| `vec4 pointer_state` | The pointer, from 0 to 1 over the output, and its velocity, or -10 when it's on another output. |
| `vec4 impulses[8]`, `float impulse_power[8]` | Splashes: position, radius and the time they start, with their strength. Windows splash when they open and close, and the `shader-pipeline-splash` action drops a stone at the pointer. |
| `float presence` | Rises while something moves and falls 9 seconds later. A pipeline whose draw pass uses it only runs while it's above 0, and the surface is drawn as usual otherwise. |

A pipeline that doesn't use `presence` runs whenever the surface is drawn.

A `shader-pipeline` works in a window rule too.
Every window gets its own states, the "output" is the window, so the pointer is relative to it, and there are no other windows.
This puts the water behind a transparent terminal, where the pointer makes waves under the text:

```kdl
window-rule {
    match app-id="^kitty$"
    background-effect {
        blur true
    }
    shader-pipeline {
        state "flow" scale=0.5
        pass "water-flow.frag" target="flow" repeat=4
        draw "water-surface.frag" input="backdrop" behind=true {
            texture "flow" from="flow"
            texture "sharp" from="backdrop"
        }
    }
}
```

`shader-pipeline-splash` drops a stone into the window under the pointer if it has a pipeline, and into the layer surfaces of the output otherwise.
A window `shader`, from the rules or set at runtime, goes over the window's pipeline, so `niri msg window-shader set` and `cycle-window-shader` work on such windows too.

## Changing shaders on the fly

niri watches the shader files from the config along with the config itself.
Save the file and the window is redrawn with the new version.

From the command line:

```sh
niri msg window-shader set crt                  # the focused window
niri msg window-shader set --id 12 ./my.frag    # a window by id, from `niri msg windows`
niri msg window-shader clear                    # back to the window rules
niri msg window-shader clear --none             # no shader, whatever the rules say
niri msg window-shader list                     # presets, and the windows with shaders
```

Files set this way are watched too.

The `cycle-window-shader` action goes through the presets on the focused window, then back to the window rules:

```kdl
binds {
    Mod+Shift+S { cycle-window-shader; }
}
```

## When something is wrong

A shader that doesn't compile doesn't break anything: the window is drawn without it.
The compiler output goes to the niri log, and `niri msg window-shader list` shows it next to the window:

```
Window ID 7:
  App ID: "kitty"
  Shader: /home/me/.config/niri/shaders/my.frag (from window rules)
  Error, drawn without the shader:
    0:12(31): error: `strenght' undeclared
```

`0:12(31)` is line 12, column 31 of your file.
`set` from the command line prints the same error right away.

The list also warns about uniforms from the config that the shader doesn't use, or that have a different number of values than the shader declares.
A uniform that the shader declares but never uses counts as unused, because the driver drops it.

## Performance

Windows without a shader are drawn the way niri always draws them.

A window with a shader is drawn to a texture first, like during the open animation, and then through the shader.
The texture only updates when the window changes.
niri checks after linking which uniforms the shader uses.
Shaders that use `niri_time` are redrawn every frame while their window is on screen; others only when the window changes, gets or loses focus, or, for shaders that use `niri_cursor`, when the pointer moves over it.

Programs are cached by the hash of their code, so ten kitty windows with one shader share one program.

## Limits

- Pipelines on layer surfaces read the backdrop through niri's xray blur, which shows the wallpaper but not the windows behind the surface.
- The ShojiWM effect format has no margin, it draws inside the frame.
- Non-xray blur (`xray false`) doesn't show up under a window with a shader, the same as during niri's open and close animations.
- The pointer is only seen over the window itself, not over its border.
- Shaders that use `niri_time` keep the GPU busy while visible. Four of them filling a 1696 × 1354 nested output took about 0.7 ms of render and GPU time per frame (1.7 ms at most) on a Radeon RX 6750 XT. This hasn't been measured on integrated graphics yet.
