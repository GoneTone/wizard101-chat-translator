//! 程式圖示給各 viewport 用的 egui 版本。

use std::sync::{Arc, OnceLock};

use eframe::egui::IconData;

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
