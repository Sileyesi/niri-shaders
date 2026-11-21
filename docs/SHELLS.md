# Shells, launchers and terminals

The shaders show what's behind a window or a panel, so they need a desktop where things are see-through.
This page is about the programs around niri that go well with them, and what to set in each.

## Desktop shells

| Shell | |
|---|---|
| [QuickShell](https://quickshell.org/) | Bars, docks and launchers written in QML. Every surface has a layer namespace you choose, so layer rules can put blur, a shader or a whole shader pipeline behind exactly that surface. [niri-showcase](https://github.com/Sileyesi/niri-showcase) is a complete desktop on it, with liquid glass behind the bar, the dock and the launcher. |
| [DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell), [Noctalia](https://github.com/noctalia-dev/noctalia-shell) | Complete desktops built on QuickShell that support niri. Turn down the opacity of their panels in their settings, then add a layer rule for the namespaces of the panels. |
| [Waybar](https://github.com/Alexays/Waybar) | A bar. Its namespace is `waybar`; give the bar a background with alpha in its CSS, like `background: rgba(20, 20, 30, 0.4);`. |

Whatever the shell, `niri msg layers` lists the namespaces of its surfaces.
Then a layer rule gives them blur, or the glass of the showcase:

```kdl
layer-rule {
    match namespace="^waybar$"
    background-effect {
        blur true
    }
}
```

The blur, the shaders and the pipelines only show where the surface itself is see-through: a panel with an opaque background hides them.
And let niri do the blur: a shell that blurs on its own, behind a niri blur, looks muddy.
See [Shader pipelines](SHADERS.md#shader-pipelines) for glass on layer surfaces.

## Launchers and notifications

[fuzzel](https://codeberg.org/dnkl/fuzzel) (namespace `launcher`) and [mako](https://github.com/emersion/mako) (namespace `notifications`) both take a background color with alpha, and work with the same layer rules.
niri blurs their pop-ups too with `popups { background-effect { blur true; } }` inside the layer rule.

## Terminals

The shaders that work over what's behind the window, like `thaw`, `rain`, `island-glass` and ShojiWM's liquid glass, frozen glass and water, need a terminal with a see-through background:

| Terminal | Setting |
|---|---|
| [kitty](https://sw.kovidgoyal.net/kitty/) | `background_opacity 0.2`. With `dynamic_background_opacity yes`, Ctrl+Shift+A then M or L changes it while the terminal runs. |
| [Ghostty](https://ghostty.org/) | `background-opacity = 0.2` |
| [foot](https://codeberg.org/dnkl/foot) | `alpha=0.2` in the `[colors]` section |

The lower the opacity, the more of the shader shows; `0` leaves only the text over the glass.
The shaders that work over the window itself, like `crt`, `hologram` and `matrix`, look best on an opaque terminal.
