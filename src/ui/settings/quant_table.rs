//! 量化表：依位元數分列的量化 chip（色點表示相容度），最後一列是 MTP；供下載對話框的推薦與自訂分頁共用。

use std::collections::BTreeMap;

use eframe::egui::{
    Align2, Color32, FontId, Response, RichText, Role, Sense, Stroke, StrokeKind, TextStyle, Ui,
    WidgetInfo, pos2, vec2,
};

use super::theme;
use super::widgets::{focus_ring, skeleton};
use crate::i18n::t;
use crate::local::RepoInfo;
use crate::local::fit::{Fit, Need, Rating, total_size};
use crate::local::hf::MtpChoice;

const CHIP_HEIGHT: f32 = 28.0;
const DOT: f32 = 8.0;
const BIT_LABEL_WIDTH: f32 = 56.0;

/// 位元組換成一位小數的 GB 字串。
pub(super) fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1u64 << 30) as f64)
}

/// 進度條的比例；`total` 為 0（還不知道大小）時為 0。
pub(super) fn fraction(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

/// 相容等級對應的色點顏色；`None`（VRAM 還沒量到）為灰。
pub(super) fn fit_color(fit: Option<Fit>) -> Color32 {
    match fit {
        Some(Fit::Smooth) => theme::OK,
        Some(Fit::Slow | Fit::Offload) => theme::WARN_TEXT,
        Some(Fit::NoFit) => theme::ERROR,
        None => theme::TEXT_MUTED,
    }
}

/// 相容等級的文字（i18n 後）。
pub(super) fn fit_text(fit: Option<Fit>) -> String {
    let key = match fit {
        Some(Fit::Smooth) => "local.fit.smooth",
        Some(Fit::Slow) => "local.fit.slow",
        Some(Fit::Offload) => "local.fit.offload",
        Some(Fit::NoFit) => "local.fit.nofit",
        None => "local.fit.unknown",
    };
    t(key, &[])
}

/// 相容等級的文字；依預估判斷的（`estimated`）加註「預估」。
pub(super) fn fit_label(level: Option<Fit>, estimated: bool) -> String {
    let text = fit_text(level);
    if estimated && level.is_some() {
        t("local.fit.estimated", &[("fit", &text)])
    } else {
        text
    }
}

/// 相容等級徽章：色點加文字；硬體偵測中（`detecting`）改畫佔位。
pub(super) fn fit_badge(ui: &mut Ui, level: Option<Fit>, estimated: bool, detecting: bool) {
    if detecting {
        let height = ui.text_style_height(&TextStyle::Body);
        skeleton(ui, 64.0, height);
        return;
    }
    dot(ui, fit_color(level));
    ui.label(RichText::new(fit_label(level, estimated)).color(fit_color(level)));
}

/// 畫一個實心色點，高度與一行文字對齊。
pub(super) fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
    ui.painter().circle_filled(rect.center(), DOT / 2.0, color);
}

/// 選了 `(choice, mtp)` 時的需求與相容等級（hub 算好的）；索引不在表內時等級未知。
pub(super) fn combined(info: &RepoInfo, (choice, mtp): (usize, Option<usize>)) -> Rating {
    let column = mtp.map_or(0, |m| m + 1);
    let rating = info.fits.get(choice).and_then(|f| f.get(column));
    rating.copied().unwrap_or(Rating {
        need: Need::estimate(0),
        fit: None,
    })
}

