//! few-shot 範例集：請設定的模型把固定示範改寫成目標語言，驗證三條路徑並以指紋快取在磁碟。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use regex::Regex;
use serde_json::Value;
use sha1::{Digest, Sha1};

use super::postprocess::{SPACE, number_lines, numbered_entries, tidy_parentheses, unnumber_lines};
use super::{Role, Turn};
use crate::config::{ServiceSpec, local_state_dir};
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::py::pystr;

/// 範例內容（含 `GAME_LANGUAGE_EXAMPLES`）或生成提示詞有變時要遞增，讓舊快取作廢。
pub const EXAMPLE_REVISION: u32 = 4;
/// 快取最多保留的指紋數，超過就捨棄最舊的。
pub const MAX_ENTRIES: usize = 32;

const INTERFACE_LABEL: &str = "OPTIONS";

/// 請模型改寫的原文行。
pub const SOURCE_LINES: [&str; 6] = [
    "[Amy] idk, Kai and I got new armor and learned Fire Cat at Colossus Boulevard lol, brb my wand is trash",
    "Kai taught you Fire Cat! Gained {0} gold at Colossus Boulevard.",
    "Talk to the Fire Cat",
    "Go to Colossus Boulevard",
    INTERFACE_LABEL,
    "and then you must",
];

const DEMO_LINES: [&str; 6] = [
    "[Amy] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道（Colossus Boulevard）學會了火貓（Fire Cat），笑死，等我一下，我的法杖超爛",
    "Kai 教會了你火貓（Fire Cat）！在巨像大道（Colossus Boulevard）獲得了 {0} 金幣。",
    "和火貓（Fire Cat）談談",
    "前往巨像大道（Colossus Boulevard）",
    "選項",
    "然後你必須",
];

/// 一組（原文, 譯文）。
pub type Pair = (String, String);

/// 三條路徑的 few-shot 範例（原文, 譯文）；通不過驗證的路徑為 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExampleSet {
    pub incoming: Option<Pair>,
    pub system: Option<Pair>,
    pub region: Option<Pair>,
}

fn pair_repr(pair: &Option<Pair>) -> String {
    match pair {
        None => "None".into(),
        Some((a, b)) => format!("({}, {})", repr_str(a), repr_str(b)),
    }
}

impl ExampleSet {
    /// 自己已有的路徑保留，缺的用 `other` 補。
    pub fn merge(&self, other: &ExampleSet) -> ExampleSet {
        let pick = |a: &Option<Pair>, b: &Option<Pair>| a.clone().or_else(|| b.clone());
        ExampleSet {
            incoming: pick(&self.incoming, &other.incoming),
            system: pick(&self.system, &other.system),
            region: pick(&self.region, &other.region),
        }
    }

    /// 三條路徑都有範例。
    pub fn complete(&self) -> bool {
        self.incoming.is_some() && self.system.is_some() && self.region.is_some()
    }

    /// 三條路徑都沒有範例。
    pub fn empty(&self) -> bool {
        self.incoming.is_none() && self.system.is_none() && self.region.is_none()
    }

    /// Python dataclass 的 `repr`，是 `digest` 的雜湊輸入。
    fn py_repr(&self) -> String {
        format!(
            "ExampleSet(incoming={}, system={}, region={})",
            pair_repr(&self.incoming),
            pair_repr(&self.system),
            pair_repr(&self.region)
        )
    }

    /// 內容 sha1 的前 12 碼，是系統訊息譯文快取指紋的一部分。
    ///
    /// 改動這裡或 `py_repr` 會讓使用者既有的譯文快取全部作廢。
    pub fn digest(&self) -> String {
        let hash = Sha1::digest(self.py_repr().as_bytes());
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        hex[..12].to_string()
    }

    /// 各路徑成敗的一行摘要，給 log 用。
    pub fn summary(&self) -> String {
        let state = |p: &Option<Pair>| if p.is_some() { "ok" } else { "failed" };
        format!(
            "incoming={} system={} region={}",
            state(&self.incoming),
            state(&self.system),
            state(&self.region)
        )
    }
}

/// 目標語言就是遊戲內建語言時直接使用的範例，不必請模型生成。
pub static GAME_LANGUAGE_EXAMPLES: LazyLock<ExampleSet> = LazyLock::new(|| {
    let pair = |a: &str, b: &str| Some((a.to_string(), b.to_string()));
    ExampleSet {
        incoming: pair(
            "[小明] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道學會了火貓，笑死，等我一下，我的法杖超爛",
            "[小明] idk, Kai and I got new armor and learned Fire Cat (火貓) at Colossus Boulevard (巨像大道) lol, brb my wand is trash",
        ),
        system: pair(
            "Kai 教会了你火猫！在巨像大道获得了 {0} 金币。",
            "Kai taught you Fire Cat (火猫)! Gained {0} gold at Colossus Boulevard (巨像大道).",
        ),
        region: pair(
            "和火猫谈谈\n前往巨像大道\n选项\n然后你必须",
            "Talk to the Fire Cat (火猫)\nGo to Colossus Boulevard (巨像大道)\nOptions\nand then you must",
        ),
    }
});

