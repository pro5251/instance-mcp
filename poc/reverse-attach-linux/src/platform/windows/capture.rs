//! `screenshot` on Windows: GDI `StretchBlt` of one display (or a region of it) with
//! the cursor drawn in, encoded as PNG or JPEG. Chosen by spike 1 (spec §4.2): no
//! prompt, no capture border, ~30 ms per 1080p frame.

use serde_json::{json, Value};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HALFTONE, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, ICONINFO,
};

use super::desk::{self, Display};

/// A validated request, in display pixels.
#[derive(Debug, PartialEq)]
pub(crate) struct Shot {
    pub(crate) display: i64,
    pub(crate) region: Option<(i32, i32, i32, i32)>,
    pub(crate) scale: f64,
    pub(crate) format: Format,
    /// JPEG quality 1–100.
    pub(crate) quality: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Png,
    Jpeg,
}

/// Parse the arguments with the Linux defaults (scale 0.5, png, quality 80).
/// `quality` takes 1–100 or, as the macOS daemon and Connect send it, 0–1.
pub(crate) fn parse(args: &Value) -> Result<Shot, String> {
    let scale = args.get("scale").and_then(Value::as_f64).unwrap_or(0.5);
    if !(0.05..=2.0).contains(&scale) {
        return Err("scale must be in 0.05..=2.0".to_string());
    }
    let format = match args.get("format").and_then(Value::as_str).unwrap_or("png") {
        "png" => Format::Png,
        "jpeg" | "jpg" => Format::Jpeg,
        other => return Err(format!("format must be jpeg or png, got {other}")),
    };
    let quality = match args.get("quality").and_then(Value::as_f64) {
        None => 80.0,
        Some(q) if q > 0.0 && q <= 1.0 => (q * 100.0).round(),
        Some(q) if (1.0..=100.0).contains(&q) => q.round(),
        Some(q) => return Err(format!("quality must be 0–1 or 1–100, got {q}")),
    };
    let display = args.get("display").and_then(Value::as_i64).unwrap_or(0);
    let region = match args.get("region") {
        None | Some(Value::Null) => None,
        Some(r) => {
            let n = |k: &str| r.get(k).and_then(Value::as_f64);
            match (n("x"), n("y"), n("width"), n("height")) {
                (Some(x), Some(y), Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((
                    x.round() as i32,
                    y.round() as i32,
                    w.round() as i32,
                    h.round() as i32,
                )),
                _ => return Err("region needs x, y, width>0, height>0".to_string()),
            }
        }
    };
    Ok(Shot {
        display,
        region,
        scale,
        format,
        quality: quality as u8,
    })
}

/// `region` clipped to the display, or the whole display.
pub(crate) fn clip(
    region: Option<(i32, i32, i32, i32)>,
    d: &Display,
) -> Result<(i32, i32, i32, i32), String> {
    let Some((x, y, w, h)) = region else {
        return Ok((0, 0, d.width, d.height));
    };
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(d.width), (y + h).min(d.height));
    if x1 <= x0 || y1 <= y0 {
        return Err("region lies outside the display".to_string());
    }
    Ok((x0, y0, x1 - x0, y1 - y0))
}

