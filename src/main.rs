use clap::{Parser, ValueEnum};
use image::{ImageBuffer, Rgb, RgbaImage};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Single,
    Dual,
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Sega Genesis/Mega Drive image quantizer (Single 16-color or Dual 31-color) using Oklab space"
)]
struct Args {
    /// Input image file (.png, .bmp, .ase, .aseprite, etc.)
    #[arg(short, long)]
    input: PathBuf,

    /// Output base path or prefix
    #[arg(short, long)]
    output: PathBuf,

    /// Quantization mode: single (15 colors + transparency) or dual (31 colors across 2 planes)
    #[arg(short, long, value_enum, default_value_t = Mode::Dual)]
    mode: Mode,

    /// Disable Floyd-Steinberg dithering
    #[arg(long, default_value_t = false)]
    no_dither: bool,

    /// Frame index (if the input is an Aseprite file)
    #[arg(long, default_value_t = 0)]
    frame: usize,
}

// Oklab color space representation
#[derive(Clone, Copy, Debug)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

impl Oklab {
    fn dist_sq(&self, other: &Oklab) -> f32 {
        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;
        dl * dl + da * da + db * db
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn rgb_to_oklab(r: u8, g: u8, b: u8) -> Oklab {
    let r_lin = srgb_to_linear(r as f32 / 255.0);
    let g_lin = srgb_to_linear(g as f32 / 255.0);
    let b_lin = srgb_to_linear(b as f32 / 255.0);

    let l = 0.41222147 * r_lin + 0.53633255 * g_lin + 0.05144599 * b_lin;
    let m = 0.21190350 * r_lin + 0.68069955 * g_lin + 0.10739696 * b_lin;
    let s = 0.08830246 * r_lin + 0.28171884 * g_lin + 0.62997870 * b_lin;

    let l_ = l.max(0.0).cbrt();
    let m_ = m.max(0.0).cbrt();
    let s_ = s.max(0.0).cbrt();

    Oklab {
        l: 0.21045426 * l_ + 0.79361780 * m_ - 0.00407205 * s_,
        a: 1.97799850 * l_ - 2.42859220 * m_ + 0.45059370 * s_,
        b: 0.02590404 * l_ + 0.78277177 * m_ - 0.80867577 * s_,
    }
}

// Sega Genesis/Mega Drive 512 nominal RGB333 master palette
fn generate_genesis_palette() -> (Vec<[u8; 3]>, Vec<Oklab>) {
    let levels = [0u8, 36, 73, 109, 146, 182, 219, 255];
    let mut rgbs = Vec::with_capacity(512);
    let mut oklabs = Vec::with_capacity(512);

    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                rgbs.push([r, g, b]);
                oklabs.push(rgb_to_oklab(r, g, b));
            }
        }
    }
    (rgbs, oklabs)
}

fn to_vdp_hex(rgb: [u8; 3]) -> String {
    let r = ((rgb[0] as f32 / 36.42).round() as u32 * 2) & 0x0E;
    let g = ((rgb[1] as f32 / 36.42).round() as u32 * 2) & 0x0E;
    let b = ((rgb[2] as f32 / 36.42).round() as u32 * 2) & 0x0E;
    format!("0x0{:X}{:X}{:X}", b, g, r)
}

fn load_input_image(path: &Path, frame: usize) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext == "ase" || ext == "aseprite" {
        println!("[INFO] Aseprite file detected. Invoking Aseprite CLI...");
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("ase_export_{}.png", std::process::id()));

        let status = Command::new("aseprite")
            .args([
                "-b",
                path.to_str().unwrap(),
                "--frame-tag",
                &frame.to_string(),
                "--save-as",
                temp_file.to_str().unwrap(),
            ])
            .status();

        let success = match status {
            Ok(s) if s.success() && temp_file.exists() => true,
            _ => {
                // Try without the --frame-tag flag
                let status_alt = Command::new("aseprite")
                    .args([
                        "-b",
                        path.to_str().unwrap(),
                        "--save-as",
                        temp_file.to_str().unwrap(),
                    ])
                    .status();
                matches!(status_alt, Ok(s) if s.success() && temp_file.exists())
            }
        };

        if !success {
            return Err(
                "Failed to run Aseprite CLI. Make sure 'aseprite' is available in your PATH."
                    .into(),
            );
        }

        let img = image::open(&temp_file)
            .map_err(|e| format!("Error opening temporary PNG: {}", e))?
            .to_rgb8();
        let _ = std::fs::remove_file(temp_file);
        Ok(img)
    } else {
        image::open(path)
            .map_err(|e| format!("Error opening image: {}", e))
            .map(|img| img.to_rgb8())
    }
}

