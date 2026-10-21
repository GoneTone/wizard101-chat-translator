//! 系統訊息譯文的持久化快取：數字先正規化成佔位符當 key，同句型不同金額共用一筆。
//!
//! 系統訊息不吃聊天上下文，同一句原文必然得到同一句譯文，因此可以安全快取。

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

use regex::{Captures, Regex};
use serde_json::{Map, Value, json};

use super::errors::TranslateError;
use super::postprocess::has_stray_latin;
use super::prompts::PROMPT_REVISION;
use super::translator::Translator;
use crate::config::{local_state_dir, py};
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::util::{lock, write_atomic};

/// 系統訊息的句型與材料名是有限集合，2000 筆足以涵蓋。
pub const MAX_ENTRIES: usize = 2000;
/// 累積這麼多筆新增才落盤一次（不逐筆寫）。
pub const FLUSH_EVERY: usize = 20;

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:[.,]\d+)*").unwrap());
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\d+)\}").unwrap());

/// 把文字中的數字換成依序編號的佔位符，回傳（樣板，依序取出的數字）。
pub fn normalize(text: &str) -> (String, Vec<String>) {
    let mut numbers = Vec::new();
    let template = NUMBER.replace_all(text, |m: &Captures| {
        numbers.push(m[0].to_string());
        format!("{{{}}}", numbers.len() - 1)
    });
    (template.into_owned(), numbers)
}

/// 把數字填回樣板；依佔位符的編號取值而非出現位置，因為譯文語序可能與原文不同。
pub fn restore(template: &str, numbers: &[String]) -> String {
    // shortcut: 非 ASCII 數字的編號（如 `{٣}`）當作越界原樣保留，Python 會解成 3；模型不會產出，需要時再補
    PLACEHOLDER
        .replace_all(template, |m: &Captures| {
            match m[1].parse::<usize>().ok().and_then(|i| numbers.get(i)) {
                Some(n) => n.clone(),
                None => m[0].to_string(),
            }
        })
        .into_owned()
}

fn placeholders(text: &str) -> Vec<&str> {
    let mut found: Vec<&str> = PLACEHOLDER
        .captures_iter(text)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    found.sort_unstable();
    found
}

/// 譯文的佔位符是否與樣板完全一致（含重複次數）。
pub fn placeholders_match(template: &str, translated: &str) -> bool {
    placeholders(template) == placeholders(translated)
}

/// 快取指紋：換服務商、端點、模型、目標語言、範例或提示詞版次時舊譯文整份作廢；絕不含 API 金鑰。
pub fn fingerprint_of(
    provider: &str,
    model: &str,
    target_language: &str,
    base_url: &str,
    examples_digest: &str,
) -> String {
    format!("{provider}|{base_url}|{model}|{target_language}|p{PROMPT_REVISION}|x{examples_digest}")
}

/// 快取檔的預設位置。
pub fn default_path() -> PathBuf {
    local_state_dir().join("system-message-cache.json")
}

/// 樣板 → （譯文樣板, 最近使用序號）。
type Entries = HashMap<String, (String, u64)>;

#[derive(Default)]
struct Inner {
    fingerprint: String,
    /// 樣板 → （譯文樣板, 最近使用序號）；序號最小的是 LRU 淘汰對象。
    entries: Entries,
    tick: u64,
    unflushed: usize,
    /// 上一組指紋與其條目；換回該指紋時直接換回來，不必重翻。
    previous: Option<(String, Entries)>,
}

impl Inner {
    fn touch(&mut self, key: &str, translated: String) {
        self.tick += 1;
        self.entries
            .insert(key.to_string(), (translated, self.tick));
    }

    fn ordered(&self) -> Vec<(&String, &String)> {
        let mut items: Vec<_> = self.entries.iter().map(|(k, (v, t))| (*t, k, v)).collect();
        items.sort_unstable_by_key(|(t, _, _)| *t);
        items.into_iter().map(|(_, k, v)| (k, v)).collect()
    }
}

