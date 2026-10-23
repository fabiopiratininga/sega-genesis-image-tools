# SGIT — Sega Genesis Image Tools

Fast, native image quantizer and color pipeline tool written in **Rust**, designed specifically for the **Sega Genesis / Mega Drive VDP** (Video Display Processor).

It maps high-color images (`.png`, `.bmp`, `.ase`, `.aseprite`) down to the console's physical **9-bit RGB333 master palette (512 colors)** using the perceptual **Oklab** color space.

The tool supports both traditional single-palette assets (sprites/tiles) and full dual-plane compositions (**Scroll A + Scroll B + Backdrop**) unlocking **31 simultaneous colors** on screen.

---

## Features

- **Perceptual Oklab Quantization:** Calculates color distance in Oklab space to prevent unwanted color shifts and preserve smooth luminance ramps.
- **Genesis Hardware Palette Constraint:** Clamps and snaps colors to the native 512-color RGB333 master grid (`0, 36, 73, 109, 146, 182, 219, 255`).
- **Two Operation Modes:**
  - `single`: Quantizes an image to **15 colors + 1 transparent index (slot 0)** for conventional sprite/tile usage.
  - `dual`: Partitions **30 colors across two palettes (15 in PAL0, 15 in PAL1)** and selects a shared **Backdrop color (index 0)** for VDP Register 7, rendering **31 visible physical colors** across Plane A and Plane B.
- **Damped Floyd-Steinberg Dithering:** Incorporates error-decay clamping (`0.75x`) to prevent noise worming and chromatic artifacts.
- **Native Aseprite Support:** Automatically invokes the Aseprite CLI in batch mode to read `.ase` and `.aseprite` work files directly without manual export.
- **VDP Ready Outputs:** Generates plane-separated images, composite previews, and text descriptors containing C/SGDK setup functions and 68000 Assembly code.

---

## Prerequisites

- **Rust & Cargo** (Edition 2024 — see `Cargo.toml`)
- **Aseprite (Optional):** Required only if you pass `.ase` or `.aseprite` files as inputs. Ensure the `aseprite` binary is in your system's `PATH`.
- **UPX (Optional):** Required only if you want to compress the final release executable down.
- **cargo-make (Optional):** Required for the `cargo make dist` UPX shortcut defined in `Makefile.toml`.

---

## Building from Source

### Standard Build

Clone the repository and build the project using Cargo:

```bash
git clone https://github.com/your-username/sega-genesis-image-tools.git
cd sega-genesis-image-tools
cargo build --release
```

The optimized binary will be available at:

- Windows: `target/release/sgit.exe`
- Linux: `target/release/sgit`

Release profile uses `opt-level = "z"`, `lto = true`, `strip = true` for minimal size.

### Building with Automatic UPX Compression

This repository includes a `cargo-make` task in `Makefile.toml` that compiles in release mode and automatically runs UPX:

```bash
cargo make dist
```

> Note: If UPX is not found in your `PATH`, the build step still succeeds — only the compression step fails.

---

## Usage

```bash
sgit [OPTIONS] --input <INPUT> --output <OUTPUT>
```

### Options

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--input` | `-i` | Input image file (`.png`, `.bmp`, `.ase`, `.aseprite`) | *Required* |
| `--output` | `-o` | Output file path or prefix | *Required* |
| `--mode` | `-m` | Quantization mode: `dual` (31 colors) or `single` (15 colors + transparency) | `dual` |
| `--no-dither` | | Disables Floyd-Steinberg dithering for clean flat colors | `false` |
| `--frame` | | Frame index to extract when opening an Aseprite file | `0` |
| `--help` | `-h` | Prints help and command options | |
| `--version` | `-V` | Prints version | |

---

## Examples

### 1. Dual-Plane Screen (31 colors for Title / Select Screens)

Maps the image across Scroll A, Scroll B, and the Backdrop color:

```bash
sgit -i title_screen.png -o title_screen -m dual
```

Generated files:

- `title_screen_composite.png`: Full color simulation.
- `title_screen_plane_b.png`: Background layer mapped to PAL0.
- `title_screen_plane_a.png`: Foreground layer mapped to PAL1.
- `title_screen_backdrop.txt`: VDP Register 7 configuration and color hex values.

### 2. Single-Plane Asset (15 colors + Transparency)

Quantizes sprites or single-layer background tilesets:

```bash
sgit -i character_sprite.aseprite -o character_out -m single
```

Generated files:

- `character_out_single.png`: 16-color indexed image (index 0 is transparent).
- `character_out_palette.txt`: VDP hex palette table (`VDP_PALETTE=0x0000,...`).

With explicit Aseprite frame selection:

```bash
sgit -i character_sprite.aseprite -o character_out -m single --frame 2
```

### 3. Flat Color Quantization (No Dithering)

```bash
sgit -i ui_elements.png -o ui_clean -m single --no-dither
```

---

## Integration with Mega Drive Projects

### VDP Register 7 Descriptor Output

When operating in `dual` mode, the generated `*_backdrop.txt` contains ready-to-use definitions:

```text
VDP_REG7_BACKDROP_COLOR=0x0000
RGB_8BIT=0,0,0
C_CODE=VDP_setBackgroundColor(0);
ASM_CODE=move.w #$0000, ($C00000)
```

In your SGDK project, load the Plane B palette into PAL0, Plane A palette into PAL1, and point the backdrop register to entry 0:

```c
VDP_setBackgroundColor(0);
VDP_setPalette(PAL0, plane_b_palette);
VDP_setPalette(PAL1, plane_a_palette);
```

---

## License

This project is licensed under the MIT License — see the `LICENSE` file for details.
