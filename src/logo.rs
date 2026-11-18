// RustWolf's original logo: a geometric, front-facing red wolf head.
// Dependency-free so build.rs can `include!` it to make the .ico, and main.rs can
// emit the same shape as SVG for the start page.

/// Points are in a 100 x 100 box.
type Poly = &'static [(f32, f32)];

const HEAD: Poly = &[
    (50.0, 24.0), (60.0, 28.0), (80.0, 5.0), (85.0, 37.0), (96.0, 50.0), (86.0, 55.0), (92.0, 66.0),
    (76.0, 70.0), (62.0, 90.0), (50.0, 97.0), (38.0, 90.0), (24.0, 70.0), (8.0, 66.0), (14.0, 55.0),
    (4.0, 50.0), (15.0, 37.0), (20.0, 5.0), (40.0, 28.0),
];
const EAR_L: Poly = &[(23.0, 14.0), (36.0, 30.0), (26.0, 37.0)];
const EAR_R: Poly = &[(77.0, 14.0), (64.0, 30.0), (74.0, 37.0)];
const BROW: Poly = &[(50.0, 27.0), (57.0, 39.0), (50.0, 50.0), (43.0, 39.0)];
const MASK: Poly = &[(50.0, 50.0), (65.0, 60.0), (60.0, 80.0), (50.0, 91.0), (40.0, 80.0), (35.0, 60.0)];
const CHEEK_L: Poly = &[(8.0, 66.0), (24.0, 70.0), (20.0, 58.0)];
const CHEEK_R: Poly = &[(92.0, 66.0), (76.0, 70.0), (80.0, 58.0)];
const EYE_L: Poly = &[(27.0, 47.0), (43.0, 53.0), (32.0, 56.0)];
const EYE_R: Poly = &[(73.0, 47.0), (57.0, 53.0), (68.0, 56.0)];
const NOSE: Poly = &[(43.0, 80.0), (57.0, 80.0), (50.0, 88.0)];

#[derive(Clone, Copy)]
enum Fill {
    Solid([u8; 4]),
    /// Vertical gradient (top color, bottom color) over the 100-unit box.
    Grad([u8; 4], [u8; 4]),
}

const LAYERS: &[(Poly, Fill)] = &[
    (HEAD, Fill::Grad([255, 77, 77, 255], [168, 16, 26, 255])),
    (EAR_L, Fill::Solid([92, 10, 16, 255])),
    (EAR_R, Fill::Solid([92, 10, 16, 255])),
    (BROW, Fill::Solid([190, 20, 31, 255])),
    (CHEEK_L, Fill::Solid([140, 12, 22, 255])),
    (CHEEK_R, Fill::Solid([140, 12, 22, 255])),
    (MASK, Fill::Grad([255, 112, 92, 255], [214, 40, 48, 255])),
    (EYE_L, Fill::Solid([255, 241, 236, 255])),
    (EYE_R, Fill::Solid([255, 241, 236, 255])),
    (NOSE, Fill::Solid([26, 5, 8, 255])),
];

fn inside(poly: Poly, x: f32, y: f32) -> bool {
    let mut c = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            c = !c;
        }
        j = i;
    }
    c
}

fn color_at(f: Fill, y: f32) -> [f32; 4] {
    match f {
        Fill::Solid(c) => c.map(|v| v as f32),
        Fill::Grad(a, b) => {
            let t = (y / 100.0).clamp(0.0, 1.0);
            [0, 1, 2, 3].map(|i| a[i] as f32 + (b[i] as f32 - a[i] as f32) * t)
        }
    }
}

/// Straight (non-premultiplied) RGBA, row-major, top row first. Transparent background.
pub fn render_rgba(size: usize) -> Vec<u8> {
    let ss = 4usize;
    let pad = size as f32 * 0.02;
    let scale = (size as f32 - 2.0 * pad) / 100.0;
    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = ((px as f32 + (sx as f32 + 0.5) / ss as f32) - pad) / scale;
                    let y = ((py as f32 + (sy as f32 + 0.5) / ss as f32) - pad) / scale;
                    let mut col: Option<[f32; 4]> = None;
                    for (poly, fill) in LAYERS {
                        if inside(poly, x, y) {
                            col = Some(color_at(*fill, y));
                        }
                    }
                    if let Some(c) = col {
                        acc[0] += c[0];
                        acc[1] += c[1];
                        acc[2] += c[2];
                        acc[3] += 255.0;
                    }
                }
            }
            let n = (ss * ss) as f32;
            let i = (py * size + px) * 4;
            if acc[3] > 0.0 {
                let cover = acc[3] / 255.0;
                out[i] = (acc[0] / cover) as u8;
                out[i + 1] = (acc[1] / cover) as u8;
                out[i + 2] = (acc[2] / cover) as u8;
                out[i + 3] = (acc[3] / n) as u8;
            }
        }
    }
    out
}

/// A Windows .ico with 32-bit BMP images at the given sizes.
pub fn make_ico(sizes: &[usize]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &s in sizes {
        let rgba = render_rgba(s);
        let mut img = Vec::new();
        let le32 = |v: u32| v.to_le_bytes();
        img.extend_from_slice(&le32(40));
        img.extend_from_slice(&le32(s as u32));
        img.extend_from_slice(&le32((s * 2) as u32));
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        for _ in 0..6 {
            img.extend_from_slice(&le32(0));
        }
        for y in (0..s).rev() {
            for x in 0..s {
                let i = (y * s + x) * 4;
                img.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        let mask_row = ((s + 31) / 32) * 4;
        img.extend(std::iter::repeat(0u8).take(mask_row * s));
        images.push(img);
    }
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (k, &s) in sizes.iter().enumerate() {
        let d = if s >= 256 { 0u8 } else { s as u8 };
        ico.extend_from_slice(&[d, d, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(images[k].len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += images[k].len();
    }
    for img in images {
        ico.extend_from_slice(&img);
    }
    ico
}

/// The same logo as an inline SVG (for the start page).
#[allow(dead_code)]
pub fn svg(px: u32) -> String {
    let hex = |c: [u8; 4]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    let mut defs = String::new();
    let mut body = String::new();
    for (k, (poly, fill)) in LAYERS.iter().enumerate() {
        let pts: Vec<String> = poly.iter().map(|(x, y)| format!("{x},{y}")).collect();
        let paint = match fill {
            Fill::Solid(c) => hex(*c),
            Fill::Grad(a, b) => {
                defs.push_str(&format!(
                    "<linearGradient id=\"g{k}\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"100\" gradientUnits=\"userSpaceOnUse\"><stop offset=\"0\" stop-color=\"{}\"/><stop offset=\"1\" stop-color=\"{}\"/></linearGradient>",
                    hex(*a),
                    hex(*b)
                ));
                format!("url(#g{k})")
            }
        };
        body.push_str(&format!("<polygon points=\"{}\" fill=\"{paint}\"/>", pts.join(" ")));
    }
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\" width=\"{px}\" height=\"{px}\"><defs>{defs}</defs>{body}</svg>")
}
