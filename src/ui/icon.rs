//! 程式圖示給各 viewport 用的 egui 版本，以及把內嵌 PNG 載成貼圖。

use std::sync::{Arc, OnceLock};

use eframe::egui::{ColorImage, Context, IconData, Id, TextureHandle, TextureId, TextureOptions};

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

/// 程式圖示（`size` px）的貼圖：第一次用到時載入並快取在 `ctx` 裡，各處共用同一張。
pub fn app_icon_texture(ctx: &Context, size: u32) -> TextureId {
    let id = Id::unique(("app_icon_texture", size));
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture.id();
    }
    let texture = png_texture(ctx, format!("app_icon_{size}"), icon_png(size))
        .expect("embedded icon is a valid PNG");
    let texture_id = texture.id();
    ctx.data_mut(|d| d.insert_temp(id, texture));
    texture_id
}

/// 把 PNG 解碼成名為 `name` 的貼圖（線性取樣）。
pub fn png_texture(
    ctx: &Context,
    name: String,
    png: &[u8],
) -> Result<TextureHandle, image::ImageError> {
    png_texture_with(ctx, name, png, |_| {})
}

/// 同 `png_texture`，但上傳前先以 `edit` 改 RGBA 像素（未預乘）。
pub fn png_texture_with(
    ctx: &Context,
    name: String,
    png: &[u8],
    edit: impl FnOnce(&mut [u8]),
) -> Result<TextureHandle, image::ImageError> {
    let mut data = eframe::icon_data::from_png_bytes(png)?;
    edit(&mut data.rgba);
    let size = [data.width as usize, data.height as usize];
    let image = ColorImage::from_rgba_unmultiplied(size, &data.rgba);
    Ok(ctx.load_texture(name, image, TextureOptions::LINEAR))
}
