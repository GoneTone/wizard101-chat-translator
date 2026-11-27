//! 帳號新增／編輯對話框：類型、名稱、密碼（可顯示），依類型顯示說明；名稱空白、重名或新帳號沒填密碼時停用〔儲存〕。

use eframe::egui::text_edit::TextEditState;
use eframe::egui::{Frame, Id, Margin, Ui};

use super::dialog::{Choice, DialogSpec, dialog};
use super::fields::{secret_field, text_field};
use super::service_form::form_row;
use super::theme;
use super::widgets::{hint, primary_button, secondary_button};
use crate::i18n::t;
use crate::launcher::accounts::encrypt_password;
use crate::launcher::{Account, AccountKind};
use crate::logs::log;
use crate::py::pystr;

/// 帳號類型的顯示名稱。
pub(super) fn kind_label(kind: AccountKind) -> String {
    let key = match kind {
        AccountKind::Official => "accounts.kind_official",
        AccountKind::Steam => "accounts.kind_steam",
    };
    t(key, &[])
}

/// 開著的帳號對話框；明文密碼只在這裡，關閉時連同輸入框的狀態一起丟掉。刻意不實作 `Debug`。
pub(super) struct AccountDialog {
    /// 開啟時的帳號；新增時名稱與密碼都是空的。
    original: Account,
    is_new: bool,
    kind: AccountKind,
    name: String,
    password: String,
    revealed: bool,
    /// 密碼輸入框的 Id；egui 的復原紀錄會留著明文，關閉時要清掉。
    password_field: Option<Id>,
    /// 加密失敗的系統錯誤。
    crypt_error: Option<String>,
}

impl AccountDialog {
    /// 新增帳號（預設官網）；`id` 是新帳號的 id。
    pub(super) fn new(id: String) -> Self {
        let original = Account {
            id,
            name: String::new(),
            kind: AccountKind::Official,
            password_dpapi: String::new(),
        };
        Self::open(original, true)
    }

    /// 編輯既有帳號；密碼欄留空就沿用原本的密碼。
    pub(super) fn edit(account: &Account) -> Self {
        Self::open(account.clone(), false)
    }

    fn open(original: Account, is_new: bool) -> Self {
        Self {
            kind: original.kind,
            name: original.name.clone(),
            original,
            is_new,
            password: String::new(),
            revealed: false,
            password_field: None,
            crypt_error: None,
        }
    }

    /// 有尚未存下的修改。
    pub(super) fn unsaved(&self) -> bool {
        self.kind != self.original.kind
            || self.name != self.original.name
            || !self.password.is_empty()
    }

    /// 畫對話框；`accounts` 是目前的帳號清單，用來擋重名。存好回傳加密後的帳號。
    pub(super) fn show(&mut self, ui: &Ui, accounts: &[Account]) -> Choice<Account> {
        let name_error = self.name_error(accounts);
        let password_error =
            (self.is_new && self.password.is_empty()).then_some("error.need_password");
        let valid = name_error.is_none() && password_error.is_none();
        let title = if self.is_new {
            "accounts.new_title"
        } else {
            "accounts.edit_title"
        };
        let spec = DialogSpec {
            salt: "account_dialog",
            width: theme::ACCOUNT_DIALOG_WIDTH,
            title: t(title, &[]),
            closable: true,
            pinned_top: true,
        };
        let shown = dialog(
            ui,
            spec,
            |ui| {
                Frame::new()
                    .inner_margin(Margin {
                        left: 20,
                        right: 20,
                        top: 8,
                        bottom: 16,
                    })
                    .show(ui, |ui| self.body(ui, name_error, password_error));
            },
            |ui| {
                let cancel = secondary_button(ui, &t("button.cancel", &[])).clicked();
                let ok = ui
                    .add_enabled_ui(valid, |ui| primary_button(ui, &t("button.save", &[])))
                    .inner
                    .clicked();
                (ok, cancel)
            },
        );
        let (ok, cancel) = shown.footer;
        let choice = if cancel || shown.cancel {
            Choice::Cancel
        } else if ok && valid {
            self.save()
        } else {
            Choice::Open
        };
        if !matches!(choice, Choice::Open) {
            self.forget_password(ui);
        }
        choice
    }

    fn body(&mut self, ui: &mut Ui, name_error: Option<&str>, password_error: Option<&str>) {
        ui.spacing_mut().item_spacing.y = 12.0;
        form_row(ui, &t("accounts.field_kind", &[]), false, |ui, _| {
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                for kind in [AccountKind::Official, AccountKind::Steam] {
                    if ui
                        .selectable_label(self.kind == kind, kind_label(kind))
                        .clicked()
                    {
                        self.kind = kind;
                    }
                }
            });
            let key = match self.kind {
                AccountKind::Official => "accounts.hint_official",
                AccountKind::Steam => "accounts.hint_steam",
            };
            hint(ui, &t(key, &[]), theme::TEXT_SECONDARY);
        });
        form_row(ui, &t("accounts.field_name", &[]), true, |ui, label| {
            text_field(ui, &mut self.name).labelled_by(label);
            if let Some(key) = name_error {
                hint(ui, &t(key, &[]), theme::ERROR);
            }
        });
        let label = t("accounts.field_password", &[]);
        form_row(ui, &label, self.is_new, |ui, label| {
            let (show, hide) = (t("button.show", &[]), t("button.hide", &[]));
            let field = secret_field(ui, &mut self.password, &mut self.revealed, &show, &hide)
                .labelled_by(label);
            self.password_field = Some(field.id);
            if let Some(key) = password_error {
                hint(ui, &t(key, &[]), theme::ERROR);
            } else if !self.is_new {
                hint(ui, &t("accounts.password_keep", &[]), theme::TEXT_SECONDARY);
            }
            if let Some(error) = &self.crypt_error {
                let text = t("error.password_encrypt_failed", &[("error", error)]);
                hint(ui, &text, theme::ERROR);
            }
        });
    }

    /// 名稱沒過的原因：空白，或與其他帳號同名（不分 ASCII 大小寫）。
    fn name_error(&self, accounts: &[Account]) -> Option<&'static str> {
        let name = pystr::strip(&self.name);
        if name.is_empty() {
            return Some("error.need_name");
        }
        accounts
            .iter()
            .any(|a| a.id != self.original.id && pystr::strip(&a.name).eq_ignore_ascii_case(name))
            .then_some("error.account_name_taken")
    }

    fn save(&mut self) -> Choice<Account> {
        let mut account = self.original.clone();
        account.kind = self.kind;
        account.name = pystr::strip(&self.name).to_owned();
        let changed = !self.password.is_empty();
        if changed {
            match encrypt_password(&self.password) {
                Ok(encrypted) => account.password_dpapi = encrypted,
                Err(e) => {
                    log(format!(
                        "[settings] account {} not saved: {}",
                        account.id, e.0
                    ));
                    self.crypt_error = Some(e.0);
                    return Choice::Open;
                }
            }
        }
        log(format!(
            "[settings] account {} saved (kind={:?}, new={}, password_changed={changed})",
            account.id, account.kind, self.is_new
        ));
        Choice::Done(account)
    }

    fn forget_password(&mut self, ui: &Ui) {
        self.password.clear();
        if let Some(id) = self.password_field {
            ui.ctx().data_mut(|d| d.remove::<TextEditState>(id));
        }
    }
}
