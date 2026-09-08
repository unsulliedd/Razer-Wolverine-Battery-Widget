
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const SS: u32 = 4; // supersample factor

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IconSpec {
    pub size: u32,
    pub fill: Option<u8>,
    pub charging: bool,
    pub light_taskbar: bool,
}

#[derive(Clone, Copy)]
struct Rgba(u8, u8, u8, u8);

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<Rgba>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Canvas {
            w,
            h,
            px: vec![Rgba(0, 0, 0, 0); (w * h) as usize],
        }
    }
    fn put(&mut self, x: u32, y: u32, c: Rgba) {
        if x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = c;
        }
    }
}

/// Filled rounded rectangle. Coordinates are in supersampled pixels.
fn round_rect(c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, col: Rgba) {
    let r = r.max(0.0).min(((x1 - x0) / 2.0).min((y1 - y0) / 2.0));
    for y in y0.floor().max(0.0) as u32..=(y1.ceil() as u32) {
        for x in x0.floor().max(0.0) as u32..=(x1.ceil() as u32) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if fx < x0 || fx > x1 || fy < y0 || fy > y1 {
                continue;
            }
            // Corner test: clamp to the inner rect and measure.
            let cx = fx.clamp(x0 + r, x1 - r);
            let cy = fy.clamp(y0 + r, y1 - r);
            let (dx, dy) = (fx - cx, fy - cy);
            if dx * dx + dy * dy <= r * r + 0.001 {
                c.put(x, y, col);
            }
        }
    }
}

/// Scan-converted polygon, even-odd rule. Used only for the charging bolt.
fn polygon(c: &mut Canvas, pts: &[(f32, f32)], col: Rgba) {
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    for (x, y) in pts {
        min_y = min_y.min(*y);
        max_y = max_y.max(*y);
        min_x = min_x.min(*x);
        max_x = max_x.max(*x);
    }
    for y in min_y.max(0.0) as u32..=(max_y.ceil() as u32) {
        for x in min_x.max(0.0) as u32..=(max_x.ceil() as u32) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut inside = false;
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let (xi, yi) = pts[i];
                let (xj, yj) = pts[j];
                if (yi > fy) != (yj > fy)
                    && fx < (xj - xi) * (fy - yi) / (yj - yi) + xi
                {
                    inside = !inside;
                }
                j = i;
            }
            if inside {
                c.put(x, y, col);
            }
        }
    }
}

fn bolt_points(x0: f32, y0: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    const P: [(f32, f32); 7] = [
        (0.58, 0.00),
        (0.16, 0.56),
        (0.44, 0.56),
        (0.36, 1.00),
        (0.84, 0.40),
        (0.54, 0.40),
        (0.64, 0.00),
    ];
    P.iter().map(|(x, y)| (x0 + x * w, y0 + y * h)).collect()
}

pub fn render(spec: IconSpec) -> Option<HICON> {
    let (bits, size) = rasterize(spec);
    to_icon(&bits, size)
}

