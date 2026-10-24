//! 翻譯入口：Translator 依用途（收訊、發話、系統訊息、框選）組提示詞、送出請求、後處理譯文。
//!
//! 收訊翻成使用者設定的目標語言，發話翻成 [`OUTGOING_LANGUAGE`]，來源語言一律自動判斷。
//! `BadOutput`（譯文被截斷、被服務商擋下或沒有文字）在翻譯路徑重試無用、該行應跳過；只有生成範例會重試它。
//! 上下文由呼叫端提供；Translator 只持有一份不可變的設定快照，每次請求只讀一次，平行呼叫仍安全。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::Turn;
use super::clients::{ChatBackend, ChatRequest, MAX_TOKENS_REGION, ModelInfo, build_client};
use super::errors::TranslateError;
use super::examples::{
    ExampleSet, ExampleStore, GAME_LANGUAGE_EXAMPLES, examples_fingerprint, generation_request,
    parse_generated,
};
use super::postprocess::{
    has_stray_latin, message_body, nonblank_lines, restore_sender, sender_of, tidy_parentheses,
};
use super::prompts::{
    CONTEXT_INTRO_INCOMING, CONTEXT_INTRO_OUTGOING, GLOSSARY_KEEP_ORIGINAL, OUTGOING_LANGUAGE,
    build_incoming_system, build_outgoing_system, build_region_system, build_system_message_system,
    build_turns, example_turns, fewshot_outgoing_turns, glossary_outgoing_note, glossary_turns,
    is_game_language,
};
use crate::config::ServiceSpec;
use crate::glossary::{Entry, Glossary, hits_key};
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::py::pystr;
use crate::util::{lock, read, write};

/// 測試連線用的固定原文。
pub const TEST_SAMPLE: &str = "[Tester] Hello! How are you?";
const EXAMPLE_ATTEMPTS: usize = 6;

type Connect = Box<dyn Fn(&ServiceSpec) -> Arc<dyn ChatBackend> + Send + Sync>;

/// 一份生效中的設定：後端、服務設定、目標語言與範例；換設定就整份換掉，不改欄位。
#[derive(Clone)]
struct Binding {
    client: Arc<dyn ChatBackend>,
    spec: ServiceSpec,
    target_language: String,
    examples: Option<ExampleSet>,
    glossary: Option<Arc<Glossary>>,
}

/// 系統訊息的譯文，與這次請求用到的術語鍵（供呼叫端決定快取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemTranslation {
    pub text: String,
    pub glossary_key: String,
}

impl Binding {
    fn fingerprint(&self) -> String {
        examples_fingerprint(&self.spec, &self.target_language)
    }

    fn examples(&self, pick: fn(&ExampleSet) -> Option<&(String, String)>) -> Vec<Turn> {
        example_turns(self.examples.as_ref().and_then(pick))
    }

    fn hits(&self, text: &str) -> Vec<Entry> {
        self.glossary
            .as_ref()
            .map_or_else(Vec::new, |g| g.hits(text))
    }

    /// `text` 命中的術語輪（含命中 log）；沒命中回空，請求維持原樣。
    fn glossary_turns(&self, kind: &str, text: &str, note: &str) -> Vec<Turn> {
        let hits = self.hits(text);
        if !hits.is_empty() {
            let terms: Vec<String> = hits.iter().map(Entry::to_string).collect();
            log(format!(
                "[translate] {kind} glossary hits: {} ({})",
                hits.len(),
                terms.join("; ")
            ));
        }
        glossary_turns(&hits, note)
    }

    /// 打一次翻譯請求並記錄一行診斷；這是各方向共用的唯一成功路徑 log 點。
    async fn chat(
        &self,
        system: &str,
        turns: &[Turn],
        call: Call<'_>,
    ) -> Result<String, TranslateError> {
        if call.cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(TranslateError::Cancelled);
        }
        let started = Instant::now();
        let req = ChatRequest {
            system,
            turns,
            max_tokens: call.max_tokens,
            deterministic: true,
        };
        let mut translated = self.client.chat(req, call.cancel).await?;
        if call.strip {
            translated = tidy_parentheses(call.source, &translated);
        }
        if pystr::strip(&translated).is_empty() {
            log(format!(
                "[translate] {} came back with no text (model={}): source={}",
                call.kind,
                self.client.model(),
                repr_str(call.source)
            ));
            return Err(TranslateError::BadOutput("model returned no text".into()));
        }
        if call.keep_sender {
            let restored = restore_sender(call.source, &translated);
            if sender_of(&restored) != sender_of(&translated) {
                log(format!(
                    "[translate] model altered the sender prefix, restored: source={} model_output={}",
                    repr_str(call.source),
                    repr_str(&translated)
                ));
            }
            translated = restored;
        }
        let shown = if call.redact {
            format!("<{} chars>", translated.chars().count())
        } else {
            repr_str(&translated)
        };
        log(format!(
            "[translate] {} done in {:.1}s (model={}, ctx={}): source={} translated={shown}",
            call.kind,
            started.elapsed().as_secs_f64(),
            self.client.model(),
            call.context_lines,
            repr_str(call.source)
        ));
        Ok(translated)
    }
}

