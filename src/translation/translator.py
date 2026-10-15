"""翻譯入口：Translator 依用途（收訊、發話、系統訊息、框選）組提示詞、送出請求、後處理譯文。

收訊翻成使用者設定的目標語言（target_language），發話翻成 OUTGOING_LANGUAGE，
來源語言一律自動判斷；提示詞在 prompts.py，譯文後處理與品質判定在 postprocess.py，
各服務商的 HTTP 協定在 clients.py，失敗的分類在 errors.py。

TranslatorBadOutput（譯文被截斷）在翻譯路徑重試無用、該行應跳過；只有生成範例會重試它。
list_models() 向端點取得模型清單，端點不支援時拋 TranslatorNoModelList。
上下文由呼叫端提供（見 context.py）；Translator 只持有一份不可變的設定快照
（後端、目標語言、few-shot 範例），每次請求只讀一次，平行呼叫仍安全。
"""
import time
from dataclasses import dataclass, replace

from src.logs import log
from src.translation.clients import (
    MAX_TOKENS_REGION,
    TIMEOUT,
    BaseClient,
    RequestHandle,
    build_client,
)
from src.translation.errors import (
    TranslatorBadOutput,
    TranslatorCancelled,
)
from src.translation.examples import (
    EMPTY,
    GAME_LANGUAGE_EXAMPLES,
    ExampleSet,
    ExampleStore,
    examples_fingerprint,
    generation_request,
    parse_generated,
)
from src.translation.postprocess import (
    has_stray_latin,
    nonblank_lines,
    restore_sender,
    sender_of,
    tidy_parentheses,
)
from src.translation.prompts import (
    CONTEXT_INTRO_INCOMING,
    CONTEXT_INTRO_OUTGOING,
    FEWSHOT_OUTGOING,
    OUTGOING_LANGUAGE,
    build_incoming_system,
    build_outgoing_system,
    build_region_system,
    build_system_message_system,
    build_turns,
    example_turns,
    is_game_language,
)

TEST_SAMPLE = "[Tester] Hello! How are you?"  # 測試連線用固定原文
_EXAMPLE_ATTEMPTS = 6


@dataclass(frozen=True)
class _Binding:
    """一份生效中的設定：後端、服務設定、目標語言與範例；換設定就整份換掉，不改欄位。"""

    impl: BaseClient
    api: dict
    target_language: str
    examples: ExampleSet | None = None

    @property
    def fingerprint(self) -> str:
        return examples_fingerprint(self.api, self.target_language)