fn rasterize(spec: IconSpec) -> (Vec<u8>, u32) {
    let s = spec.size.max(8);
    let n = s * SS;
    let mut c = Canvas::new(n, n);

    let f = n as f32;
    let clear = Rgba(0, 0, 0, 0);
    let ink = if spec.light_taskbar {
        Rgba(0x1A, 0x1A, 0x1A, 0xFF)
    } else {
        Rgba(0xFF, 0xFF, 0xFF, 0xFF)
    };
    let low = Rgba(0xE8, 0x11, 0x23, 0xFF);
    let level = spec.fill.unwrap_or(0);
    let body_col = if spec.fill.is_some() && level <= 10 && !spec.charging {
        low
    } else {
        ink
    };

    let stroke = (f * 0.055).max(1.0);
    let bx0 = f * 0.055;
    let bx1 = f * 0.80;
    let by0 = f * 0.29;
    let by1 = f * 0.71;
    let radius = f * 0.07;

    // Body outline: fill, then punch the inside back out.
    round_rect(&mut c, bx0, by0, bx1, by1, radius, body_col);
    round_rect(
        &mut c,
        bx0 + stroke,
        by0 + stroke,
        bx1 - stroke,
        by1 - stroke,
        (radius - stroke * 0.5).max(0.0),
        clear,
    );

    round_rect(
        &mut c,
        bx1 + f * 0.02,
        f * 0.42,
        f * 0.925,
        f * 0.58,
        f * 0.03,
        body_col,
    );

    let gap = stroke * 0.7;
    let ix0 = bx0 + stroke + gap;
    let ix1 = bx1 - stroke - gap;
    let iy0 = by0 + stroke + gap;
    let iy1 = by1 - stroke - gap;
    if let Some(pct) = spec.fill {
        let frac = (pct as f32 / 100.0).clamp(0.0, 1.0);
        if frac > 0.005 {
            let w = (ix1 - ix0) * frac;
            round_rect(&mut c, ix0, iy0, ix0 + w, iy1, f * 0.02, body_col);
        }
    } else {
        // No reading: a centred dash, distinct from empty.
        let cy = (iy0 + iy1) * 0.5;
        round_rect(
            &mut c,
            ix0 + (ix1 - ix0) * 0.28,
            cy - stroke * 0.5,
            ix1 - (ix1 - ix0) * 0.28,
            cy + stroke * 0.5,
            stroke * 0.5,
            body_col,
        );
    }

    if spec.charging {
        let bh = (iy1 - iy0) * 1.2;
        let bw = bh * 0.66;
        let bxc = (ix0 + ix1) * 0.5 - bw * 0.5;
        let byc = (iy0 + iy1) * 0.5 - bh * 0.5;
        let halo = stroke * 0.45;
        polygon(
            &mut c,
            &bolt_points(bxc - halo, byc - halo, bw + halo * 2.0, bh + halo * 2.0),
            clear,
        );
        polygon(&mut c, &bolt_points(bxc, byc, bw, bh), ink);
    }

    (downsample(&c, s), s)
}

/// Box-filter the oversampled canvas down to the final size.
fn downsample(c: &Canvas, size: u32) -> Vec<u8> {
    let mut bits: Vec<u8> = Vec::with_capacity((size * size * 4) as usize);
    let f = SS * SS;
    // DIBs are bottom-up.
    for y in (0..size).rev() {
        for x in 0..size {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let p = c.px[((y * SS + sy) * c.w + (x * SS + sx)) as usize];
                    r += p.0 as u32;
                    g += p.1 as u32;
                    b += p.2 as u32;
                    a += p.3 as u32;
                }
            }
            let (r, g, b, a) = ((r / f) as u8, (g / f) as u8, (b / f) as u8, (a / f) as u8);
            // Premultiplied: DrawIconEx alpha-blends 32bpp icons.
            let m = |v: u8| ((v as u32 * a as u32) / 255) as u8;
            bits.push(m(b));
            bits.push(m(g));
            bits.push(m(r));
            bits.push(a);
        }
    }
    bits
}

