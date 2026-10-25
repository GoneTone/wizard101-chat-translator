//! 下載對話框推薦列的零件：記憶體需求文字、已下載狀態、判斷依據，以及列的文字與按鈕。

use eframe::egui::{Label, RichText, Ui};

use super::local_dialog::DialogMode;
use super::quant_table::{dot, fit_color, fit_text, gb};
use super::theme;
use super::widgets::{primary_button, secondary_button};
use crate::i18n::t;
use crate::local::Snapshot;
use crate::local::fit::{Fit, MemoryKind};
use crate::local::hardware::{Hardware, effective};

pub(super) fn memory_key(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Vram => "local.memory.vram",
        MemoryKind::VramPlusRam => "local.memory.vram_ram",
        MemoryKind::Ram => "local.memory.ram",
    }
}

/// 已下載的狀態：完整、未完成（進度在設定頁），或沒有。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Have {
    No,
    Complete,
    InProgress,
}

pub(super) fn have_of(snapshot: &Snapshot, repo: &str, quant: &str) -> Have {
    let mut models = snapshot
        .models
        .iter()
        .filter(|m| m.manifest.repo == repo && m.manifest.quant == quant);
    match models.clone().find(|m| m.complete) {
        Some(_) => Have::Complete,
        None if models.next().is_some() => Have::InProgress,
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
    level: Option<Fit>,
    several: bool,
    open: bool,
    mode: DialogMode,
    have: Have,
) -> (bool, bool) {
    ui.spacing_mut().item_spacing.x = 8.0;
    dot(ui, fit_color(level));
    ui.label(RichText::new(fit_text(level)).color(fit_color(level)));
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
