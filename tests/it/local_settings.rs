//! 「下載模型」對話框：推薦與自訂分頁、放不下的確認、精靈用的選擇模式。

use crate::common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use common::kittest::{click, click_in_dialog, has_label, painted_texts, run, themed_harness, tr};
use common::settings_env::FakeEnv;
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::local::catalog::RECOMMENDED;
use wizard101_chat_translator::local::fit::{Fit, Pick};
use wizard101_chat_translator::local::gguf::ModelMeta;
use wizard101_chat_translator::local::hardware::{Gpu, GpuKind, Hardware};
use wizard101_chat_translator::local::hf::{Catalog, Choice, Quant, RepoFile};
use wizard101_chat_translator::local::{Command, RepoInfo, RepoState, Snapshot};
use wizard101_chat_translator::ui::settings::env::SettingsEnv;
use wizard101_chat_translator::ui::settings::local_dialog::{
    DialogMode, DialogOutcome, DownloadDialog, Selection, Tab,
};

const LANGUAGE: &str = "en-US";
const G: u64 = 1 << 30;

struct State {
    dialog: DownloadDialog,
    env: FakeEnv,
    outcome: DialogOutcome,
}

fn choice(label: &str, bits: u8, size: u64) -> Choice {
    Choice {
        quant: Quant {
            label: label.into(),
            bits: Some(bits),
        },
        files: vec![RepoFile {
            path: format!("{label}.gguf"),
            size,
            sha256: None,
        }],
    }
}

/// 兩個量化（或一個）的 repo；自動挑第一個，等級為 `fit`。
fn info(fit: Fit, moe: bool, choices: usize, required: u64) -> RepoInfo {
    let all = [choice("Q4_K_M", 4, 2 * G), choice("Q8_0", 8, 4 * G)];
    RepoInfo {
        catalog: Catalog {
            choices: all[..choices].to_vec(),
            mtp: vec![],
        },
        meta: ModelMeta {
            expert_count: if moe { 8 } else { 0 },
            ..Default::default()
        },
        fits: vec![(required, Some(fit)); choices],
        mtp_required: vec![],
        auto: Some(Pick {
            choice: 0,
            mtp: None,
            fit,
            required,
        }),
    }
}

/// 推薦清單全部載入中，`ready` 內的 repo 換成查好的結果。
fn snapshot(ready: &[(usize, RepoInfo)]) -> Snapshot {
    let mut s = Snapshot {
        hardware: Hardware {
            gpu: Some(Gpu {
                name: "t".into(),
                vendor: 0,
                kind: GpuKind::Discrete,
            }),
            vram_total: 8 * G,
            vram_free: 8 * G,
            ram_total: 16 * G,
            ram_free: 16 * G,
        },
        ..Default::default()
    };
    for r in RECOMMENDED {
        s.repos.insert(r.repo.into(), RepoState::Loading);
    }
    for (i, info) in ready {
        s.repos
            .insert(RECOMMENDED[*i].repo.into(), RepoState::Ready(info.clone()));
    }
    s
}

fn harness(mode: DialogMode, tab: Tab, snap: Snapshot) -> Harness<'static, State> {
    i18n::set_language(LANGUAGE);
    let env = FakeEnv::default();
    *env.snapshot.borrow_mut() = snap;
    let state = State {
        dialog: DownloadDialog::new(mode, tab),
        env,
        outcome: DialogOutcome::Open,
    };
    themed_harness(
        egui::vec2(760.0, 700.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut State| {
            let snap = s.env.local_snapshot();
            let outcome = s.dialog.ui(ui, &s.env, &snap);
            if outcome != DialogOutcome::Open {
                s.outcome = outcome;
            }
        },
        state,
    )
}

fn size_arg(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / G as f64)
}

#[test]
fn recommended_rows_show_memory_kind_not_tier() {
    let snap = snapshot(&[
        (0, info(Fit::Smooth, false, 2, 3 * G)),
        (4, info(Fit::Offload, true, 2, 18 * G)),
    ]);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let texts = painted_texts(&h);
    let vram = t("local.memory.vram", &[("size", &size_arg(3 * G))]);
    let both = t("local.memory.vram_ram", &[("size", &size_arg(18 * G))]);
    assert!(texts.contains(&vram), "{texts:?}");
    assert!(texts.contains(&both), "{texts:?}");
    assert!(!include_str!("../../src/i18n/locales/en-US.json").contains("local.tier."));
    let queried = h.state().env.commands.borrow().len();
    assert_eq!(
        queried, 0,
        "every recommended repo is already in the snapshot"
    );
}

#[test]
fn opening_queries_only_repos_missing_from_the_snapshot() {
    let mut snap = snapshot(&[]);
    snap.repos.remove(RECOMMENDED[0].repo);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let commands = h.state().env.commands.borrow();
    assert!(
        matches!(&commands[..], [Command::QueryRepo(r)] if r == RECOMMENDED[0].repo),
        "{commands:?}"
    );
}

#[test]
fn single_quant_repo_disables_choose_version() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 1, 3 * G))]);
    let h = harness(DialogMode::Download, Tab::Recommended, snap);
    let label = tr("local.dialog.choose_version");
    let choose = h.get_by_role_and_label(Role::Button, &label);
    assert!(choose.accesskit_node().is_disabled());
}

#[test]
fn custom_tab_rejects_non_hf_input() {
    let mut h = harness(DialogMode::Download, Tab::Custom, snapshot(&[]));
    h.get_by_role_and_label(Role::TextInput, &tr("local.dialog.repo_hint"))
        .click();
    h.event(Event::Text("https://example.com/a/b".into()));
    run(&mut h);
    click(&mut h, Role::Button, &tr("local.dialog.query"));
    assert!(has_label(&h, &tr("local.dialog.bad_repo")));
    assert!(h.state().env.commands.borrow().is_empty());
}

#[test]
fn nofit_download_asks_first() {
    let snap = snapshot(&[(0, info(Fit::NoFit, false, 2, 40 * G))]);
    let mut h = harness(DialogMode::Download, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.download"));
    assert!(has_label(&h, &tr("local.dialog.nofit_confirm")));
    assert!(h.state().env.commands.borrow().is_empty());
    click_in_dialog(
        &mut h,
        &tr("dialog.confirm_title"),
        &tr("local.dialog.download"),
    );
    let commands = h.state().env.commands.borrow();
    assert!(
        matches!(&commands[..], [Command::Download { repo, choice: 0, mtp: None }] if repo == RECOMMENDED[0].repo),
        "{commands:?}"
    );
    assert_eq!(h.state().outcome, DialogOutcome::Closed);
}

#[test]
fn select_mode_returns_the_choice() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    let mut h = harness(DialogMode::Select, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.select"));
    assert_eq!(
        h.state().outcome,
        DialogOutcome::Selected(Selection {
            repo: RECOMMENDED[0].repo.into(),
            choice: 0,
            mtp: None
        })
    );
    assert!(h.state().env.commands.borrow().is_empty());
}

#[test]
fn picking_a_quant_chip_changes_the_selection() {
    let snap = snapshot(&[(0, info(Fit::Smooth, false, 2, 3 * G))]);
    let mut h = harness(DialogMode::Select, Tab::Recommended, snap);
    click(&mut h, Role::Button, &tr("local.dialog.choose_version"));
    click(&mut h, Role::RadioButton, "Q8_0");
    click(&mut h, Role::Button, &tr("local.dialog.select"));
    assert!(matches!(
        &h.state().outcome,
        DialogOutcome::Selected(Selection { choice: 1, .. })
    ));
}
