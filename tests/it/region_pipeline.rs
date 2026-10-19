//! 辨識加翻譯的流程：OCR 與翻譯 backend 都用替身；另驗證 OCR 不佔用 async worker、錯誤分類與文案。

use crate::common;

use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};

use common::fake_backend::Fake;
use common::log_capture::log_lines_with;
use common::reader_fixture::{EXPECTED_PANIC, quiet_panics};
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::region::ocr::{OcrError, OcrUnavailable};
use wizard101_chat_translator::region::pipeline::{
    Recognize, RegionError, RegionPipeline, RegionResult, describe_error,
};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::translator::Translator;

const RECT: (i32, i32, i32, i32) = (10, 20, 300, 100);

fn translator(fake: &Arc<Fake>) -> Arc<Translator> {
    let fake = Arc::clone(fake);
    Translator::with_client(
        move |_| fake.clone(),
        common::fake_backend::spec("m"),
        "繁體中文（台灣）".into(),
    )
}

fn pipeline(fake: &Arc<Fake>, text: &'static str) -> RegionPipeline {
    RegionPipeline::new(translator(fake), Arc::new(move |_| Ok(text.to_owned())))
}

#[tokio::test]
async fn run_returns_translation_and_source() {
    let fake = Fake::texts(&["譯文"]);
    let result = pipeline(&fake, "Hello")
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await;
    assert_eq!(
        result,
        Ok(RegionResult {
            text: "譯文".into(),
            source: "Hello".into()
        })
    );
    assert_eq!(fake.calls().len(), 1);
    let logs = log_lines_with("[region] done in ");
    assert!(
        logs.iter()
            .any(|l| l.ends_with("(rect=(10, 20, 300, 100), source_chars=5, chars=2)")),
        "{logs:?}"
    );
}

#[tokio::test]
async fn no_text_skips_translation() {
    let fake = Fake::texts(&["unused"]);
    let result = pipeline(&fake, "")
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await;
    assert_eq!(result, Ok(RegionResult::default()));
    assert!(fake.calls().is_empty());
    assert!(
        log_lines_with("[region] local OCR found no text (rect=(10, 20, 300, 100))").len() == 1
    );
}

#[tokio::test]
async fn cancel_is_passed_to_the_translator() {
    let fake = Fake::texts(&["譯文"]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = pipeline(&fake, "Hello")
        .run(b"png".to_vec(), RECT, &cancel)
        .await;
    assert_eq!(
        result,
        Err(RegionError::Translate(TranslateError::Cancelled))
    );
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn errors_propagate() {
    let config = TranslateError::Config {
        status: Some(401),
        detail: "bad key".into(),
    };
    let fake = Fake::new("m", vec![Err(config.clone())]);
    let result = pipeline(&fake, "Hello")
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await;
    assert_eq!(result, Err(RegionError::Translate(config)));

    let fake = Fake::texts(&["unused"]);
    let unavailable = RegionPipeline::new(
        translator(&fake),
        Arc::new(|_| Err(OcrError::Unavailable(OcrUnavailable("no models".into())))),
    );
    let result = unavailable
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await;
    assert_eq!(
        result,
        Err(RegionError::Ocr(OcrUnavailable("no models".into())))
    );
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn translator_offline_error_propagates() {
    let offline = TranslateError::offline("down");
    let fake = Fake::new("m", vec![Err(offline.clone())]);
    let result = pipeline(&fake, "Hello")
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await;
    assert_eq!(result, Err(RegionError::Translate(offline)));
}

#[tokio::test]
async fn recognition_errors_and_panics_are_unexpected() {
    quiet_panics();
    let fake = Fake::texts(&["unused"]);
    let run = |recognize: Recognize| {
        let pipeline = RegionPipeline::new(translator(&fake), recognize);
        async move {
            pipeline
                .run(b"png".to_vec(), RECT, &CancellationToken::new())
                .await
        }
    };
    assert_eq!(
        run(Arc::new(|_| Err(OcrError::Failed("detect boom".into())))).await,
        Err(RegionError::Unexpected("detect boom".into()))
    );
    let message = format!("{EXPECTED_PANIC} recognize exploded");
    let panicking = message.clone();
    assert_eq!(
        run(Arc::new(move |_| panic!("{panicking}"))).await,
        Err(RegionError::Unexpected(message))
    );
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn recognize_runs_off_the_async_worker() {
    let seen: Arc<Mutex<Option<ThreadId>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let fake = Fake::texts(&["譯文"]);
    let pipeline = RegionPipeline::new(
        translator(&fake),
        Arc::new(move |_| {
            *recorder.lock().unwrap() = Some(thread::current().id());
            Ok("Hello".into())
        }),
    );
    pipeline
        .run(b"png".to_vec(), RECT, &CancellationToken::new())
        .await
        .unwrap();
    let recognized_on = seen.lock().unwrap().expect("recognize was not called");
    assert_ne!(recognized_on, thread::current().id());
}

#[test]
fn describe_names_the_translation_service() {
    let fake = Fake::texts(&["x"]);
    let translator = translator(&fake);
    let expected = translator.describe();
    let pipeline = RegionPipeline::new(translator, Arc::new(|_| Ok(String::new())));
    assert_eq!(pipeline.describe(), expected);
}

#[test]
fn describe_error_maps_each_failure_kind() {
    let translate = |e| describe_error(&RegionError::Translate(e));
    assert_eq!(
        translate(TranslateError::Config {
            status: Some(401),
            detail: "bad key".into()
        }),
        t(
            "error.api_response",
            &[("status", "401"), ("message", "bad key")]
        )
    );
    assert_eq!(
        translate(TranslateError::offline("refused")),
        t("error.offline_detail", &[("message", "refused")])
    );
    assert_eq!(
        describe_error(&RegionError::Ocr(OcrUnavailable("x".into()))),
        t("region.ocr_unavailable", &[])
    );
    assert_eq!(
        translate(TranslateError::BadOutput("truncated".into())),
        t("region.failed", &[("error", "truncated")])
    );
    assert_eq!(
        translate(TranslateError::Unexpected("boom".into())),
        t("error.unexpected", &[("error", "boom")])
    );
    assert_eq!(
        describe_error(&RegionError::Unexpected("detect boom".into())),
        t("error.unexpected", &[("error", "detect boom")])
    );
}