/// 送出一次翻譯請求的選項；`kind` 與 `source` 只用於診斷 log。
#[derive(Default)]
struct Call<'a> {
    kind: &'a str,
    source: &'a str,
    context_lines: usize,
    strip: bool,
    keep_sender: bool,
    max_tokens: Option<u32>,
    redact: bool,
    cancel: Option<&'a CancellationToken>,
}

/// 共用翻譯 client：依服務商選擇後端，以 `Arc` 在執行緒間共享。
///
/// 每個請求開頭讀一次設定快照、之後只用它，請求期間的 `reconfigure`／`set_examples`
/// 才不會讓提示詞、範例與後端混用兩份設定。
pub struct Translator {
    connect: Connect,
    binding: RwLock<Arc<Binding>>,
}

/// 服務 id 與名稱不影響請求。
fn same_service(a: &ServiceSpec, b: &ServiceSpec) -> bool {
    let request_fields = |s: &ServiceSpec| ServiceSpec {
        id: String::new(),
        name: String::new(),
        ..s.clone()
    };
    request_fields(a) == request_fields(b)
}

impl Translator {
    /// 依服務設定建立後端。
    pub fn new(spec: ServiceSpec, target_language: String) -> Arc<Self> {
        Self::with_client(
            |spec| Arc::new(build_client(spec)) as Arc<dyn ChatBackend>,
            spec,
            target_language,
        )
    }

    /// 測試注入：後端一律由 `connect` 建立（含 `reconfigure` 重建時）。
    pub fn with_client(
        connect: impl Fn(&ServiceSpec) -> Arc<dyn ChatBackend> + Send + Sync + 'static,
        spec: ServiceSpec,
        target_language: String,
    ) -> Arc<Self> {
        let binding = Binding {
            client: connect(&spec),
            spec,
            target_language,
            examples: None,
            glossary: None,
        };
        Arc::new(Self {
            connect: Box::new(connect),
            binding: RwLock::new(Arc::new(binding)),
        })
    }

    fn binding(&self) -> Arc<Binding> {
        Arc::clone(&read(&self.binding))
    }

    /// 設定變更後就地重建後端，回傳是否重建；範例屬於舊設定，不帶過去。
    ///
    /// 值與目前生效的相同時整個跳過：重建會丟掉端點學到的參數限制（見 OpenAI 相容 client），
    /// 得重新付一輪 400 才學回來。飛行中的請求持有舊快照，照樣用舊後端跑完。
    pub fn reconfigure(&self, spec: ServiceSpec, target_language: String) -> bool {
        let mut binding = write(&self.binding);
        if same_service(&spec, &binding.spec) && target_language == binding.target_language {
            return false;
        }
        *binding = Arc::new(Binding {
            client: (self.connect)(&spec),
            spec,
            target_language,
            examples: None,
            glossary: binding.glossary.clone(),
        });
        true
    }

    /// 目前生效的服務，失敗 log 用的診斷欄位；金鑰絕不列入。
    pub fn describe(&self) -> String {
        let b = self.binding();
        format!("provider={}, model={}", b.spec.provider, b.client.model())
    }

    /// 先連上目前服務的端點，免得下一則翻譯付交握時間；尚未設定服務時略過。
    pub async fn warm_up(&self) {
        let b = self.binding();
        if !b.spec.id.is_empty() {
            b.client.warm_up(&self.describe()).await;
        }
    }

    /// 目前套用的 few-shot 範例；尚未設定為 `None`（不放範例輪）。
    pub fn examples(&self) -> Option<ExampleSet> {
        self.binding().examples.clone()
    }

