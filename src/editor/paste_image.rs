//! Clipboard-image paste: naming and link building. The editor owns
//! the file I/O; everything here is pure.

use gpui::ImageFormat;

/// File extension for a clipboard image format.
pub fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
    }
}

/// `pasted-<stamp>.<ext>`, suffixed `-2`, `-3`, … while `taken` says
/// the name already exists.
pub fn pick_name(taken: impl Fn(&str) -> bool, stamp: &str, ext: &str) -> String {
    let base = format!("pasted-{stamp}.{ext}");
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("pasted-{stamp}-{n}.{ext}"))
        .find(|name| !taken(name))
        .unwrap()
}

/// The markdown image link for an asset file name.
pub fn markdown_link(name: &str) -> String {
    format!("![](assets/{name})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_cover_every_format() {
        assert_eq!(extension(ImageFormat::Png), "png");
        assert_eq!(extension(ImageFormat::Jpeg), "jpg");
        assert_eq!(extension(ImageFormat::Webp), "webp");
        assert_eq!(extension(ImageFormat::Gif), "gif");
        assert_eq!(extension(ImageFormat::Svg), "svg");
        assert_eq!(extension(ImageFormat::Bmp), "bmp");
        assert_eq!(extension(ImageFormat::Tiff), "tiff");
    }

    #[test]
    fn names_stamp_and_dodge_collisions() {
        assert_eq!(pick_name(|_| false, "20260825-101500", "png"), "pasted-20260825-101500.png");
        let taken = |n: &str| n == "pasted-s.png" || n == "pasted-s-2.png";
        assert_eq!(pick_name(taken, "s", "png"), "pasted-s-3.png");
    }

    #[test]
    fn link_is_relative_to_the_assets_folder() {
        assert_eq!(markdown_link("pasted-s.png"), "![](assets/pasted-s.png)");
    }
}