class Translator:
    """共用翻譯 client：依 provider 選擇後端，收訊/發話介面不變。

    `**api` 是某一家服務商的設定（provider、model、api_key…；每家欄位不同，見
    services.API_PROFILE_FIELDS），呼叫端直接把 services.resolve(cfg, slot) 展開進來，缺的欄位
    由 build_client 補預設值。`timeout`／`client` 供測試注入假 client。

    設定一律整份換掉（見 _Binding）：每個請求開頭讀一次 `self._binding`、之後只用它，
    請求期間的 reconfigure／set_examples 才不會讓提示詞、範例與後端混用兩份設定。
    reconfigure／set_examples 只在 UI 執行緒呼叫（單一寫入者）。"""

    def __init__(self, *, target_language: str, timeout: float = TIMEOUT,
                 client=None, **api):
        self._binding = _Binding(build_client(**api, timeout=timeout, client=client),
                                 api, target_language)

    def reconfigure(self, *, target_language: str, **api) -> bool:
        """設定變更後就地重建後端 client（呼叫端不需換 Translator 實例），回傳是否重建。

        值與目前生效的相同時整個跳過：重建會關掉連線池，飛行中的請求收到 WinSock 斷線、
        被判成「翻譯伺服器離線」（發話與區域沒有重試），端點學到的參數限制
        （見 _OpenAICompatClient）也得重新付一輪 400 才學回來。
        舊 client 一定要關：否則每改一次設定就多留一個連線池。範例屬於舊設定，不帶過去。"""
        old = self._binding
        if api == old.api and target_language == old.target_language:
            return False
        self._binding = _Binding(build_client(**api), api, target_language)
        old.impl.close()
        return True

    def describe(self) -> str:
        """目前生效的服務，失敗 log 用的診斷欄位（`provider=…, model=…`）。
        金鑰絕不列入 —— 這些欄位會寫進 app.log。"""
        b = self._binding
        return f"provider={b.api.get('provider', 'custom')}, model={b.impl.model}"

    def close(self) -> None:
        """釋放後端的連線池（一次性用途如測試連線，用完即關）。"""
        self._binding.impl.close()

    @property
    def examples(self) -> ExampleSet | None:
        """目前套用的 few-shot 範例；尚未設定為 None（不放範例輪）。"""
        return self._binding.examples

    @property
    def examples_fingerprint(self) -> str:
        """目前服務與目標語言對應的範例指紋（見 examples.examples_fingerprint）。"""
        return self._binding.fingerprint

    def set_examples(self, examples: ExampleSet | None, fingerprint: str) -> bool:
        """套用範例；指紋與目前設定不符（生成期間設定被改了）就不套用並回 False。"""
        b = self._binding
        if fingerprint != b.fingerprint:
            log(f"[translate] ignoring examples for a stale fingerprint: {fingerprint}")
            return False
        self._binding = replace(b, examples=examples)
        return True

    @property
    def target_language(self) -> str:
        """目前的目標語言。呼叫端要判斷譯文品質時需要它（見 has_stray_latin）。"""
        return self._binding.target_language

    def _chat(self, impl: BaseClient, kind: str, system: str, turns: list[dict], *,
              source: str, context_lines: int, strip: bool = False,
              keep_sender: bool = False, max_tokens: int | None = None, redact: bool = False,
              cancel: RequestHandle | None = None) -> str:
        """打一次翻譯請求，回傳最終譯文並記錄一行診斷。

        三個方向共用的唯一成功路徑 log 點 —— 使用者匯出 app.log 後，能把每則原文與
        實際譯文並排對照（messages.log 只留原文，不留譯文）。失敗分支不在這裡記錄：
        例外往上拋，由 pool 依重試結果記錄（見 translation.pool）。
        `keep_sender=True` 把譯文的 `[發送者]` 換回原文那一個（見 restore_sender）。
        `redact=True` 只記字數不記內容：區域翻譯的原文可能整頁、譯文可能很長。
        `cancel` 給的話請求可以中途撤銷（見 RequestHandle）；還沒送就被取消的不送。
        """
        if cancel is not None and cancel.cancelled:
            raise TranslatorCancelled()
        started = time.monotonic()
        translated = impl.chat(system, turns, max_tokens=max_tokens, cancel=cancel)
        if strip:
            translated = tidy_parentheses(source, translated)
        if keep_sender:
            restored = restore_sender(source, translated)
            if sender_of(restored) != sender_of(translated):
                log(f"[translate] model altered the sender prefix, restored: "
                    f"source={source!r} model_output={translated!r}")
            translated = restored
        shown = f"<{len(translated)} chars>" if redact else repr(translated)
        log(f"[translate] {kind} done in {time.monotonic() - started:.1f}s "
            f"(model={impl.model}, ctx={context_lines}): "
            f"source={source!r} translated={shown}")
        return translated

    def translate_incoming(self, text: str, context: list[str]) -> str:
        """收訊：把遊戲聊天（任何語言）翻成使用者設定的目標語言。
        context 為該行之前的原文行，由呼叫端依讀取順序維護（見 ChatContext）。"""
        b = self._binding
        return self._chat(
            b.impl, "incoming",
            build_incoming_system(b.target_language),
            build_turns(context, text, CONTEXT_INTRO_INCOMING,
                        examples=example_turns(b.examples.incoming if b.examples else None)),
            source=text, context_lines=len(context), strip=True, keep_sender=True)

    def translate_system_message(self, text: str) -> str:
        """系統訊息：把遊戲系統通知（任何語言）翻成使用者設定的目標語言。

        刻意不吃 context：系統訊息彼此獨立，8 行的上下文窗會被掉寶洗光、玩家對話失去
        語境；純函式化的呼叫也是譯文快取正確性的前提（見 translation.cache）。

        譯文落回英文時重譯一次（見 has_stray_latin）：裸名詞特別容易被改用官方英文名，
        實測重譯救得回約三分之一，救不回的（音譯玩家名）照樣回傳，呼叫端負責不寫進快取。
        只有這條路徑重譯：收訊有完整句子語境、實測不會落回英文。"""
        b = self._binding
        translated = self._system_message_once(b, text)
        if has_stray_latin(text, translated, b.target_language):
            log(f"[translate] system message is not in the target language, retrying "
                f"strictly: source={text!r} translated={translated!r}")
            translated = self._system_message_once(b, text, strict=True)
            if has_stray_latin(text, translated, b.target_language):
                log(f"[translate] strict retry is still not in the target language, "
                    f"using it as is: source={text!r} translated={translated!r}")
        return translated

    def _system_message_once(self, b: _Binding, text: str, strict: bool = False) -> str:
        return self._chat(
            b.impl, "system message (strict retry)" if strict else "system message",
            build_system_message_system(b.target_language, strict=strict),
            [*example_turns(b.examples.system if b.examples else None),
             {"role": "user", "content": text}],
            source=text, context_lines=0, strip=True)

    def translate_outgoing(self, text: str, context: list[str],
                           cancel: RequestHandle | None = None) -> str:
        """發話：把玩家輸入（任何語言）翻成遊戲聊天語言（固定）。
        輸入框在等譯文時被關掉會經 `cancel` 撤銷請求，不白白付 token。
        發話內容不寫入上下文 —— 送出後遊戲會回顯成聊天行，由收訊路徑記錄。
        few-shot 一律帶：曾只在無上下文時帶，但遊戲內幾乎永遠有上下文，實測模型會把
        「不好意思我英文不好，用翻譯器」當成對它說的話回「No worries, I'll help you out!」，
        而該回覆會被原樣送進遊戲聊天。"""
        return self._chat(
            self._binding.impl, "outgoing",
            build_outgoing_system(OUTGOING_LANGUAGE),
            build_turns(context, text, CONTEXT_INTRO_OUTGOING,
                        examples=FEWSHOT_OUTGOING),
            source=text, context_lines=len(context), cancel=cancel)

    def translate_region_text(self, text: str, cancel: RequestHandle | None = None) -> str:
        """區域翻譯：本機 OCR 辨識出的畫面文字 → 目標語言；`cancel` 撤銷還在跑的請求。

        原文逐行送出、不加編號，模型把折行的句子併成一段時整段顯示。括號原文（`tidy_parentheses`）
        行數對得上時逐行比對該行與上下相鄰行，對不上時比對整段；提示詞仍要求照抄同一行，刻意比判定嚴。"""
        originals = nonblank_lines(text)
        b = self._binding
        translated = self._chat(
            b.impl, "region text",
            build_region_system(b.target_language),
            [*example_turns(b.examples.region if b.examples else None),
             {"role": "user", "content": "\n".join(originals)}],
            source=f"<text {len(text)} chars, {len(originals)} lines>", context_lines=0,
            max_tokens=MAX_TOKENS_REGION, redact=True, cancel=cancel)
        lines = [line for line in nonblank_lines(translated) if not line.startswith("```")]
        if not lines:
            # 空字串在卡片上代表「畫面上沒有文字」，模型沒回東西得當成失敗
            log(f"[translate] region text came back with no text for {len(originals)} lines")
            raise TranslatorBadOutput("model returned no text")
        if len(lines) != len(originals):
            log(f"[translate] region text came back as {len(lines)} lines for "
                f"{len(originals)}; showing it as a whole")
            return tidy_parentheses(text, "\n".join(lines))
        return "\n".join(tidy_parentheses("\n".join(originals[max(i - 1, 0):i + 2]), line)
                         for i, line in enumerate(lines))

    def generate_examples(self) -> tuple[str, ExampleSet]:
        """請目前的模型把示範改寫成目標語言，回傳（指紋, 範例集）；不套用，由呼叫端 set_examples。

        驗證失敗與截斷會重送，最多送 _EXAMPLE_ATTEMPTS 次請求並合併各次通過的路徑；其他例外往上拋。"""
        b = self._binding
        impl, api, target, fingerprint = b.impl, b.api, b.target_language, b.fingerprint
        if is_game_language(target):
            return fingerprint, GAME_LANGUAGE_EXAMPLES
        system, turns = generation_request(target)
        started = time.monotonic()
        examples = EMPTY
        attempt = 0
        while attempt < _EXAMPLE_ATTEMPTS and not examples.complete:
            attempt += 1
            log(f"[translate] generating examples (provider={api.get('provider', 'custom')}, "
                f"model={impl.model}, target={target}, attempt={attempt})")
            try:
                output = impl.chat(system, turns, max_tokens=MAX_TOKENS_REGION,
                                   deterministic=False)
            except TranslatorBadOutput as exc:
                log(f"[translate] example generation output rejected (attempt={attempt}): {exc}")
                continue
            parsed, failures = parse_generated(output)
            if failures:
                log(f"[translate] example validation failed (attempt={attempt}): "
                    f"{failures}; output={output!r}")
            examples = examples.merge(parsed)
        log(f"[translate] examples generated in {time.monotonic() - started:.1f}s "
            f"after {attempt} attempt(s): {examples.summary()}")
        return fingerprint, examples