    /// 目前服務與目標語言對應的範例指紋。
    pub fn examples_fingerprint(&self) -> String {
        self.binding().fingerprint()
    }

    /// 套用範例；指紋與目前設定不符（生成期間設定被改了）就不套用並回 `false`。
    pub fn set_examples(&self, examples: Option<ExampleSet>, fingerprint: &str) -> bool {
        let mut binding = write(&self.binding);
        if fingerprint != binding.fingerprint() {
            log(format!(
                "[translate] ignoring examples for a stale fingerprint: {fingerprint}"
            ));
            return false;
        }
        *binding = Arc::new(Binding {
            examples,
            ..Binding::clone(&binding)
        });
        true
    }

    /// 換術語表；只動快照裡的術語表，後端與範例不變。
    pub fn set_glossary(&self, glossary: Option<Arc<Glossary>>) {
        let mut binding = write(&self.binding);
        *binding = Arc::new(Binding {
            glossary,
            ..Binding::clone(&binding)
        });
    }

    /// `text` 命中的術語鍵；沒啟用術語表或沒命中為空字串。
    pub fn glossary_key(&self, text: &str) -> String {
        hits_key(&self.binding().hits(text))
    }

    /// 目前的目標語言；呼叫端判斷譯文品質時需要它。
    pub fn target_language(&self) -> String {
        self.binding().target_language.clone()
    }

    /// 收訊：把遊戲聊天（任何語言）翻成目標語言；`context` 為該行之前的原文行。
    pub async fn translate_incoming(
        &self,
        text: &str,
        context: &[String],
    ) -> Result<String, TranslateError> {
        let b = self.binding();
        let glossary = b.glossary_turns("incoming", message_body(text), GLOSSARY_KEEP_ORIGINAL);
        let turns = build_turns(
            context,
            &glossary,
            text,
            CONTEXT_INTRO_INCOMING,
            &b.examples(|e| e.incoming.as_ref()),
        );
        let call = Call {
            kind: "incoming",
            source: text,
            context_lines: context.len(),
            strip: true,
            keep_sender: true,
            ..Call::default()
        };
        b.chat(&build_incoming_system(&b.target_language), &turns, call)
            .await
    }

    /// 系統訊息：把遊戲系統通知翻成目標語言；不吃上下文，同一句原文才會得到同一句譯文。
    ///
    /// 譯文落回英文時以 strict 提示詞重譯一次，救不回的照樣回傳，由呼叫端負責不寫進快取。
    pub async fn translate_system_message(
        &self,
        text: &str,
    ) -> Result<SystemTranslation, TranslateError> {
        let b = self.binding();
        let glossary = b.glossary_turns("system message", text, GLOSSARY_KEEP_ORIGINAL);
        let mut translated = self.system_message_once(&b, text, &glossary, false).await?;
        if has_stray_latin(text, &translated, &b.target_language) {
            log(format!(
                "[translate] system message is not in the target language, retrying strictly: source={} translated={}",
                repr_str(text),
                repr_str(&translated)
            ));
            translated = self.system_message_once(&b, text, &glossary, true).await?;
            if has_stray_latin(text, &translated, &b.target_language) {
                log(format!(
                    "[translate] strict retry is still not in the target language, using it as is: source={} translated={}",
                    repr_str(text),
                    repr_str(&translated)
                ));
            }
        }
        Ok(SystemTranslation {
            text: translated,
            glossary_key: hits_key(&b.hits(text)),
        })
    }

    async fn system_message_once(
        &self,
        b: &Binding,
        text: &str,
        glossary: &[Turn],
        strict: bool,
    ) -> Result<String, TranslateError> {
        let mut turns = b.examples(|e| e.system.as_ref());
        turns.extend_from_slice(glossary);
        turns.push(Turn::user(text));
        let call = Call {
            kind: if strict {
                "system message (strict retry)"
            } else {
                "system message"
            },
            source: text,
            strip: true,
            ..Call::default()
        };
        b.chat(
            &build_system_message_system(&b.target_language, strict),
            &turns,
            call,
        )
        .await
    }

