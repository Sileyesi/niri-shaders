# ShojiWM configs in niri

The shaders of five ShojiWM configurations by bea4dev, running in niri as they are.
See `NOTICE.md` about where they come from: the repositories have no license.

Build niri, then run a demo in a nested window:

```sh
cargo build --release --no-default-features --features dbus,systemd
examples/shojiwm-configs/run.sh liquid-desktop ~/Pictures/wallpaper.png
```

The wallpaper is optional, but the glass and water look like much more with one.
Nested, the Mod key is Alt.

| Demo | What it shows | ShojiWM config |
|---|---|---|
| `rainy-terminal` | Rain on the blurred glass behind transparent terminals. | [rainy-terminal-config-shojiwm](https://github.com/bea4dev/rainy-terminal-config-shojiwm) |
| `liquid-terminal` | Water running across the blurred glass behind transparent terminals. | [liquid-terminal-config-shojiwm](https://github.com/bea4dev/liquid-terminal-config-shojiwm) |
| `frozen-desktop` | The blurred glass behind transparent terminals frozen into big crystals. | [frozen-desktop-config-shojiwm](https://github.com/bea4dev/frozen-desktop-config-shojiwm) |
| `liquid-desktop` | The wallpaper is water: windows and the pointer make waves, windows splash when they open and close, Mod+I drops a stone. Terminals have liquid glass, and three more windows show water-terminal, frozen-glass and electric-frame. | [liquid-desktop-shojiwm](https://github.com/bea4dev/liquid-desktop-shojiwm) |
| `water-window` | The water of `liquid-desktop` on the terminals instead of the wallpaper: the pointer makes waves behind the text. | [liquid-desktop-shojiwm](https://github.com/bea4dev/liquid-desktop-shojiwm) |
| `true-liquid-glass` | The LiquidIslandQS bar (QuickShell; click it) with the true liquid glass behind it, and liquid glass behind the terminals. | [true-liquid-glass-shojiwm](https://github.com/bea4dev/true-liquid-glass-shojiwm) |

Every demo has a 2 px gold border on the focused window and blur behind layer surfaces, like the configs, and Mod+Shift+S cycles niri's own shader presets on the focused window.

## How the ShojiWM pieces map to niri

- `compileEffect({ input: backdropSource(), pipeline: [dualKawaseBlur(...), shaderStage(...)] })` around a window is `shader "file.frag" { input "backdrop"; }` with `background-effect { blur true; }` in a window rule.
  niri's blur is the dual kawase blur, set with `blur { passes 2; offset 4; }`.
- Uniforms set in the TypeScript config are `uniform "name" ...` lines.
  The ones a config animates, like `time`, are tied to niri's clock: `time` by itself, others with `uniform "phase_01" time-scale=0.25`.
- Layer effects with several stages and state textures (`renderTo`, `stateTexture`, `save`, `get`) are a `shader-pipeline` in a layer rule: `state` for every texture, `pass` for every stage, and `draw` for the last one.
  A `shader-pipeline` works in window rules too, with the window in place of the output.
  For the water of liquid-desktop, niri itself feeds what `water.ts` computes: `dt`, the window rects and their motion, the pointer and the splashes.
- `dualKawaseBlur` stages inside the island glass pipeline are niri's blur of the backdrop.

## What is different

- niri draws no title bars, so the title bar with its round buttons isn't there.
- niri's blur behind windows and layer surfaces is xray blur: it shows the wallpaper and not the windows behind.
  That's why the bar of `true-liquid-glass` sits above the windows.
- The water ignores the workspace switch animation of liquid-desktop's window manager, which niri doesn't have.