// Fast K-Means algorithm in Oklab color space
fn kmeans_oklab(pixels: &[Oklab], k: usize, iters: usize) -> Vec<Oklab> {
    let mut rng = thread_rng();
    let mut centroids: Vec<Oklab> = pixels.choose_multiple(&mut rng, k).cloned().collect();

    while centroids.len() < k {
        centroids.push(pixels[0]);
    }

    let mut assignments = vec![0usize; pixels.len()];

    for _ in 0..iters {
        let mut changed = false;
        // Assignment step
        for (i, p) in pixels.iter().enumerate() {
            let mut best_dist = f32::MAX;
            let mut best_c = 0;
            for (ci, c) in centroids.iter().enumerate() {
                let d = p.dist_sq(c);
                if d < best_dist {
                    best_dist = d;
                    best_c = ci;
                }
            }
            if assignments[i] != best_c {
                assignments[i] = best_c;
                changed = true;
            }
        }

        if !changed {
            break;
        }

        // Update step
        let mut sums_l = vec![0.0f32; k];
        let mut sums_a = vec![0.0f32; k];
        let mut sums_b = vec![0.0f32; k];
        let mut counts = vec![0usize; k];

        for (i, p) in pixels.iter().enumerate() {
            let c = assignments[i];
            sums_l[c] += p.l;
            sums_a[c] += p.a;
            sums_b[c] += p.b;
            counts[c] += 1;
        }

        for c in 0..k {
            if counts[c] > 0 {
                let n = counts[c] as f32;
                centroids[c] = Oklab {
                    l: sums_l[c] / n,
                    a: sums_a[c] / n,
                    b: sums_b[c] / n,
                };
            }
        }
    }

    centroids
}

fn quantize_single_plane(
    img: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    output_prefix: &Path,
    dither: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n[MODE: SINGLE PLANE (15 COLORS + TRANSPARENCY)]");
    let (width, height) = img.dimensions();
    let (gen_rgbs, gen_oklabs) = generate_genesis_palette();

    println!("[1/3] Identifying 15 dominant colors in Oklab space...");
    let pixels_oklab: Vec<Oklab> = img
        .pixels()
        .map(|p| rgb_to_oklab(p[0], p[1], p[2]))
        .collect();

    let centroids = kmeans_oklab(&pixels_oklab, 15, 20);

    let mut pal_indices = Vec::new();
    for c in &centroids {
        let mut best_d = f32::MAX;
        let mut best_i = 0;
        for (i, g) in gen_oklabs.iter().enumerate() {
            let d = c.dist_sq(g);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        if !pal_indices.contains(&best_i) {
            pal_indices.push(best_i);
        }
    }

    let mut fill_idx = 0;
    while pal_indices.len() < 15 && fill_idx < 512 {
        if !pal_indices.contains(&fill_idx) {
            pal_indices.push(fill_idx);
        }
        fill_idx += 1;
    }

    let pal_15_rgb: Vec<[u8; 3]> = pal_indices.iter().map(|&i| gen_rgbs[i]).collect();
    let pal_15_oklab: Vec<Oklab> = pal_indices.iter().map(|&i| gen_oklabs[i]).collect();

    let mut out_rgba = RgbaImage::new(width, height);
    let mut buffer = pixels_oklab;

    println!("[2/3] Mapping pixels...");
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let current = buffer[idx];

            let mut best_d = f32::MAX;
            let mut best_i = 0;
            for (i, c) in pal_15_oklab.iter().enumerate() {
                let d = current.dist_sq(c);
                if d < best_d {
                    best_d = d;
                    best_i = i;
                }
            }

            let chosen_rgb = pal_15_rgb[best_i];
            let chosen_oklab = pal_15_oklab[best_i];

            out_rgba.put_pixel(
                x,
                y,
                image::Rgba([chosen_rgb[0], chosen_rgb[1], chosen_rgb[2], 255]),
            );

            if dither {
                let err_l = (current.l - chosen_oklab.l) * 0.75;
                let err_a = (current.a - chosen_oklab.a) * 0.75;
                let err_b = (current.b - chosen_oklab.b) * 0.75;

                let apply_err = |target: &mut Oklab, weight: f32| {
                    target.l = (target.l + err_l * weight).clamp(0.0, 1.0);
                    target.a = (target.a + err_a * weight).clamp(-0.4, 0.4);
                    target.b = (target.b + err_b * weight).clamp(-0.4, 0.4);
                };

                if x + 1 < width {
                    apply_err(&mut buffer[idx + 1], 7.0 / 16.0);
                }
                if y + 1 < height {
                    if x > 0 {
                        apply_err(&mut buffer[idx + width as usize - 1], 3.0 / 16.0);
                    }
                    apply_err(&mut buffer[idx + width as usize], 5.0 / 16.0);
                    if x + 1 < width {
                        apply_err(&mut buffer[idx + width as usize + 1], 1.0 / 16.0);
                    }
                }
            }
        }
    }

    println!("[3/3] Writing output files...");
    let base_str = output_prefix.to_str().unwrap().trim_end_matches(".png");
    let out_png = format!("{}_single.png", base_str);
    let out_txt = format!("{}_palette.txt", base_str);

    out_rgba.save(&out_png)?;

    let mut txt_file = File::create(&out_txt)?;
    writeln!(txt_file, "; 16-color palette (0 = Transparent)")?;
    let mut words = vec!["0x0000".to_string()];
    for c in pal_15_rgb {
        words.push(to_vdp_hex(c));
    }
    writeln!(txt_file, "VDP_PALETTE={}", words.join(","))?;

    println!("[OK] Completed:");
    println!("  -> Equivalent indexed PNG: {}", out_png);
    println!("  -> VDP palette:            {}", out_txt);
    Ok(())
}