    /// 發話：把玩家輸入（任何語言）翻成 [`OUTGOING_LANGUAGE`]；固定帶 few-shot 防模型脫稿回話。
    pub async fn translate_outgoing(
        &self,
        text: &str,
        context: &[String],
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        let b = self.binding();
        let glossary = b.glossary_turns("outgoing", text, &glossary_outgoing_note());
        let turns = build_turns(
            context,
            &glossary,
            text,
            CONTEXT_INTRO_OUTGOING,
            &fewshot_outgoing_turns(),
        );
        let call = Call {
            kind: "outgoing",
            source: text,
            context_lines: context.len(),
            cancel,
            ..Call::default()
        };
        b.chat(&build_outgoing_system(OUTGOING_LANGUAGE), &turns, call)
            .await
    }

    /// 區域翻譯：OCR 辨識出的畫面文字逐行送出（不加編號）→ 目標語言。
    ///
    /// 行數對得上時逐行比對該行與上下相鄰行來清括號原文，對不上（模型併行）時整段處理；
    /// 空結果是 `BadOutput`，因為空字串在卡片上代表「畫面上沒有文字」。
    pub async fn translate_region_text(
        &self,
        text: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<String, TranslateError> {
        let originals = nonblank_lines(text);
        let b = self.binding();
        let joined = originals.join("\n");
        let mut turns = b.examples(|e| e.region.as_ref());
        turns.extend(b.glossary_turns("region text", &joined, GLOSSARY_KEEP_ORIGINAL));
        turns.push(Turn::user(joined));
        let source = format!(
            "<text {} chars, {} lines>",
            text.chars().count(),
            originals.len()
        );
        let call = Call {
            kind: "region text",
            source: &source,
            max_tokens: Some(MAX_TOKENS_REGION),
            redact: true,
            cancel,
            ..Call::default()
        };
        let translated = b
            .chat(&build_region_system(&b.target_language), &turns, call)
            .await?;
        let lines: Vec<String> = nonblank_lines(&translated)
            .into_iter()
            .filter(|line| !line.starts_with("```"))
            .collect();
        if lines.is_empty() {
            log(format!(
                "[translate] region text came back with no text for {} lines",
                originals.len()
            ));
            return Err(TranslateError::BadOutput("model returned no text".into()));
        }
        if lines.len() != originals.len() {
            log(format!(
                "[translate] region text came back as {} lines for {}; showing it as a whole",
                lines.len(),
                originals.len()
            ));
            return Ok(tidy_parentheses(text, &lines.join("\n")));
        }
        let tidied: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let nearby = &originals[i.saturating_sub(1)..(i + 2).min(originals.len())];
                tidy_parentheses(&nearby.join("\n"), line)
            })
            .collect();
        Ok(tidied.join("\n"))
    }

    /// 請目前的模型把示範改寫成目標語言，回傳（指紋, 範例集）；不套用，由呼叫端 `set_examples`。
    ///
    /// 驗證失敗與 `BadOutput` 會重送，最多 6 次並合併各次通過的路徑；其他錯誤直接回傳。
    pub async fn generate_examples(&self) -> Result<(String, ExampleSet), TranslateError> {
        let b = self.binding();
        let (target, fingerprint) = (&b.target_language, b.fingerprint());
        if is_game_language(target) {
            return Ok((fingerprint, GAME_LANGUAGE_EXAMPLES.clone()));
        }
        let (system, turns) = generation_request(target);
        let started = Instant::now();
        let mut examples = ExampleSet::default();
        let mut attempt = 0;
        while attempt < EXAMPLE_ATTEMPTS && !examples.complete() {
            attempt += 1;
            log(format!(
                "[translate] generating examples (provider={}, model={}, target={target}, attempt={attempt})",
                b.spec.provider,
                b.client.model()
            ));
            let req = ChatRequest {
                system: &system,
                turns: &turns,
                max_tokens: Some(MAX_TOKENS_REGION),
                deterministic: false,
            };
            let output = match b.client.chat(req, None).await {
                Err(TranslateError::BadOutput(message)) => {
                    log(format!(
                        "[translate] example generation output rejected (attempt={attempt}): {message}"
                    ));
                    continue;
                }
                result => result?,
            };
            let (parsed, failures) = parse_generated(&output);
            if !failures.is_empty() {
                let failures: Vec<String> = failures.iter().map(|f| repr_str(f)).collect();
                log(format!(
                    "[translate] example validation failed (attempt={attempt}): [{}]; output={}",
                    failures.join(", "),
                    repr_str(&output)
                ));
            }
            examples = examples.merge(&parsed);
        }
        log(format!(
            "[translate] examples generated in {:.1}s after {attempt} attempt(s): {}",
            started.elapsed().as_secs_f64(),
            examples.summary()
        ));
        Ok((fingerprint, examples))
    }
}