/// 畫量化表；點 chip 會改 `pick`（主模型索引, MTP 索引）。
pub(super) fn show(ui: &mut Ui, info: &RepoInfo, pick: &mut (usize, Option<usize>)) {
    let mut groups: BTreeMap<(bool, u8), Vec<usize>> = BTreeMap::new();
    for (i, c) in info.catalog.choices.iter().enumerate() {
        let key = c.quant.bits.map_or((true, 0), |b| (false, b));
        groups.entry(key).or_default().push(i);
    }
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    for ((other, bits), indexes) in groups {
        let name = if other {
            t("local.quant.other", &[])
        } else {
            format!("{bits}-bit")
        };
        row(ui, &name, |ui| {
            for i in indexes {
                let rating = combined(info, (i, None));
                let c = &info.catalog.choices[i];
                let size = Some(gb(total_size(c)));
                let r = chip(ui, pick.0 == i, Some(rating), &c.quant.label, size);
                if r.clicked() {
                    *pick = (i, pick.1);
                }
            }
        });
    }
    // 沒有 MTP 檔時只剩「不使用」，整列不顯示
    if info.catalog.mtp.is_empty() {
        return;
    }
    row(ui, "MTP", |ui| {
        let none = t("local.quant.mtp_none", &[]);
        if chip(ui, pick.1.is_none(), None, &none, None).clicked() {
            pick.1 = None;
        }
        for (i, m) in info.catalog.mtp.iter().enumerate() {
            let rating = combined(info, (pick.0, Some(i)));
            let label = mtp_label(&info.catalog.mtp, i);
            let r = chip(
                ui,
                pick.1 == Some(i),
                Some(rating),
                &label,
                Some(gb(m.file.size)),
            );
            if r.clicked() {
                pick.1 = Some(i);
            }
        }
    });
}

/// MTP 選項的名稱；只有一個且檔名看不出量化時只寫「MTP」，不把整串檔名放進來。
pub(super) fn mtp_label(mtp: &[MtpChoice], i: usize) -> String {
    let m = &mtp[i];
    if m.quant.bits.is_none() && mtp.len() == 1 {
        "MTP".into()
    } else {
        format!("MTP {}", m.quant.label)
    }
}

fn row(ui: &mut Ui, name: &str, chips: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.add_sized(
            vec2(BIT_LABEL_WIDTH, CHIP_HEIGHT),
            eframe::egui::Label::new(
                eframe::egui::RichText::new(name).color(theme::TEXT_SECONDARY),
            ),
        );
        ui.horizontal_wrapped(chips);
    });
}

/// 一個可選的量化 chip：色點（有 `rating` 時）、名稱、檔案大小；選中的以強調色框起。
fn chip(
    ui: &mut Ui,
    selected: bool,
    rating: Option<Rating>,
    label: &str,
    size: Option<String>,
) -> Response {
    let color = rating.map(|r| fit_color(r.fit));
    let font = FontId::proportional(theme::CONTROL_TEXT_SIZE);
    let painter = ui.painter();
    let name = painter.layout_no_wrap(label.to_owned(), font.clone(), theme::TEXT);
    let size = size.map(|s| {
        painter.layout_no_wrap(
            s + " GB",
            FontId::proportional(theme::HINT_SIZE),
            theme::TEXT_SECONDARY,
        )
    });
    let dot_width = color.map_or(0.0, |_| DOT + 6.0);
    let size_width = size.as_ref().map_or(0.0, |g| g.size().x + 6.0);
    let width = 20.0 + dot_width + name.size().x + size_width;
    let (rect, response) = ui.allocate_exact_size(vec2(width, CHIP_HEIGHT), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(Role::RadioButton, ui.is_enabled(), selected, label));
    // 相容度不能只靠色點傳達
    if let Some(r) = rating {
        let text = fit_label(r.fit, r.need.estimated());
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_description(text));
    }
    if ui.is_rect_visible(rect) {
        let stroke = if selected {
            Stroke::new(1.5, theme::ACCENT)
        } else {
            Stroke::new(1.0, theme::CONTROL_BORDER)
        };
        let fill = if response.hovered() {
            theme::CONTROL_BORDER
        } else {
            theme::CONTROL_BG
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            theme::CONTROL_RADIUS,
            fill,
            stroke,
            StrokeKind::Inside,
        );
        let mut x = rect.left() + 10.0;
        if let Some(color) = color {
            painter.circle_filled(pos2(x + DOT / 2.0, rect.center().y), DOT / 2.0, color);
            x += dot_width;
        }
        painter.text(
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            font,
            theme::TEXT,
        );
        x += name.size().x + 6.0;
        if let Some(size) = size {
            let pos = pos2(x, rect.center().y - size.size().y / 2.0);
            painter.galley(pos, size, theme::TEXT_SECONDARY);
        }
    }
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}