/// 範例快取指紋：換服務商、端點、模型、目標語言或範例版次就作廢；絕不含 API 金鑰。
pub fn examples_fingerprint(spec: &ServiceSpec, target: &str) -> String {
    format!(
        "{}|{}|{}|{}|e{EXAMPLE_REVISION}",
        spec.provider,
        spec.base_url,
        spec.model,
        pystr::strip(target)
    )
}

/// 組出請模型把示範改寫成 `target` 的（system 提示詞, 對話輪）。
pub fn generation_request(target: &str) -> (String, Vec<Turn>) {
    let numbered = |lines: &[&str]| number_lines(&lines.join("\n")).1;
    let demo = numbered(&DEMO_LINES);
    let source = numbered(&SOURCE_LINES);
    let system = format!(
        "把使用者給的每一行翻成 {target}，逐行對應、保留行首編號，只輸出譯文。\
         格式照下面的中文示範：遊戲專有名詞翻成 {target} 後緊接括號照抄英文原文；\
         [Amy]、人名 Kai、{{0}} 照抄不翻；縮寫、一般名詞與介面按鈕文字直接翻、不加括號；\
         最後一行是被截斷的句子，譯文也停在同一處。\n\
         中文示範：\n{demo}"
    );
    (
        system,
        vec![Turn {
            role: Role::User,
            content: source,
        }],
    )
}

/// 名稱譯過且括號附原文；「Fire Cat (Fire Cat)」這種照抄英文再重複一次的不算。
fn in_parentheses(line: &str, name: &str) -> bool {
    let kept = tidy_parentheses(name, line);
    let pattern = format!("[（(]{SPACE}*{}{SPACE}*[)）]", regex::escape(name));
    Regex::new(&pattern)
        .unwrap_or_else(|e| panic!("bad regex {pattern}: {e}"))
        .is_match(&kept)
}

fn failures_if(ok: bool, message: &str) -> Vec<String> {
    if ok {
        vec![]
    } else {
        vec![message.to_string()]
    }
}

fn check_incoming(line: &str) -> Vec<String> {
    failures_if(
        line.contains("[Amy]")
            && line.contains("Kai")
            && in_parentheses(line, "Fire Cat")
            && in_parentheses(line, "Colossus Boulevard"),
        "line 1: needs [Amy], Kai and both names translated with the original in parentheses",
    )
}

fn check_system(line: &str) -> Vec<String> {
    failures_if(
        line.contains("Kai")
            && line.matches("{0}").count() == 1
            && in_parentheses(line, "Fire Cat")
            && in_parentheses(line, "Colossus Boulevard"),
        "line 2: needs Kai, one {0} and both names translated with the original in parentheses",
    )
}

fn check_region(lines: &[String]) -> Vec<String> {
    // 介面標籤那行示範「不加括號」：少了它，小模型會替整排短標籤都附原文
    let checks = [
        (
            3,
            in_parentheses(&lines[0], "Fire Cat"),
            "needs Fire Cat translated with the original in parentheses",
        ),
        (
            4,
            in_parentheses(&lines[1], "Colossus Boulevard"),
            "needs Colossus Boulevard translated with the original in parentheses",
        ),
        (
            5,
            !lines[2].contains(['（', '(', ')', '）']),
            "interface label must not have parentheses",
        ),
        (6, !lines[3].is_empty(), "must not be empty"),
    ];
    checks
        .into_iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(n, _, failure)| format!("line {n}: {failure}"))
        .collect()
}

