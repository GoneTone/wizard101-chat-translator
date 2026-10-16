//! 執行期基礎設施：單一實例、權限檢查、內嵌資源。

pub mod resources;
pub mod single_instance;

pub use single_instance::{
    InstanceLock, acquire_single_instance, focus_running_instance, is_elevated,
};
