//! 測試用的假 `ChatBackend`：依序回覆並記錄每次請求。
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::config::ServiceSpec;
use wizard101_chat_translator::translation::clients::{ChatBackend, ChatRequest};
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::{BoxFuture, Turn};

use super::log_capture::capture_logs;

/// 合格的範例生成回覆（日文，六行齊全）。
pub const JA: &str = "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑\n\
2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。\n\
3. 火猫（Fire Cat）に話しかける\n\
4. 巨像大道（Colossus Boulevard）へ行く\n\
5. オプション\n\
6. そしてあなたは";

pub struct Call {
    pub system: String,
    pub turns: Vec<Turn>,
    pub max_tokens: Option<u32>,
    pub deterministic: bool,
}

pub type Hook = Box<dyn FnMut(usize) + Send>;

/// 假 backend：依序回 `replies`（用完重複最後一個），記錄每次請求；`hook` 在請求途中被呼叫。
pub struct Fake {
    model: String,
    replies: Mutex<VecDeque<Result<String, TranslateError>>>,
    calls: Mutex<Vec<Call>>,
    hook: Mutex<Option<Hook>>,
}

impl Fake {
    pub fn new(model: &str, replies: Vec<Result<String, TranslateError>>) -> Arc<Self> {
        capture_logs();
        Arc::new(Self {
            model: model.into(),
            replies: Mutex::new(replies.into()),
            calls: Mutex::default(),
            hook: Mutex::default(),
        })
    }

    pub fn texts(replies: &[&str]) -> Arc<Self> {
        Self::new("m", replies.iter().map(|r| Ok(r.to_string())).collect())
    }

    pub fn calls(&self) -> std::sync::MutexGuard<'_, Vec<Call>> {
        self.calls.lock().unwrap()
    }

    pub fn on_call(&self, hook: impl FnMut(usize) + Send + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
    }
}

impl ChatBackend for Fake {
    fn chat<'a>(
        &'a self,
        req: ChatRequest<'a>,
        _cancel: Option<&'a CancellationToken>,
    ) -> BoxFuture<'a, Result<String, TranslateError>> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(Call {
                system: req.system.into(),
                turns: req.turns.to_vec(),
                max_tokens: req.max_tokens,
                deterministic: req.deterministic,
            });
            calls.len()
        };
        if let Some(hook) = self.hook.lock().unwrap().as_mut() {
            hook(n);
        }
        let reply = {
            let mut replies = self.replies.lock().unwrap();
            if replies.len() > 1 {
                replies.pop_front().unwrap()
            } else {
                replies[0].clone()
            }
        };
        Box::pin(async move { reply })
    }

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<String>, TranslateError>> {
        Box::pin(async { Ok(vec![]) })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

pub fn spec(model: &str) -> ServiceSpec {
    ServiceSpec {
        provider: "custom".into(),
        base_url: "http://x".into(),
        model: model.into(),
        ..ServiceSpec::default()
    }
}