/// 取得端點上可用的模型 ID；用設定表單當下的值，與翻譯走同一條分派。
pub async fn list_models(spec: &ServiceSpec) -> Result<Vec<ModelInfo>, TranslateError> {
    build_client(spec).list_models().await
}

/// 測試連線的結果：譯文與翻譯那一次的耗時。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub translated: String,
    pub elapsed: Duration,
}

type GenerationSlots = Mutex<HashMap<(PathBuf, String), Arc<tokio::sync::Mutex<()>>>>;

/// 每個（快取檔, 指紋）一把鎖：測試連線與背景生成用的是不同的 Translator／ExampleStore，靠它避免重複付費生成。
static GENERATING: LazyLock<GenerationSlots> = LazyLock::new(Mutex::default);

/// 取得目前服務與目標語言的範例並回傳（指紋, 範例集）：快取命中直接回傳，否則生成並存檔（全空不存、回 `None`）。
///
/// 同一快取檔與指紋已在生成中時等它結束再查快取；它失敗就輪到這次生成。
pub async fn generate_and_store(
    tr: &Translator,
    store: &ExampleStore,
) -> Result<Option<(String, ExampleSet)>, TranslateError> {
    let fingerprint = tr.examples_fingerprint();
    // 每個鍵留一個空鎖不清：鍵數只隨本次執行換過的設定成長
    let slot = Arc::clone(
        lock(&GENERATING)
            .entry((store.path().to_path_buf(), fingerprint.clone()))
            .or_default(),
    );
    let _turn = match slot.try_lock() {
        Ok(turn) => turn,
        Err(_) => {
            log(format!(
                "[translate] examples already generating, waiting for it ({}, fingerprint={fingerprint})",
                tr.describe()
            ));
            slot.lock().await
        }
    };
    if tr.examples_fingerprint() != fingerprint {
        log(format!(
            "[translate] example generation skipped, settings changed while waiting ({}, fingerprint={fingerprint})",
            tr.describe()
        ));
        return Ok(None);
    }
    if let Some(cached) = store.get(&fingerprint) {
        log(format!(
            "[translate] examples loaded from cache (fingerprint={fingerprint}): {}",
            cached.summary()
        ));
        return Ok(Some((fingerprint, cached)));
    }
    let (fingerprint, examples) = tr.generate_examples().await?;
    if examples.empty() {
        log(format!(
            "[translate] example generation failed validation on every path ({}, fingerprint={fingerprint})",
            tr.describe()
        ));
        return Ok(None);
    }
    store.put(&fingerprint, &examples);
    Ok(Some((fingerprint, examples)))
}

/// 測試連線：實際翻一句固定文字，與正式翻譯同一條路。
///
/// 給 `store` 時翻譯成功後順便生成並快取範例；生成失敗只記 log，不影響回傳的譯文。
pub async fn test_translate(
    tr: Arc<Translator>,
    store: Option<Arc<ExampleStore>>,
) -> Result<TestOutcome, TranslateError> {
    let started = Instant::now();
    let translated = tr.translate_incoming(TEST_SAMPLE, &[]).await?;
    let elapsed = started.elapsed();
    if let Some(store) = store
        && let Err(err) = generate_and_store(&tr, &store).await
    {
        log(format!(
            "[translate] example generation after the connection test failed: {}: {err}",
            err.kind()
        ));
    }
    Ok(TestOutcome {
        translated,
        elapsed,
    })
}

/// 背景生成沒走到收尾（panic 或 task 被丟掉）時照失敗收尾，指紋才不會永遠停在「生成中」。
struct PendingGuard<'a> {
    coordinator: &'a ExampleCoordinator,
    requested: &'a str,
    armed: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if std::thread::panicking() {
            log(format!(
                "[translate] example generation panicked (fingerprint={})",
                self.requested
            ));
        }
        self.coordinator.finish(self.requested, None);
    }
}

#[derive(Default)]
struct CoordinatorState {
    translators: Vec<Arc<Translator>>,
    pending: HashSet<String>,
    failed: HashSet<String>,
}

