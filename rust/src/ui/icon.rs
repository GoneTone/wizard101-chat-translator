//! 程式圖示給各 viewport 用的 egui 版本，以及把內嵌 PNG 載成貼圖。

use std::sync::{Arc, OnceLock};

use eframe::egui::{ColorImage, Context, IconData, TextureHandle, TextureOptions};

use crate::runtime::resources::icon_png;

/// 程式圖示（48px）；只解碼一次，每幀交給 viewport 的都是同一份，egui 不會當成變更。
pub fn app_icon() -> Arc<IconData> {
    static ICON: OnceLock<Arc<IconData>> = OnceLock::new();
    let icon = ICON.get_or_init(|| {
        let icon = eframe::icon_data::from_png_bytes(icon_png(48));
        Arc::new(icon.expect("embedded icon is a valid PNG"))
    });
    Arc::clone(icon)
}

/// 把 PNG 解碼成名為 `name` 的貼圖（線性取樣）。
pub fn png_texture(
    ctx: &Context,
    name: String,
    png: &[u8],
) -> Result<TextureHandle, image::ImageError> {
    let data = eframe::icon_data::from_png_bytes(png)?;
    let size = [data.width as usize, data.height as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, &data.rgba);
    Ok(ctx.load_texture(name, image, TextureOptions::LINEAR))
}