/// 系統訊息譯文快取；reader 查詢與翻譯 worker 寫入共用同一把鎖。
pub struct TranslationCache {
    path: PathBuf,
    inner: Mutex<Inner>,
}

impl TranslationCache {
    /// 以檔案路徑與目前生效的指紋建立空快取（尚未讀檔，見 `load`）。
    pub fn new(path: PathBuf, fingerprint: String) -> Self {
        Self {
            path,
            inner: Mutex::new(Inner {
                fingerprint,
                ..Inner::default()
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    /// 目前生效的指紋；翻譯開始前先取一份，寫回時交給 `put` 比對。
    pub fn fingerprint(&self) -> String {
        self.lock().fingerprint.clone()
    }

    /// 查快取，命中回傳已回填數字的譯文。
    pub fn get(&self, text: &str) -> Option<String> {
        let (template, numbers) = normalize(text);
        let translated = {
            let mut inner = self.lock();
            let translated = inner.entries.get(&template)?.0.clone();
            inner.touch(&template, translated.clone());
            translated
        };
        Some(restore(&translated, &numbers))
    }

    /// 存入一筆（`translated_template` 是樣板的譯文）；佔位符對不上或指紋已過期回傳 `false`。
    ///
    /// `fingerprint` 是譯文產出當下的指紋：翻譯飛行中若已 `rebind`，舊設定的譯文不得存進新快取。
    pub fn put(&self, text: &str, translated_template: &str, fingerprint: &str) -> bool {
        let (template, _) = normalize(text);
        if !placeholders_match(&template, translated_template) {
            log(format!(
                "[cache] placeholder mismatch, not cached: template={} translated={}",
                repr_str(&template),
                repr_str(translated_template)
            ));
            return false;
        }
        let due = {
            let mut inner = self.lock();
            if fingerprint != inner.fingerprint {
                log(format!(
                    "[cache] fingerprint changed while translating, discarding stale translation: template={} produced_under={} current={}",
                    repr_str(&template),
                    repr_str(fingerprint),
                    repr_str(&inner.fingerprint)
                ));
                return false;
            }
            inner.touch(&template, translated_template.to_string());
            while inner.entries.len() > MAX_ENTRIES {
                let oldest = inner
                    .entries
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| k.clone())
                    .unwrap();
                inner.entries.remove(&oldest);
                log(format!(
                    "[cache] evicted least recently used entry: {}",
                    repr_str(&oldest)
                ));
            }
            inner.unflushed += 1;
            inner.unflushed >= FLUSH_EVERY
        };
        if due {
            self.flush();
        }
        true
    }

    /// 指紋變更（換收訊用的服務或目標語言）：先落盤舊的，舊條目暫存一份；換回暫存的指紋就還原。
    pub fn rebind(&self, fingerprint: &str) {
        if fingerprint == self.lock().fingerprint {
            return;
        }
        self.flush();
        let mut inner = self.lock();
        let restored = match inner.previous.take() {
            Some((previous, entries)) if previous == fingerprint => Some(entries),
            _ => None,
        };
        log(format!(
            "[cache] fingerprint changed at runtime, stashing {} entries (restored_from_stash={})",
            inner.entries.len(),
            restored.is_some()
        ));
        let was_restored = restored.is_some();
        let old = (
            std::mem::replace(&mut inner.fingerprint, fingerprint.to_string()),
            std::mem::replace(&mut inner.entries, restored.unwrap_or_default()),
        );
        inner.previous = Some(old);
        // 換回時磁碟檔還是 B 的內容；記成未落盤，下一輪 put 就會寫回 A
        inner.unflushed = if was_restored { FLUSH_EVERY } else { 0 };
    }

    /// 使用者主動清空：不寫回舊內容、刪掉磁碟檔，回傳清掉的筆數；刪檔失敗只記 log。
    pub fn clear(&self) -> usize {
        let count = {
            let mut inner = self.lock();
            let count = inner.entries.len();
            inner.entries.clear();
            inner.unflushed = 0;
            count
        };
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => log(format!(
                "[cache] could not delete {}: {e}",
                self.path.display()
            )),
        }
        log(format!("[cache] cleared {count} entries on user request"));
        count
    }

    /// 從磁碟載入；指紋不符、檔案損壞或不存在一律當作空快取。
    pub fn load(&self) {
        if !self.path.exists() {
            log("[cache] no cache file yet, starting empty");
            return;
        }
        let (stored, entries) = match self.read_file() {
            Ok(parsed) => parsed,
            Err(err) => {
                log(format!(
                    "[cache] unreadable cache file, starting empty: {err}"
                ));
                return;
            }
        };
        let mut inner = self.lock();
        if stored.as_str() != Some(inner.fingerprint.as_str()) {
            log(format!(
                "[cache] fingerprint changed, discarding {} entries (stored={}, current={})",
                entries.len(),
                py::repr(&stored),
                repr_str(&inner.fingerprint)
            ));
            return;
        }
        let skip = entries.len().saturating_sub(MAX_ENTRIES);
        inner.entries.clear();
        let mut kept = 0;
        for (key, value) in entries.into_iter().skip(skip) {
            inner.touch(&key, value);
            kept += 1;
        }
        log(format!(
            "[cache] loaded {kept} entries from {}",
            self.path.display()
        ));
    }

    fn read_file(&self) -> Result<(Value, Vec<(String, String)>), String> {
        let text = fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let root = root.as_object().ok_or("root is not an object")?;
        let stored = root.get("fingerprint").cloned().unwrap_or(Value::Null);
        let entries = match root.get("entries") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Object(map)) => map
                .iter()
                .map(|(k, v)| match v {
                    Value::String(s) => Ok((k.clone(), s.clone())),
                    _ => Err(format!("entry value is not a string: {}", repr_str(k))),
                })
                .collect::<Result<_, _>>()?,
            Some(other) if !py::truthy(Some(other)) => Vec::new(),
            Some(other) => {
                return Err(format!(
                    "entries is {}, expected dict",
                    py::type_name(other)
                ));
            }
        };
        Ok((stored, entries))
    }

