//! 與 Python 版逐字一致所需的小工具（`repr(str)`、`str` 的空白與換行語意），各模組共用。

pub mod pyrepr;
pub(crate) mod pystr;