fn to_icon(bits: &[u8], size: u32) -> Option<HICON> {
    unsafe {
        let hdr = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: size as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let info = BITMAPINFO {
            bmiHeader: hdr,
            ..Default::default()
        };
        let hdc = GetDC(HWND::default());
        let mut dib_bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(hdc, &info, DIB_RGB_COLORS, &mut dib_bits, None, 0).ok();
        ReleaseDC(HWND::default(), hdc);
        let color: HBITMAP = color?;
        if dib_bits.is_null() {
            let _ = DeleteObject(color);
            return None;
        }
        std::ptr::copy_nonoverlapping(bits.as_ptr(), dib_bits as *mut u8, bits.len());

        // A 32bpp icon still needs a mask bitmap, though the alpha channel is
        // what gets used.
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);

        let ii = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&ii).ok();
        let _ = DeleteObject(color);
        let _ = DeleteObject(mask);
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_icon_sheet() {
        let size = 32u32;
        let states: Vec<IconSpec> = [
            (Some(100u8), false),
            (Some(72), false),
            (Some(72), true),
            (Some(35), false),
            (Some(8), false),
            (Some(8), true),
            (None, false),
        ]
        .iter()
        .map(|(fill, charging)| IconSpec {
            size,
            fill: *fill,
            charging: *charging,
            light_taskbar: false,
        })
        .collect();

        let cell = size + 8;
        let sheet_w = cell * states.len() as u32;
        let sheet_h = cell;
        // Bottom-up BGRA, mid-grey so both light and dark ink show.
        let mut sheet = vec![0u8; (sheet_w * sheet_h * 4) as usize];
        for i in (0..sheet.len()).step_by(4) {
            sheet[i] = 0x28;
            sheet[i + 1] = 0x28;
            sheet[i + 2] = 0x28;
            sheet[i + 3] = 0xFF;
        }
        for (i, spec) in states.iter().enumerate() {
            let (bits, s) = rasterize(*spec);
            for y in 0..s {
                for x in 0..s {
                    let src = ((y * s + x) * 4) as usize;
                    let a = bits[src + 3] as u32;
                    let dx = i as u32 * cell + 4 + x;
                    let dy = 4 + y;
                    let dst = ((dy * sheet_w + dx) * 4) as usize;
                    for ch in 0..3 {
                        let s0 = bits[src + ch] as u32; // premultiplied
                        let d0 = sheet[dst + ch] as u32;
                        sheet[dst + ch] = (s0 + d0 * (255 - a) / 255).min(255) as u8;
                    }
                }
            }
        }

        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/icons.bmp");
        write_bmp(&out, &sheet, sheet_w, sheet_h);
        assert!(out.exists());
    }

    #[test]
    fn dump_app_icon() {
        const ACCENT: (u8, u8, u8) = (0x40, 0xD0, 0x6B); // matches Palette::good
        let sizes = [16u32, 32, 48, 64];

        let images: Vec<(u32, Vec<u8>)> = sizes
            .iter()
            .map(|&size| {
                let (bits, s) = rasterize(IconSpec {
                    size,
                    fill: Some(100),
                    charging: false,
                    light_taskbar: false,
                });
                assert_eq!(s, size);
                let mut out = bits.clone();
                for px in out.chunks_mut(4) {
                    // An ICO carries straight alpha, so keep the alpha and
                    // write the colour unscaled.
                    px[0] = ACCENT.2;
                    px[1] = ACCENT.1;
                    px[2] = ACCENT.0;
                }
                (size, out)
            })
            .collect();

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.ico");
        write_ico(&path, &images);
        assert!(path.exists());
    }

    /// A directory followed by 32-bit DIBs, each a BITMAPINFOHEADER whose
    /// height is doubled to cover the legacy AND mask. The mask is all zeroes
    /// but must be present and correctly padded or the icon fails to load.
    fn write_ico(path: &std::path::Path, images: &[(u32, Vec<u8>)]) {
        let mut dir = Vec::new();
        let mut blobs = Vec::new();

        dir.extend_from_slice(&0u16.to_le_bytes()); // reserved
        dir.extend_from_slice(&1u16.to_le_bytes()); // type: icon
        dir.extend_from_slice(&(images.len() as u16).to_le_bytes());

        let mut offset = 6 + 16 * images.len() as u32;
        for (size, bgra) in images {
            let mask_row = size.div_ceil(32) * 4;
            let mask_len = mask_row * size;
            let blob_len = 40 + bgra.len() as u32 + mask_len;

            dir.push(if *size >= 256 { 0 } else { *size as u8 });
            dir.push(if *size >= 256 { 0 } else { *size as u8 });
            dir.push(0); // palette entries
            dir.push(0); // reserved
            dir.extend_from_slice(&1u16.to_le_bytes()); // planes
            dir.extend_from_slice(&32u16.to_le_bytes()); // bpp
            dir.extend_from_slice(&blob_len.to_le_bytes());
            dir.extend_from_slice(&offset.to_le_bytes());
            offset += blob_len;

            let mut b = Vec::with_capacity(blob_len as usize);
            b.extend_from_slice(&40u32.to_le_bytes());
            b.extend_from_slice(&(*size as i32).to_le_bytes());
            b.extend_from_slice(&(*size as i32 * 2).to_le_bytes()); // colour + mask
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&32u16.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
            b.extend_from_slice(&(bgra.len() as u32 + mask_len).to_le_bytes());
            for _ in 0..4 {
                b.extend_from_slice(&0u32.to_le_bytes());
            }
            b.extend_from_slice(bgra);
            b.extend(std::iter::repeat_n(0u8, mask_len as usize));
            blobs.push(b);
        }

        let mut f = dir;
        for b in blobs {
            f.extend_from_slice(&b);
        }
        std::fs::write(path, f).unwrap();
    }

    fn write_bmp(path: &std::path::Path, bgra: &[u8], w: u32, h: u32) {
        let mut f = Vec::new();
        let pixel_bytes = w * h * 4;
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&(54u32 + pixel_bytes).to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&(w as i32).to_le_bytes());
        f.extend_from_slice(&(h as i32).to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&32u16.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&pixel_bytes.to_le_bytes());
        for _ in 0..4 {
            f.extend_from_slice(&0u32.to_le_bytes());
        }
        f.extend_from_slice(bgra);
        std::fs::write(path, f).unwrap();
    }
}
