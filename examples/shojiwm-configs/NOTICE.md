# Shaders from bea4dev's ShojiWM configs

The files in this directory are copied unchanged from three ShojiWM configurations by bea4dev:

| Directory | Repository | Commit |
|---|---|---|
| `rainy-terminal/` | https://github.com/bea4dev/rainy-terminal-config-shojiwm (`src/*.frag`) | `1c693f504c7cdb9ae36574ccc4763359e0552566`, 2026-06-24 |
| `true-liquid-glass/` | https://github.com/bea4dev/true-liquid-glass-shojiwm (`shojiwm/src/*.frag`, `LiquidIslandQS/`) | `f821a7e8ec8483b6486e1ed8ca682fd0d9717168`, 2026-09-14 |
| `liquid-desktop/` | https://github.com/bea4dev/liquid-desktop-shojiwm (`src/*.frag`) | `aebf0b7bff473ebe600806615a8e1e5cb0629342`, 2026-07-31 |

Two more configs ship the same files and only set them up differently, so nothing was copied from them:

| Same files as | Repository | Commit |
|---|---|---|
| `rainy-terminal/` | https://github.com/bea4dev/liquid-terminal-config-shojiwm (`src/*.frag`) | `682fdd56a2414db0266f26b633a90a695d10183b`, 2026-06-24 |
| `liquid-desktop/` | https://github.com/bea4dev/frozen-desktop-config-shojiwm (`src/*.frag`) | `552c7552c5b5184fe90e8805b928741c16e6a981`, 2026-07-11 |

`liquid-terminal.kdl` uses the uniforms of the first one, `frozen-desktop.kdl` and the frozen window of niri-showcase those of the second one.

**None of these repositories has a license**, so this directory is not covered by niri's GPL: the code stays bea4dev's.
It is here, with credit to its author, to show that niri runs these shaders as they are.
niri itself doesn't need it, and nothing here is built into the niri binary.

bea4dev, if you'd rather not have your shaders here, open an issue and they'll be taken out.

The niri configs that use them are next to this file, see `README.md`.