/// 拆開模型生成的各行譯文並逐路徑驗證，回傳（通過的範例, 失敗說明）。
///
/// 缺編號的行會被 `unnumber_lines` 補回原文，所以與原文相同的行視同缺行。
pub fn parse_generated(output: &str) -> (ExampleSet, Vec<String>) {
    let originals: Vec<String> = SOURCE_LINES.iter().map(|s| s.to_string()).collect();
    let unnumbered = unnumber_lines(output, &originals);
    let raw = pystr::splitlines(&unnumbered);
    if raw.len() != SOURCE_LINES.len() {
        let failure = format!("line count {} != {}", raw.len(), SOURCE_LINES.len());
        return (ExampleSet::default(), vec![failure]);
    }
    let lines: Vec<String> = raw
        .into_iter()
        .map(|l| pystr::strip(l).to_string())
        .collect();
    // 介面標籤在部分語言拼法與英文相同（法文 OPTIONS），模型真的寫了這行時照抄也算譯文；
    // 漏掉而被補回原文的不算，否則範例會示範「標籤不翻」
    let label_n = SOURCE_LINES
        .iter()
        .position(|l| *l == INTERFACE_LABEL)
        .map_or(0, |i| i + 1);
    let label_answered = numbered_entries(output)
        .get(&(label_n as u128))
        .is_some_and(|text| !text.is_empty());
    let untranslated: Vec<String> = lines
        .iter()
        .zip(SOURCE_LINES)
        .enumerate()
        .filter(|(i, (line, source))| line == source && !(i + 1 == label_n && label_answered))
        .map(|(i, _)| format!("line {} untranslated", i + 1))
        .collect();
    if !untranslated.is_empty() {
        return (ExampleSet::default(), untranslated);
    }
    let incoming = check_incoming(&lines[0]);
    let system = check_system(&lines[1]);
    let region = check_region(&lines[2..]);
    let examples = ExampleSet {
        incoming: incoming
            .is_empty()
            .then(|| (SOURCE_LINES[0].to_string(), lines[0].clone())),
        system: system
            .is_empty()
            .then(|| (SOURCE_LINES[1].to_string(), lines[1].clone())),
        region: region
            .is_empty()
            .then(|| (SOURCE_LINES[2..].join("\n"), lines[2..].join("\n"))),
    };
    (examples, [incoming, system, region].concat())
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// 範例集的磁碟快取，以指紋為 key；不在記憶體保存內容，每次都直接讀檔。
pub struct ExampleStore {
    path: PathBuf,
}

impl Default for ExampleStore {
    fn default() -> Self {
        Self::new(local_state_dir().join("translation-examples.json"))
    }
}

type Entries = Vec<(String, ExampleSet)>;

fn pair_from_json(value: Option<&Value>) -> Result<Option<Pair>, String> {
    match value {
        None => Err("missing field".into()),
        Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => match items.as_slice() {
            [Value::String(a), Value::String(b)] => Ok(Some((a.clone(), b.clone()))),
            _ => Err("pair is not a list of two strings".into()),
        },
        Some(_) => Err("pair is not a list of two strings".into()),
    }
}

fn parse_entries(text: &str) -> Result<Entries, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let raw = root
        .get("entries")
        .and_then(Value::as_object)
        .ok_or("missing entries")?;
    raw.iter()
        .map(|(fingerprint, entry)| {
            let set = ExampleSet {
                incoming: pair_from_json(entry.get("incoming"))?,
                system: pair_from_json(entry.get("system"))?,
                region: pair_from_json(entry.get("region"))?,
            };
            Ok((fingerprint.clone(), set))
        })
        .collect()
}

pub(super) fn json_str(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

fn pair_json(pair: &Option<Pair>) -> String {
    match pair {
        None => "null".into(),
        Some((a, b)) => format!("[{}, {}]", json_str(a), json_str(b)),
    }
}

/// 等同 Python `json.dump({"entries": ...}, ensure_ascii=False)`（分隔符帶空格）。
fn render_entries(entries: &Entries) -> String {
    let body: Vec<String> = entries
        .iter()
        .map(|(fp, e)| {
            format!(
                "{}: {{\"incoming\": {}, \"system\": {}, \"region\": {}}}",
                json_str(fp),
                pair_json(&e.incoming),
                pair_json(&e.system),
                pair_json(&e.region)
            )
        })
        .collect();
    format!("{{\"entries\": {{{}}}}}", body.join(", "))
}

impl ExampleStore {
    /// 以指定檔案路徑建立快取。
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// 查指紋對應的範例集，沒有（或檔案壞掉）回傳 `None`。
    pub fn get(&self, fingerprint: &str) -> Option<ExampleSet> {
        self.read()
            .into_iter()
            .find(|(fp, _)| fp == fingerprint)
            .map(|(_, set)| set)
    }

    /// 存入並把該指紋移到最後，超過 `MAX_ENTRIES` 捨棄最舊的；全空的範例集不寫入。
    pub fn put(&self, fingerprint: &str, examples: &ExampleSet) {
        if examples.empty() {
            return;
        }
        let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read();
        entries.retain(|(fp, _)| fp != fingerprint);
        entries.push((fingerprint.to_string(), examples.clone()));
        let excess = entries.len().saturating_sub(MAX_ENTRIES);
        entries.drain(..excess);
        self.write(&entries);
    }

    fn read(&self) -> Entries {
        if !self.path.exists() {
            return Vec::new();
        }
        let parsed = fs::read_to_string(&self.path)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_entries(&text));
        parsed.unwrap_or_else(|err| {
            log(format!(
                "[translate] example cache unreadable ({}: {err}), treating as empty",
                self.path.display()
            ));
            Vec::new()
        })
    }

    fn write(&self, entries: &Entries) {
        match write_atomic(&self.path, &render_entries(entries)) {
            Ok(()) => log(format!(
                "[translate] example cache saved: {} entries to {}",
                entries.len(),
                self.path.display()
            )),
            Err(err) => log(format!(
                "[translate] example cache write failed ({}: {err})",
                self.path.display()
            )),
        }
    }
}

/// 先寫同目錄暫存檔再 rename，讀者永遠看不到寫到一半的檔案；失敗時清掉暫存檔。
pub(super) fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp_name = path.as_os_str().to_owned();
    // 每次呼叫獨立的暫存檔，重疊的寫入才不會互相截斷
    static SEQ: AtomicU64 = AtomicU64::new(0);
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp_name);
    let result = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