/// 決定範例何時走快取、何時背景生成，並把結果交給符合指紋的 translator。
///
/// Translator 本身執行緒安全，背景生成完成後直接套用，不必繞回 UI 執行緒。
pub struct ExampleCoordinator {
    store: Arc<ExampleStore>,
    rt: Handle,
    on_applied: Arc<dyn Fn(&Translator) + Send + Sync>,
    state: Mutex<CoordinatorState>,
}

impl ExampleCoordinator {
    /// 以範例快取、背景生成用的 runtime 與「範例已套用」回呼建立協調器。
    pub fn new(
        store: Arc<ExampleStore>,
        rt: Handle,
        on_applied: Arc<dyn Fn(&Translator) + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            rt,
            on_applied,
            state: Mutex::default(),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CoordinatorState> {
        lock(&self.state)
    }

    /// 讓 translator 取得目前指紋的範例：命中快取就套用，否則背景生成（同指紋只生成一次）。
    ///
    /// 有開始背景生成時回傳它的 handle，呼叫端不必等待。
    pub fn ensure(self: &Arc<Self>, translator: &Arc<Translator>) -> Option<JoinHandle<()>> {
        let fingerprint = translator.examples_fingerprint();
        let cached = {
            let mut state = self.state();
            if !state.translators.iter().any(|t| Arc::ptr_eq(t, translator)) {
                state.translators.push(Arc::clone(translator));
            }
            // 查快取與登記 pending 在同一把鎖內：生成若在兩者之間寫完快取並收尾，會再付費生成一次
            let cached = self.store.get(&fingerprint);
            if cached.is_none() {
                let pending = state.pending.contains(&fingerprint);
                if pending || state.failed.contains(&fingerprint) {
                    log(format!(
                        "[translate] examples unavailable, translating without them ({}, fingerprint={fingerprint}, {})",
                        translator.describe(),
                        if pending { "pending" } else { "failed" }
                    ));
                    return None;
                }
                state.pending.insert(fingerprint.clone());
            }
            cached
        };
        if let Some(cached) = cached {
            self.apply(translator, &cached, &fingerprint);
            log(format!(
                "[translate] examples cache hit ({}, fingerprint={fingerprint}, {})",
                translator.describe(),
                cached.summary()
            ));
            return None;
        }
        log(format!(
            "[translate] generating examples in background ({}, fingerprint={fingerprint})",
            translator.describe()
        ));
        let (this, translator) = (Arc::clone(self), Arc::clone(translator));
        Some(
            self.rt
                .spawn(async move { this.generate(&translator, fingerprint).await }),
        )
    }

    fn apply(&self, translator: &Translator, examples: &ExampleSet, fingerprint: &str) -> bool {
        if !translator.set_examples(Some(examples.clone()), fingerprint) {
            return false;
        }
        (self.on_applied)(translator);
        true
    }

    async fn generate(&self, translator: &Translator, requested: String) {
        let mut guard = PendingGuard {
            coordinator: self,
            requested: &requested,
            armed: true,
        };
        let outcome = match generate_and_store(translator, &self.store).await {
            Err(err) => {
                log(format!(
                    "[translate] example generation failed ({}, fingerprint={requested}): {}: {err}",
                    translator.describe(),
                    err.kind()
                ));
                None
            }
            Ok(outcome) => outcome,
        };
        guard.armed = false;
        self.finish(&requested, outcome);
    }

    fn finish(&self, requested: &str, outcome: Option<(String, ExampleSet)>) {
        let mut state = self.state();
        state.pending.remove(requested);
        let Some((fingerprint, examples)) = outcome else {
            if state
                .translators
                .iter()
                .any(|t| t.examples_fingerprint() == requested)
            {
                state.failed.insert(requested.to_owned());
            } else {
                log(format!(
                    "[translate] example generation abandoned, settings changed (fingerprint={requested})"
                ));
            }
            return;
        };
        let candidates: Vec<Arc<Translator>> = state
            .translators
            .iter()
            .filter(|t| t.examples_fingerprint() == fingerprint)
            .cloned()
            .collect();
        // 回呼可能再呼叫 ensure，放開鎖再套用
        drop(state);
        let applied = candidates
            .iter()
            .filter(|t| self.apply(t, &examples, &fingerprint))
            .count();
        if applied > 0 {
            log(format!(
                "[translate] examples generated and applied to {applied} translator(s) ({})",
                examples.summary()
            ));
        } else {
            log(format!(
                "[translate] examples stored but not applied, no translator matches (fingerprint={fingerprint})"
            ));
        }
    }
}
