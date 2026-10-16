//! 翻譯：提示詞、上下文、譯文後處理與各服務商的 client。

pub mod cache;
pub mod clients;
pub mod context;
pub mod errors;
pub mod examples;
pub mod gate;
pub mod pool;
pub mod postprocess;
pub mod prompts;
pub mod pyrepr;
pub(crate) mod pystr;
pub mod translator;

use std::future::Future;
use std::pin::Pin;

/// 可跨執行緒傳遞的 boxed future，供 trait 物件回傳非同步結果。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 對話輪的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    /// 對話輪角色在 API 中的名稱。
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// 送給模型的一輪對話。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub role: Role,
    pub content: String,
}

impl Turn {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}