    /// 原子寫回磁碟（暫存檔再 rename）；寫檔失敗只記 log。
    pub fn flush(&self) {
        let (text, count) = {
            let mut inner = self.lock();
            inner.unflushed = 0;
            let entries: Map<String, Value> = inner
                .ordered()
                .into_iter()
                .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
                .collect();
            let text = json!({ "fingerprint": inner.fingerprint, "entries": entries }).to_string();
            (text, inner.entries.len())
        };
        match write_atomic(&self.path, &text) {
            Ok(()) => log(format!(
                "[cache] flushed {count} entries to {}",
                self.path.display()
            )),
            Err(e) => log(format!(
                "[cache] flush failed ({}: {e})",
                self.path.display()
            )),
        }
    }
}

/// 翻一則系統訊息並存進快取；佔位符被弄壞或翻譯期間指紋被換掉時不快取，改用原文直翻一次。
///
/// 落回英文的譯文（見 `has_stray_latin`）照樣回傳但不落盤，避免把錯誤固化。
pub async fn translate_and_cache(
    tr: &Translator,
    cache: &TranslationCache,
    text: &str,
) -> Result<String, TranslateError> {
    let (template, numbers) = normalize(text);
    let fingerprint = cache.fingerprint();
    let target = tr.target_language();
    let translated = tr.translate_system_message(&template).await?;
    if has_stray_latin(&template, &translated, &target) {
        log(format!(
            "[cache] translation is not in the target language, not cached: template={} translated={}",
            repr_str(&template),
            repr_str(&translated)
        ));
        return Ok(restore(&translated, &numbers));
    }
    // put 收原文並自行正規化；傳樣板會把 `{0}` 的 0 再當數字
    if cache.put(text, &translated, &fingerprint) {
        return Ok(restore(&translated, &numbers));
    }
    log(format!(
        "[cache] falling back to a direct translation: {}",
        repr_str(text)
    ));
    tr.translate_system_message(text).await
}
