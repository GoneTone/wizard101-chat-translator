//! 量化表：依位元數分列的量化 chip（色點表示相容度），最後一列是 MTP；供下載對話框的推薦與自訂分頁共用。

use std::collections::BTreeMap;

use eframe::egui::{
    Align2, Color32, FontId, Response, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2,
};

use super::theme;
use super::widgets::{focus_ring, hover_tip};
use crate::i18n::t;
use crate::local::RepoInfo;
use crate::local::fit::{self, Fit, total_size};
use crate::local::hardware::{Hardware, effective};

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

/// 畫一個實心色點，高度與一行文字對齊。
pub(super) fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(DOT, DOT), Sense::hover());
    ui.painter().circle_filled(rect.center(), DOT / 2.0, color);
}

/// 選了 `(choice, mtp)` 時所需的位元組與相容等級；VRAM 還沒量到時等級為 `None`。
pub(super) fn combined(
    info: &RepoInfo,
    hw: &Hardware,
    (choice, mtp): (usize, Option<usize>),
) -> (u64, Option<Fit>) {
    let (base, level) = info.fits.get(choice).copied().unwrap_or((0, None));
    let Some(m) = mtp else { return (base, level) };
    let required = base + info.mtp_required.get(m).copied().unwrap_or(0);
    let moe = info.meta.expert_count > 0;
    (required, level.map(|_| fit::fit(required, moe, hw)))
}

/// 畫量化表；點 chip 會改 `pick`（主模型索引, MTP 索引）。`hw` 用來算滑過 chip 時的可用量。
pub(super) fn show(ui: &mut Ui, info: &RepoInfo, hw: &Hardware, pick: &mut (usize, Option<usize>)) {
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
                let (required, level) = combined(info, hw, (i, None));
                let c = &info.catalog.choices[i];
                let size = Some(gb(total_size(c)));
                let r = chip(ui, pick.0 == i, Some(level), &c.quant.label, size);
                tip(ui, &r, level, required, hw);
                if r.clicked() {
                    *pick = (i, pick.1);
                }
            }
        });
    }
    row(ui, "MTP", |ui| {
        let none = t("local.quant.mtp_none", &[]);
        if chip(ui, pick.1.is_none(), None, &none, None).clicked() {
            pick.1 = None;
        }
        for (i, m) in info.catalog.mtp.iter().enumerate() {
            let (required, level) = combined(info, hw, (pick.0, Some(i)));
            let label = format!("MTP {}", m.quant.label);
            let r = chip(
                ui,
                pick.1 == Some(i),
                Some(level),
                &label,
                Some(gb(m.file.size)),
            );
            tip(ui, &r, level, required, hw);
            if r.clicked() {
                pick.1 = Some(i);
            }
        }
    });
}

/// 「需要約 X GB，可用 Y GB」；VRAM 還沒量到時不顯示。
fn tip(ui: &Ui, response: &Response, level: Option<Fit>, required: u64, hw: &Hardware) {
    let Some(level) = level else { return };
    if !response.hovered() {
        return;
    }
    let (vram, ram) = effective(hw);
    // 與 `fit::fit` 比較的對象一致：Smooth 只看 VRAM，其餘看 VRAM＋RAM
    let available = if level == Fit::Smooth {
        vram
    } else {
        vram + ram
    };
    let text = t(
        "local.quant.hover",
        &[("need", &gb(required)), ("have", &gb(available))],
    );
    hover_tip(ui, response, &text);
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

/// 一個可選的量化 chip：色點、名稱、檔案大小；選中的以強調色框起。
fn chip(
    ui: &mut Ui,
    selected: bool,
    level: Option<Option<Fit>>,
    label: &str,
    size: Option<String>,
) -> Response {
    let color = level.map(fit_color);
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
    if let Some(level) = level {
        let text = fit_text(level);
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