/// BGRA pixels, top-down.
struct Frame {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

fn grab(d: &Display, (rx, ry, rw, rh): (i32, i32, i32, i32), scale: f64) -> Result<Frame, String> {
    let ow = ((rw as f64 * scale).round() as i32).max(1);
    let oh = ((rh as f64 * scale).round() as i32).max(1);
    // SAFETY: every GDI object created here is selected out and released before return;
    // the DIB buffer is sized for ow*oh 32-bit pixels.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("GetDC failed".to_string());
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, ow, oh);
        let old = SelectObject(mem, bmp);
        SetStretchBltMode(mem, HALFTONE as _);
        SetBrushOrgEx(mem, 0, 0, std::ptr::null_mut());
        let ok = StretchBlt(
            mem,
            0,
            0,
            ow,
            oh,
            screen,
            d.left + rx,
            d.top + ry,
            rw,
            rh,
            SRCCOPY | CAPTUREBLT,
        );
        if ok != 0 {
            draw_cursor(mem, d, (rx, ry), scale);
        }
        SelectObject(mem, old);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ow,
            biHeight: -oh, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bgra = vec![0u8; ow as usize * oh as usize * 4];
        let lines = if ok != 0 {
            GetDIBits(
                mem,
                bmp,
                0,
                oh as u32,
                bgra.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        if ok == 0 || lines == 0 {
            return Err("capture failed (StretchBlt/GetDIBits)".to_string());
        }
        Ok(Frame {
            width: ow,
            height: oh,
            bgra,
        })
    }
}

/// Draw the visible cursor where it is on screen, as the macOS capture shows it.
unsafe fn draw_cursor(
    mem: windows_sys::Win32::Graphics::Gdi::HDC,
    d: &Display,
    (rx, ry): (i32, i32),
    scale: f64,
) {
    let mut ci: CURSORINFO = std::mem::zeroed();
    ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
    if GetCursorInfo(&mut ci) == 0 || ci.flags & CURSOR_SHOWING == 0 {
        return;
    }
    let mut ii: ICONINFO = std::mem::zeroed();
    if GetIconInfo(ci.hCursor, &mut ii) == 0 {
        return;
    }
    let POINT { x, y } = ci.ptScreenPos;
    let cx = ((x - d.left - rx - ii.xHotspot as i32) as f64 * scale).round() as i32;
    let cy = ((y - d.top - ry - ii.yHotspot as i32) as f64 * scale).round() as i32;
    let size = ((32.0 * scale).round() as i32).max(8);
    DrawIconEx(
        mem,
        cx,
        cy,
        ci.hCursor,
        size,
        size,
        0,
        std::ptr::null_mut(),
        DI_NORMAL,
    );
    if !ii.hbmMask.is_null() {
        DeleteObject(ii.hbmMask);
    }
    if !ii.hbmColor.is_null() {
        DeleteObject(ii.hbmColor);
    }
}

fn encode(f: &Frame, format: Format, quality: u8) -> Result<Vec<u8>, String> {
    let mut rgb = Vec::with_capacity(f.width as usize * f.height as usize * 3);
    for px in f.bgra.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let mut out = Vec::new();
    match format {
        Format::Png => {
            let mut enc = png::Encoder::new(&mut out, f.width as u32, f.height as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut w = enc.write_header().map_err(|e| format!("png: {e}"))?;
            w.write_image_data(&rgb).map_err(|e| format!("png: {e}"))?;
        }
        Format::Jpeg => {
            let enc = jpeg_encoder::Encoder::new(&mut out, quality);
            enc.encode(
                &rgb,
                f.width as u16,
                f.height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .map_err(|e| format!("jpeg: {e}"))?;
        }
    }
    Ok(out)
}

pub(crate) fn tool_screenshot(args: &Value) -> Result<Value, (i64, String)> {
    let shot = parse(args).map_err(|e| (-32602, e))?;
    desk::require_usable_desktop().map_err(|e| (-32000, e))?;
    let d = desk::display(shot.display).map_err(|e| (-32602, e))?;
    let region = clip(shot.region, &d).map_err(|e| (-32602, e))?;
    let frame = grab(&d, region, shot.scale).map_err(|e| (-32000, e))?;
    let bytes = encode(&frame, shot.format, shot.quality).map_err(|e| (-32000, e))?;
    let (mime, name) = match shot.format {
        Format::Png => ("image/png", "png"),
        Format::Jpeg => ("image/jpeg", "jpeg"),
    };
    let caption = format!(
        "display {}: {}×{} px{} → {}×{} px {mime} ({} KiB), scale {}",
        shot.display,
        d.width,
        d.height,
        if shot.region.is_some() {
            format!(
                " region {},{} {}×{}",
                region.0, region.1, region.2, region.3
            )
        } else {
            String::new()
        },
        frame.width,
        frame.height,
        bytes.len() / 1024,
        shot.scale,
    );
    let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(json!({
        "content": [
            { "type": "image", "mimeType": mime, "data": data },
            { "type": "text", "text": caption }
        ],
        "structuredContent": {
            "bytes": bytes.len(),
            "scale": shot.scale,
            "format": name,
            "display": shot.display,
            "device": d.device,
            "points": { "width": d.width, "height": d.height },
            "region": { "x": region.0, "y": region.1, "width": region.2, "height": region.3 },
            "image": { "width": frame.width, "height": frame.height, "bytes": bytes.len() }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> Display {
        Display {
            device: "D".into(),
            left: 1920,
            top: -122,
            width: 1920,
            height: 1200,
            dpi: 120,
            primary: false,
        }
    }

    #[test]
    fn defaults_match_linux() {
        let s = parse(&json!({})).unwrap();
        assert_eq!(
            (s.display, s.scale, s.format, s.quality, s.region),
            (0, 0.5, Format::Png, 80, None)
        );
    }

    #[test]
    fn quality_accepts_both_ranges() {
        assert_eq!(parse(&json!({"quality": 0.6})).unwrap().quality, 60);
        assert_eq!(parse(&json!({"quality": 1})).unwrap().quality, 100);
        assert_eq!(parse(&json!({"quality": 75})).unwrap().quality, 75);
        assert!(parse(&json!({"quality": 0})).is_err());
        assert!(parse(&json!({"quality": 101})).is_err());
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse(&json!({"scale": 3})).is_err());
        assert!(parse(&json!({"format": "gif"})).is_err());
        assert!(parse(&json!({"region": {"x": 0, "y": 0, "width": 0, "height": 5}})).is_err());
    }

    #[test]
    fn regions_are_clipped_to_the_display() {
        assert_eq!(clip(None, &d()), Ok((0, 0, 1920, 1200)));
        assert_eq!(
            clip(Some((1900, 1190, 100, 100)), &d()),
            Ok((1900, 1190, 20, 10))
        );
        assert_eq!(clip(Some((-10, -10, 20, 20)), &d()), Ok((0, 0, 10, 10)));
        assert!(clip(Some((2000, 0, 10, 10)), &d()).is_err());
    }
}
