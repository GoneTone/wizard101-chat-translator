//! 服務分頁編輯中的草稿：服務清單、預設服務與各用途指定，以及它們的增刪改。

use serde_json::Value;

use crate::config::Config;
use crate::config::services::{ServiceSlot, str_field, unique_name};
use crate::i18n::t;
use crate::logs::log;

pub(super) fn id_of(service: &Value) -> &str {
    str_field(service, "id")
}

/// 服務分頁編輯中的三個設定鍵：服務清單、預設服務與各用途指定；`write_into` 寫回設定。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceDraft {
    pub services: Vec<Value>,
    pub default: Option<String>,
    /// 各用途指定的服務 id，順序同 `ServiceSlot::ALL`；`None` 是跟隨預設。
    pub slots: [Option<String>; 3],
}

impl ServiceDraft {
    /// 從設定取出三個鍵。
    pub fn from_config(cfg: &Config) -> Self {
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let slots = cfg.get("service_slots");
        Self {
            services: cfg
                .get("services")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            default: text(cfg.get("default_service")),
            slots: ServiceSlot::ALL.map(|slot| text(slots.and_then(|s| s.get(slot.key())))),
        }
    }

    /// 寫回設定；`service_slots` 裡認不得的鍵原樣保留。
    pub fn write_into(&self, cfg: &mut Config) {
        cfg.insert("services".into(), Value::Array(self.services.clone()));
        cfg.insert("default_service".into(), self.default.clone().into());
        let mut slots = cfg
            .get("service_slots")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (slot, id) in ServiceSlot::ALL.iter().zip(&self.slots) {
            slots.insert(slot.key().into(), id.clone().into());
        }
        cfg.insert("service_slots".into(), Value::Object(slots));
    }

    /// 這筆服務擔任的角色：先「預設」，再依用途順序。
    pub(super) fn badges(&self, service: &Value) -> Vec<String> {
        let id = id_of(service);
        let default =
            (self.default.as_deref() == Some(id)).then(|| t("service.badge_default", &[]));
        let slots = ServiceSlot::ALL
            .iter()
            .zip(&self.slots)
            .filter(|(_, assigned)| assigned.as_deref() == Some(id))
            .map(|(slot, _)| t(&format!("slot.{}", slot.key()), &[]));
        default.into_iter().chain(slots).collect()
    }

    /// 卡片的標題：服務名稱後接它擔任的角色，以 ` · ` 分隔。
    pub fn card_label(&self, service: &Value) -> String {
        let name = str_field(service, "name");
        let badges = self.badges(service);
        if badges.is_empty() {
            name.to_owned()
        } else {
            format!("{name}\u{3000}{}", badges.join(" · "))
        }
    }

    /// 最後一筆不給刪：刪光就無從翻譯。
    pub fn can_delete(&self) -> bool {
        self.services.len() > 1
    }

    /// 把對話框確定的服務併回清單（同 id 就地取代，否則加在最後）；清單原本是空的就接手預設。
    pub fn apply_dialog_result(&mut self, mut service: Value) {
        let id = id_of(&service).to_owned();
        // 三個用途下拉只顯示名稱，同名會讓人選錯
        service["name"] =
            unique_name(str_field(&service, "name"), &self.services, Some(&id)).into();
        match self.services.iter_mut().find(|s| id_of(s) == id) {
            Some(existing) => *existing = service,
            None => self.services.push(service),
        }
        if self.default.is_none() {
            log(format!("[settings] first service added; default={id}"));
            self.default = Some(id);
        }
    }

    /// 刪除一筆服務：指到它的用途改回跟隨預設，被刪的是預設就交給清單第一筆。
    pub fn delete(&mut self, id: &str) {
        if !self.can_delete() {
            return;
        }
        self.services.retain(|s| id_of(s) != id);
        for (slot, assigned) in ServiceSlot::ALL.iter().zip(&mut self.slots) {
            if assigned.as_deref() == Some(id) {
                *assigned = None;
                log(format!(
                    "[settings] slot {} followed the default after its service was deleted",
                    slot.key()
                ));
            }
        }
        if self.default.as_deref() == Some(id) {
            self.default = self.services.first().map(|s| id_of(s).to_owned());
            log(format!(
                "[settings] default service deleted; default={}",
                self.default.as_deref().unwrap_or("none")
            ));
        }
    }

    pub fn set_default(&mut self, id: &str) {
        self.default = Some(id.to_owned());
        log(format!("[settings] default service set to {id}"));
    }

    /// 指定用途的服務；`None` 是跟隨預設。
    pub fn set_slot(&mut self, slot: ServiceSlot, id: Option<String>) {
        log(format!(
            "[settings] slot {} set to {}",
            slot.key(),
            id.as_deref().unwrap_or("default")
        ));
        if let Some(index) = ServiceSlot::ALL.iter().position(|s| *s == slot) {
            self.slots[index] = id;
        }
    }

    pub(super) fn find(&self, id: Option<&str>) -> Option<&Value> {
        let id = id?;
        self.services.iter().find(|s| id_of(s) == id)
    }
}