def generate_and_store(api: dict, target_language: str, store: ExampleStore) -> ExampleSet | None:
    """取得 api＋目標語言的範例：快取命中直接回傳，否則生成並存檔（全空不存、回 None）。"""
    fingerprint = examples_fingerprint(api, target_language)
    cached = store.get(fingerprint)
    if cached is not None:
        log(f"[translate] examples loaded from cache: {cached.summary()}")
        return cached
    translator = Translator(**api, target_language=target_language)
    try:
        _, examples = translator.generate_examples()
    finally:
        translator.close()
    if examples.empty:
        return None
    store.put(fingerprint, examples)
    return examples


def list_models(api: dict, client=None) -> list[str]:
    """取得端點上可用的模型 ID（已排序）。api 為設定表單當下的值，與翻譯走同一條分派。
    端點不提供清單時拋 TranslatorNoModelList —— 呼叫端應提示改為自行輸入模型名稱。"""
    impl = build_client(**api, timeout=TIMEOUT, client=client)
    try:
        return impl.list_models()
    finally:
        impl.close()


def test_translate(api: dict, target_language: str,
                   example_store: ExampleStore | None = None) -> str:
    """測試連線：用表單當下的 api 設定實際翻一句固定文字，與正式翻譯同一條路。

    給 example_store 時翻譯成功後順便生成並快取範例；生成失敗只記 log，不影響回傳的譯文。"""
    translator = Translator(**api, target_language=target_language)
    try:
        translated = translator.translate_incoming(TEST_SAMPLE, [])
    finally:
        translator.close()
    if example_store is not None:
        try:
            generate_and_store(api, target_language, example_store)
        except Exception as exc:
            log(f"[translate] example generation after the connection test failed: "
                f"{type(exc).__name__}: {exc}")
    return translated
