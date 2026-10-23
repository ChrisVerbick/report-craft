//! An edition's app icon from its mark (SVG), on Apple's icon grid: a 1024-px tile holds an
//! 824-px plate, centred, with corners of 0.2237 of the plate; the rest is transparent.

use std::path::Path;

/// The mark on Apple's grid at `side` × `side` pixels (at most 4096), as straight (not
/// premultiplied) RGBA: the mark fills the plate, masked to its rounded square. None when the SVG
/// doesn't render.
pub fn icon_grid(svg: &[u8], side: u32) -> Option<Vec<u8>> {
    if side > 4096 {
        return None;
    }
    let plate = (side as f32 * 824.0 / 1024.0).round() as u32;
    let origin = (side - plate) / 2;
    let (mark, w, h) = designcraft_images::render_svg(svg, plate.max(1))?;
    let bytes = |w: u32, h: u32| (w as usize).checked_mul(h as usize)?.checked_mul(4);
    if bytes(w, h)? != mark.len() || w > plate || h > plate {
        return None;
    }
    let radius = 0.2237 * plate as f32;
    let mut out = vec![0; bytes(side, side)?];
    // A mark that isn't square is centred on the plate.
    let (left, top) = (origin + (plate - w) / 2, origin + (plate - h) / 2);
    for y in 0..h {
        for x in 0..w {
            let (tx, ty) = (left + x, top + y);
            let cover = plate_coverage(tx as f32 + 0.5, ty as f32 + 0.5, origin as f32, plate as f32, radius);
            let i = (y as usize * w as usize + x as usize) * 4;
            let j = (ty as usize * side as usize + tx as usize) * 4;
            let (Some(&[r, g, b, a]), Some(px)) = (mark.get(i..i + 4).and_then(|p| <&[u8; 4]>::try_from(p).ok()), out.get_mut(j..j + 4)) else {
                continue;
            };
            if a == 0 || cover <= 0.0 {
                continue;
            }
            // Premultiplied to straight; the plate's edge becomes coverage.
            let straight = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            px.copy_from_slice(&[straight(r), straight(g), straight(b), (f32::from(a) * cover).round() as u8]);
        }
    }
    Some(out)
}

/// How much of the pixel centred at (x, y) the plate covers: the square at `origin` with sides
/// `size` and corners of `radius`, anti-aliased along the corners' arcs.
fn plate_coverage(x: f32, y: f32, origin: f32, size: f32, radius: f32) -> f32 {
    let (lo, hi, r) = (origin, origin + size, radius.min(size / 2.0));
    if x < lo || y < lo || x > hi || y > hi {
        return 0.0;
    }
    // The nearest point of the square inset by the radius: how far past it a pixel is, is how far
    // into a corner it is.
    let (cx, cy) = (x.clamp(lo + r, hi - r), y.clamp(lo + r, hi - r));
    (r - (x - cx).hypot(y - cy) + 0.5).clamp(0.0, 1.0)
}

/// The `.iconset` folder `iconutil` turns into an `.icns`: the mark on Apple's grid at every size
/// macOS asks for, 16 to 1024 px, as `icon_<n>x<n>.png` and `icon_<n>x<n>@2x.png`.
pub fn write_iconset(svg: &[u8], dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for n in [16, 32, 128, 256, 512] {
        for (scale, suffix) in [(1, ""), (2, "@2x")] {
            let side = n * scale;
            let rgba = icon_grid(svg, side).ok_or("the edition's mark doesn't render")?;
            let image = image::RgbaImage::from_raw(side, side, rgba).ok_or("the icon's pixels don't fill it")?;
            let path = dir.join(format!("icon_{n}x{n}{suffix}.png"));
            image.save_with_format(&path, image::ImageFormat::Png).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Task 1's fixture edition (`crates/edition/tests/fixture/`): its identity and mark.
#[cfg(test)]
pub(crate) const TEST_EDITION: designcraft_edition::Edition = designcraft_edition::Edition {
    name: "Test Edition",
    bundle_id: "org.example.test-edition",
    data_folder: "Test Edition",
    wordmark: "TEST",
    mark_svg: include_bytes!("../../../crates/edition/tests/fixture/mark.svg"),
    theme: designcraft_edition::Theme {
        bg: [0x10, 0x14, 0x18],
        raised: [0x20, 0x26, 0x2c],
        hover: [0x2a, 0x31, 0x38],
        text: [0xe8, 0xec, 0xf0],
        accent: [0x3a, 0xa6, 0x75],
        on_accent: [0x00, 0x00, 0x00],
        success: [0x4c, 0xc3, 0x8a],
        error: [0xe5, 0x48, 0x4d],
        secondary: 0.72,
        muted: 0.52,
        faint: 0.32,
        hairline: 0.18,
        radius: 0,
        density: designcraft_edition::Density::Compact,
    },
    fonts: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(rgba: &[u8], side: u32, x: u32, y: u32) -> u8 {
        rgba[((y * side + x) * 4 + 3) as usize]
    }

    /// The mark fills an 824-px plate centred in the 1024-px tile, with Apple's rounded corners;
    /// the margin and the corners are transparent.
    #[test]
    fn icon_grid_masks_to_the_plate() {
        let rgba = icon_grid(TEST_EDITION.mark_svg, 1024).unwrap();
        assert_eq!(rgba.len(), 1024 * 1024 * 4);
        assert_eq!(alpha(&rgba, 1024, 512, 512), 255);
        assert_eq!(alpha(&rgba, 1024, 512, 102), 255, "the plate's top edge, between its corners");
        for (x, y) in [(0, 0), (50, 512), (110, 110), (122, 122), (1013, 1013)] {
            assert_eq!(alpha(&rgba, 1024, x, y), 0, "({x}, {y})");
        }
        // Straight alpha: the fixture's light circle keeps its colour.
        assert_eq!(&rgba[(512 * 1024 + 512) * 4..][..3], &[0xe8, 0xec, 0xf0]);
    }

    /// A mark that won't render gives no icon, and the app keeps DesignCraft's.
    #[test]
    fn icon_grid_rejects_garbage() {
        assert_eq!(icon_grid(b"<nope", 64), None);
        let fallback = crate::app_icon_from(Some(b"<nope")).unwrap();
        let plain = crate::app_icon_from(None).unwrap();
        assert!(fallback.width == plain.width && fallback.height == plain.height && fallback.rgba == plain.rgba);
        // The fixture's mark becomes a 512-px icon on Apple's grid.
        let icon = crate::app_icon_from(Some(TEST_EDITION.mark_svg)).unwrap();
        assert_eq!((icon.width, icon.height), (512, 512));
        assert_eq!(icon.rgba[3], 0, "the corner is transparent");
    }

    /// The iconset holds every size `iconutil` asks for, as PNGs of those sizes.
    #[test]
    fn write_iconset_writes_every_size() {
        let dir = std::env::temp_dir().join(format!("dc-edition-iconset-{}", std::process::id()));
        write_iconset(TEST_EDITION.mark_svg, &dir).unwrap();
        for n in [16u32, 32, 128, 256, 512] {
            for (scale, suffix) in [(1, ""), (2, "@2x")] {
                let png = std::fs::read(dir.join(format!("icon_{n}x{n}{suffix}.png"))).unwrap();
                let image = image::load_from_memory(&png).unwrap();
                assert_eq!((image.width(), image.height()), (n * scale, n * scale), "icon_{n}x{n}{suffix}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