fn quantize_dual_plane(
    img: &ImageBuffer<Rgb<u8>, Vec<u8>>,
    output_prefix: &Path,
    dither: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n[MODE: DUAL PLANE (31 SHARED COLORS)]");
    let (width, height) = img.dimensions();
    let (gen_rgbs, gen_oklabs) = generate_genesis_palette();

    println!("[1/4] Clustering 31 colors in Oklab space...");
    let pixels_oklab: Vec<Oklab> = img
        .pixels()
        .map(|p| rgb_to_oklab(p[0], p[1], p[2]))
        .collect();

    let centroids = kmeans_oklab(&pixels_oklab, 31, 25);

    let mut pal_indices = Vec::new();
    for c in &centroids {
        let mut best_d = f32::MAX;
        let mut best_i = 0;
        for (i, g) in gen_oklabs.iter().enumerate() {
            let d = c.dist_sq(g);
            if d < best_d {
                best_d = d;
                best_i = i;
            }
        }
        if !pal_indices.contains(&best_i) {
            pal_indices.push(best_i);
        }
    }

    let mut fill_idx = 0;
    while pal_indices.len() < 31 && fill_idx < 512 {
        if !pal_indices.contains(&fill_idx) {
            pal_indices.push(fill_idx);
        }
        fill_idx += 1;
    }

    let pal_31_rgb: Vec<[u8; 3]> = pal_indices.iter().map(|&i| gen_rgbs[i]).collect();
    let pal_31_oklab: Vec<Oklab> = pal_indices.iter().map(|&i| gen_oklabs[i]).collect();

    println!("[2/4] Determining Backdrop color and partitioning PAL0 and PAL1...");
    // Select the color with the lowest lightness among the first candidates as Backdrop
    let mut bg_idx = 0;
    let mut min_l = f32::MAX;
    for i in 0..pal_31_oklab.len().min(5) {
        if pal_31_oklab[i].l < min_l {
            min_l = pal_31_oklab[i].l;
            bg_idx = i;
        }
    }

    let bg_rgb = pal_31_rgb[bg_idx];
    let bg_oklab = pal_31_oklab[bg_idx];

    let mut remaining_rgb = Vec::new();
    let mut remaining_oklab = Vec::new();
    for i in 0..31 {
        if i != bg_idx {
            remaining_rgb.push(pal_31_rgb[i]);
            remaining_oklab.push(pal_31_oklab[i]);
        }
    }

    // Split into 2 clusters of 15 colors
    let split_centers = kmeans_oklab(&remaining_oklab, 2, 10);
    let mut pal0_rgb = Vec::new();
    let mut pal0_oklab = Vec::new();
    let mut pal1_rgb = Vec::new();
    let mut pal1_oklab = Vec::new();

    for i in 0..30 {
        let d0 = remaining_oklab[i].dist_sq(&split_centers[0]);
        let d1 = remaining_oklab[i].dist_sq(&split_centers[1]);
        if d0 <= d1 {
            pal0_rgb.push(remaining_rgb[i]);
            pal0_oklab.push(remaining_oklab[i]);
        } else {
            pal1_rgb.push(remaining_rgb[i]);
            pal1_oklab.push(remaining_oklab[i]);
        }
    }

    while pal0_rgb.len() > 15 {
        pal1_rgb.push(pal0_rgb.pop().unwrap());
        pal1_oklab.push(pal0_oklab.pop().unwrap());
    }
    while pal1_rgb.len() > 15 {
        pal0_rgb.push(pal1_rgb.pop().unwrap());
        pal0_oklab.push(pal1_oklab.pop().unwrap());
    }

    // Assemble ordered catalog of 31 options
    let mut full_31_rgb = vec![bg_rgb];
    full_31_rgb.extend(&pal0_rgb);
    full_31_rgb.extend(&pal1_rgb);

    let mut full_31_oklab = vec![bg_oklab];
    full_31_oklab.extend(&pal0_oklab);
    full_31_oklab.extend(&pal1_oklab);

    println!("[3/4] Mapping pixels across Plane A and Plane B...");
    let mut comp_rgba = RgbaImage::new(width, height);
    let mut plane_b_rgba = RgbaImage::new(width, height);
    let mut plane_a_rgba = RgbaImage::new(width, height);

    let mut buffer = pixels_oklab;

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let current = buffer[idx];

            let mut best_d = f32::MAX;
            let mut best_i = 0;
            for (i, c) in full_31_oklab.iter().enumerate() {
                let d = current.dist_sq(c);
                if d < best_d {
                    best_d = d;
                    best_i = i;
                }
            }

            let chosen_rgb = full_31_rgb[best_i];
            let chosen_oklab = full_31_oklab[best_i];

            comp_rgba.put_pixel(
                x,
                y,
                image::Rgba([chosen_rgb[0], chosen_rgb[1], chosen_rgb[2], 255]),
            );

            if best_i == 0 {
                // Transparent in both planes -> shows Backdrop
                plane_a_rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                plane_b_rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            } else if best_i <= 15 {
                // Plane B active (PAL0), Plane A transparent
                plane_a_rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                plane_b_rgba.put_pixel(
                    x,
                    y,
                    image::Rgba([chosen_rgb[0], chosen_rgb[1], chosen_rgb[2], 255]),
                );
            } else {
                // Plane A active (PAL1), Plane B clear
                plane_a_rgba.put_pixel(
                    x,
                    y,
                    image::Rgba([chosen_rgb[0], chosen_rgb[1], chosen_rgb[2], 255]),
                );
                plane_b_rgba.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }

            if dither {
                let err_l = (current.l - chosen_oklab.l) * 0.75;
                let err_a = (current.a - chosen_oklab.a) * 0.75;
                let err_b = (current.b - chosen_oklab.b) * 0.75;

                let apply_err = |target: &mut Oklab, weight: f32| {
                    target.l = (target.l + err_l * weight).clamp(0.0, 1.0);
                    target.a = (target.a + err_a * weight).clamp(-0.4, 0.4);
                    target.b = (target.b + err_b * weight).clamp(-0.4, 0.4);
                };

                if x + 1 < width {
                    apply_err(&mut buffer[idx + 1], 7.0 / 16.0);
                }
                if y + 1 < height {
                    if x > 0 {
                        apply_err(&mut buffer[idx + width as usize - 1], 3.0 / 16.0);
                    }
                    apply_err(&mut buffer[idx + width as usize], 5.0 / 16.0);
                    if x + 1 < width {
                        apply_err(&mut buffer[idx + width as usize + 1], 1.0 / 16.0);
                    }
                }
            }
        }
    }

    println!("[4/4] Writing output files and descriptor...");
    let base_str = output_prefix.to_str().unwrap().trim_end_matches(".png");
    let out_comp = format!("{}_composite.png", base_str);
    let out_b = format!("{}_plane_b.png", base_str);
    let out_a = format!("{}_plane_a.png", base_str);
    let out_txt = format!("{}_backdrop.txt", base_str);

    comp_rgba.save(&out_comp)?;
    plane_b_rgba.save(&out_b)?;
    plane_a_rgba.save(&out_a)?;

    let vdp_hex = to_vdp_hex(bg_rgb);
    let mut txt_file = File::create(&out_txt)?;
    writeln!(txt_file, "VDP_REG7_BACKDROP_COLOR={}", vdp_hex)?;
    writeln!(
        txt_file,
        "RGB_8BIT={},{},{}",
        bg_rgb[0], bg_rgb[1], bg_rgb[2]
    )?;
    writeln!(txt_file, "C_CODE=VDP_setBackgroundColor(0);")?;
    writeln!(
        txt_file,
        "ASM_CODE=move.w #{}, ($C00000)",
        vdp_hex.replace("0x", "$")
    )?;

    println!("[OK] Completed:");
    println!("  -> Backdrop VDP: {}", vdp_hex);
    println!("  -> Descriptor:   {}", out_txt);
    println!("  -> Composite:    {}", out_comp);
    println!("  -> Plane B:      {}", out_b);
    println!("  -> Plane A:      {}", out_a);

    Ok(())
}

fn main() {
    let args = Args::parse();

    let img = match load_input_image(&args.input, args.frame) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("[FATAL ERROR] {}", e);
            std::process::exit(1);
        }
    };

    let result = match args.mode {
        Mode::Single => quantize_single_plane(&img, &args.output, !args.no_dither),
        Mode::Dual => quantize_dual_plane(&img, &args.output, !args.no_dither),
    };

    if let Err(e) = result {
        eprintln!("[FATAL ERROR] Processing failed: {}", e);
        std::process::exit(1);
    }
}