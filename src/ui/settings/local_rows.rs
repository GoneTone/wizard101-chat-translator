//! 下載對話框推薦列的零件：記憶體需求文字、已下載狀態、判斷依據，以及列的文字與按鈕。

use eframe::egui::{Label, RichText, Ui};

use super::local_dialog::DialogMode;
use super::quant_table::{fit_badge, gb};
use super::theme;
use super::widgets::{primary_button, secondary_button};
use crate::i18n::t;
use crate::local::Snapshot;
use crate::local::fit::{Basis, MemoryKind, Need, Rating, memory_kind};
use crate::local::hardware::{Hardware, effective};

/// 記憶體需求文字：實測列出 VRAM 與 RAM，其餘是「預估約需」多少哪種記憶體。
pub(super) fn need_text(need: &Need, moe: bool, hw: &Hardware) -> String {
    let kind = memory_kind(moe, hw);
    if need.basis == Basis::Measured {
        let (vram, ram) = (gb(need.vram), gb(need.ram));
        // 內顯的 GPU 記憶體已算在 RAM 裡
        return if kind == MemoryKind::Ram {
            t("local.memory.measured_ram", &[("ram", &ram)])
        } else {
            t("local.memory.measured", &[("vram", &vram), ("ram", &ram)])
        };
    }
    let (key, size) = match kind {
        MemoryKind::Vram => ("local.memory.vram", need.vram),
        MemoryKind::VramPlusRam => ("local.memory.vram_ram", need.total()),
        MemoryKind::Ram => ("local.memory.ram", need.total()),
    };
    t(key, &[("size", &gb(size))])
}

/// 已下載的狀態：完整、未完成（進度在設定頁），或沒有。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Have {
    No,
    Complete,
    InProgress,
}

/// 模型 `id`（`store::choice_id`）的下載狀態。
pub(super) fn have_of(snapshot: &Snapshot, id: &str) -> Have {
    match snapshot.models.iter().find(|m| m.id == id) {
        Some(m) if m.complete => Have::Complete,
        Some(_) => Have::InProgress,
        None => Have::No,
    }
}

/// 推薦分頁頂端：判斷相容等級所依據的可用記憶體。
pub(super) fn basis(ui: &mut Ui, hw: &Hardware) {
    let (vram, ram) = effective(hw);
    let text = if !hw.measured {
        t("local.dialog.basis_unknown", &[])
    } else if vram == 0 {
        t("local.dialog.basis_ram", &[("ram", &gb(ram))])
    } else {
        t(
            "local.dialog.basis",
            &[("vram", &gb(vram)), ("ram", &gb(ram))],
        )
    };
    ui.label(
        RichText::new(text)
            .size(theme::HINT_SIZE)
            .color(theme::TEXT_SECONDARY),
    );
}

/// 推薦列左側的文字：名稱、記憶體需求、所選量化，以及下載狀態。
pub(super) fn row_text(ui: &mut Ui, name: RichText, memory: &str, version: &str, have: Have) {
    let small = |text: &str, color| RichText::new(text).size(theme::HINT_SIZE).color(color);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(Label::new(name).truncate());
        ui.add(Label::new(small(memory, theme::TEXT_SECONDARY)).wrap());
        ui.label(small(version, theme::TEXT_SECONDARY));
        if have == Have::Complete {
            ui.label(small(
                &format!("✓ {}", t("local.downloaded", &[])),
                theme::OK,
            ));
        }
    });
}

/// 推薦列右側：相容徽章、〔選擇版本〕與動作鈕；回傳（切換量化表, 按了動作鈕）。
pub(super) fn row_buttons(
    ui: &mut Ui,
    rating: Rating,
    several: bool,
    open: bool,
    mode: DialogMode,
    have: Have,
) -> (bool, bool) {
    ui.spacing_mut().item_spacing.x = 8.0;
    fit_badge(ui, rating.fit, rating.need.estimated(), false);
    let version_key = if open {
        "local.dialog.collapse"
    } else {
        "local.dialog.choose_version"
    };
    let toggle = ui
        .add_enabled_ui(several, |ui| {
            secondary_button(ui, &t(version_key, &[])).clicked()
        })
        .inner;
    let (key, enabled) = match (mode, have) {
        (DialogMode::Select, _) | (_, Have::No) => (
            if mode == DialogMode::Select {
                "local.dialog.select"
            } else {
                "local.dialog.download"
            },
            true,
        ),
        (_, Have::Complete) => ("local.downloaded", false),
        (_, Have::InProgress) => ("local.in_progress", false),
    };
    let go = ui
        .add_enabled_ui(enabled, |ui| primary_button(ui, &t(key, &[])).clicked())
        .inner;
    (toggle, go)
}
