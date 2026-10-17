//! 服務商卡片（設定視窗的選擇對話框與首次設定精靈共用）與服務商圖示的載入。

use std::sync::Arc;

use eframe::egui::{
    Align2, Color32, ColorImage, Context, FontId, Galley, Id, Rect, Response, Role, Sense, Stroke,
    StrokeKind, TextureHandle, TextureId, TextureOptions, Ui, Vec2, WidgetInfo, pos2, vec2,
};

use super::theme::{self, icon};
use super::widgets::focus_ring;
use crate::config::services::{Provider, providers};
use crate::i18n::t;
use crate::logs::log;
use crate::runtime::resources::provider_icon;

/// 服務商卡片的排法：選擇對話框一列一張，精靈兩欄。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardLayout {
    List,
    Grid,
}

const ICON_SIZE: f32 = 32.0;
const GRID_GAP: f32 = 12.0;
const GRID_MIN_HEIGHT: f32 = 150.0;

/// 每家服務商一張卡片；回傳這一幀被點下（或以 Enter／Space 觸發）的服務商鍵名。
pub fn provider_cards(ui: &mut Ui, layout: CardLayout) -> Option<&'static str> {
    let mut picked = None;
    ui.scope(|ui| match layout {
        CardLayout::List => {
            ui.spacing_mut().item_spacing.y = 8.0;
            let width = ui.available_width();
            for provider in providers() {
                let content = CardText::new(ui, provider, layout, width);
                let height = content.height(layout);
                if provider_card(ui, provider, &content, vec2(width, height), layout).clicked() {
                    picked = Some(provider.key);
                }
            }
        }
        CardLayout::Grid => {
            ui.spacing_mut().item_spacing = Vec2::splat(GRID_GAP);
            let width = ((ui.available_width() - GRID_GAP) / 2.0).max(0.0);
            for row in providers().chunks(2) {
                let contents: Vec<CardText> = row
                    .iter()
                    .map(|p| CardText::new(ui, p, layout, width))
                    .collect();
                let height = contents
                    .iter()
                    .map(|c| c.height(layout))
                    .fold(GRID_MIN_HEIGHT, f32::max);
                ui.horizontal(|ui| {
                    for (provider, content) in row.iter().zip(&contents) {
                        let size = vec2(width, height);
                        if provider_card(ui, provider, content, size, layout).clicked() {
                            picked = Some(provider.key);
                        }
                    }
                });
            }
        }
    });
    if let Some(key) = picked {
        log(format!("[settings] provider picked: {key}"));
    }
    picked
}

/// 卡片上排好的標題與說明。
struct CardText {
    title: Arc<Galley>,
    desc: Arc<Galley>,
}

impl CardText {
    fn new(ui: &Ui, provider: &Provider, layout: CardLayout, width: f32) -> Self {
        let text_width = match layout {
            // 左右內距、圖示、兩道間距與右側箭頭
            CardLayout::List => width - 28.0 - ICON_SIZE - 2.0 * 14.0 - 12.0,
            CardLayout::Grid => width - 32.0,
        };
        let title_size = match layout {
            CardLayout::List => 14.0,
            CardLayout::Grid => 14.5,
        };
        let painter = ui.painter();
        Self {
            title: painter.layout(
                t(provider.label_key, &[]),
                theme::bold(title_size),
                theme::TEXT,
                text_width.max(1.0),
            ),
            desc: painter.layout(
                t(provider.desc_key, &[]),
                FontId::proportional(theme::HINT_SIZE),
                theme::TEXT_SECONDARY,
                text_width.max(1.0),
            ),
        }
    }

    fn height(&self, layout: CardLayout) -> f32 {
        let (title, desc) = (self.title.size().y, self.desc.size().y);
        match layout {
            CardLayout::List => (title + 2.0 + desc).max(ICON_SIZE) + 24.0,
            CardLayout::Grid => ICON_SIZE + 8.0 + title + 8.0 + desc + 32.0,
        }
    }
}

fn provider_card(
    ui: &mut Ui,
    provider: &Provider,
    content: &CardText,
    size: Vec2,
    layout: CardLayout,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let title = t(provider.label_key, &[]);
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &title));
    let radius = match layout {
        CardLayout::List => theme::CARD_RADIUS,
        CardLayout::Grid => theme::DIALOG_RADIUS,
    };
    if ui.is_rect_visible(rect) {
        let hot = response.hovered();
        let (fill, border) = if hot {
            (theme::HOT_BG, theme::ACCENT)
        } else {
            (theme::CARD_BG, theme::CONTROL_BORDER)
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        match layout {
            CardLayout::List => {
                let icon_rect = Rect::from_center_size(
                    pos2(rect.left() + 14.0 + ICON_SIZE / 2.0, rect.center().y),
                    Vec2::splat(ICON_SIZE),
                );
                paint_provider_icon(ui, provider.key, icon_rect);
                let text_height = content.title.size().y + 2.0 + content.desc.size().y;
                let left = icon_rect.right() + 14.0;
                let top = rect.center().y - text_height / 2.0;
                let desc_top = top + content.title.size().y + 2.0;
                painter.galley(pos2(left, top), content.title.clone(), theme::TEXT);
                painter.galley(pos2(left, desc_top), content.desc.clone(), theme::TEXT);
                painter.text(
                    pos2(rect.right() - 14.0 - 6.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    icon::NEXT,
                    theme::icons(12.0),
                    theme::TEXT_SECONDARY,
                );
            }
            CardLayout::Grid => {
                let icon_rect =
                    Rect::from_min_size(rect.min + Vec2::splat(16.0), Vec2::splat(ICON_SIZE));
                paint_provider_icon(ui, provider.key, icon_rect);
                let title_top = icon_rect.bottom() + 8.0;
                let desc_top = title_top + content.title.size().y + 8.0;
                let left = rect.left() + 16.0;
                painter.galley(pos2(left, title_top), content.title.clone(), theme::TEXT);
                painter.galley(pos2(left, desc_top), content.desc.clone(), theme::TEXT);
            }
        }
    }
    focus_ring(ui, &response, radius);
    response
}

/// 服務商圖示的貼圖：第一次用到時載入並快取在 `ctx` 裡；沒有圖檔或解不開時回 `None`。
pub(super) fn provider_texture(ctx: &Context, key: &str) -> Option<TextureId> {
    let id = Id::unique(("provider_icon", key));
    if let Some(cached) = ctx.data(|d| d.get_temp::<Option<TextureHandle>>(id)) {
        return cached.map(|texture| texture.id());
    }
    let texture = provider_icon(key).and_then(|png| match eframe::icon_data::from_png_bytes(png) {
        Ok(data) => {
            let size = [data.width as usize, data.height as usize];
            let image = ColorImage::from_rgba_unmultiplied(size, &data.rgba);
            let name = format!("provider_icon_{key}");
            Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
        }
        Err(e) => {
            log(format!("[ui] provider icon {key} failed to decode: {e}"));
            None
        }
    });
    let found = texture.as_ref().map(TextureHandle::id);
    ctx.data_mut(|d| d.insert_temp(id, texture));
    found
}

/// 在 `rect` 畫服務商圖示；沒有圖示就不畫。
pub(super) fn paint_provider_icon(ui: &Ui, key: &str, rect: Rect) {
    if let Some(texture) = provider_texture(ui.ctx(), key) {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(texture, rect, uv, Color32::WHITE);
    }
}
