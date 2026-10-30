//! 本程式啟動的 llama-server 實際佔用的記憶體：專用 GPU 記憶體讀 Windows 效能計數器，RAM 讀程序的 working set（私有部分與整體）。
//! 另外也從同一套計數器讀整張顯示卡目前的專用記憶體用量。

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::logs::log;

/// 一組程序實際佔用的位元組。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub vram: u64,
    pub ram: u64,
}

/// 一次量測；`gpu_read` 為 false 時需要的 GPU 計數器讀不到，那部分當 0。
///
/// `usage.ram` 只算程序私有的 working set；模型檔以記憶體對映讀進來的分頁不算（已搬上 GPU 的權重在
/// Windows 上仍留著對映，記憶體吃緊時系統可直接丟掉）。`resident` 是整個 working set，程序結束時會全部空出來。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub usage: Usage,
    pub resident: u64,
    pub gpu_read: bool,
}

/// `pids` 合計的 VRAM 與 RAM 佔用；讀不到的部分記 log 並當 0。
///
/// 獨顯：專用 GPU 記憶體為 VRAM、working set 為 RAM（共享 GPU 記憶體不加，免得與 working set 重複）。
/// 內顯（`integrated`）：沒有 VRAM，專用＋共享 GPU 記憶體也算 RAM。
pub fn process_usage(pids: &[u32], integrated: bool) -> Reading {
    if pids.is_empty() {
        return Reading {
            gpu_read: true,
            ..Reading::default()
        };
    }
    let (dedicated, shared) = gpu_memory(pids).unwrap_or_else(|e| {
        log(format!("[local] reading GPU process memory failed: {e}"));
        (None, None)
    });
    let ram = pids
        .iter()
        .map(|&pid| working_set(pid))
        .fold(WorkingSet::default(), |a, w| WorkingSet {
            private: a.private + w.private,
            total: a.total + w.total,
        });
    combine(dedicated, shared, ram, integrated)
}

fn combine(
    dedicated: Option<u64>,
    shared: Option<u64>,
    ws: WorkingSet,
    integrated: bool,
) -> Reading {
    let (d, s) = (dedicated.unwrap_or(0), shared.unwrap_or(0));
    if integrated {
        Reading {
            usage: Usage {
                vram: 0,
                ram: ws.private + d + s,
            },
            resident: ws.total + d + s,
            gpu_read: dedicated.is_some() && shared.is_some(),
        }
    } else {
        Reading {
            usage: Usage {
                vram: d,
                ram: ws.private,
            },
            resident: ws.total,
            gpu_read: dedicated.is_some(),
        }
    }
}

/// 工作管理員「專用／共享 GPU 記憶體」同一份計數器，實例名稱是 `pid_{pid}_luid_…`；回傳（專用, 共享），讀不到的為 `None`。
fn gpu_memory(pids: &[u32]) -> Result<(Option<u64>, Option<u64>), String> {
    with_query(|query| read_gpu_memory(query, pids))
}

/// 整張顯示卡目前的專用記憶體用量（所有程序合計，同工作管理員）；`luid` 是 DXGI adapter 的 LUID，
/// 計數器實例名稱是 `luid_0x{高位}_0x{低位}_phys_…`。
pub fn adapter_dedicated_usage(luid: (u32, u32)) -> Result<u64, String> {
    let prefix = format!("luid_0x{:08x}_0x{:08x}_", luid.0, luid.1);
    with_query(|query| {
        let counter = add_counter(query, "\\GPU Adapter Memory(*)\\Dedicated Usage")?;
        collect(query)?;
        sum_for(counter, &[prefix])
    })
}

/// 開一個即時資料的 PDH 查詢交給 `f`，用完關掉。
fn with_query<T>(f: impl FnOnce(PDH_HQUERY) -> Result<T, String>) -> Result<T, String> {
    let mut query: PDH_HQUERY = std::ptr::null_mut();
    // SAFETY: 輸出指標有效；資料來源為 null 表示即時資料
    let status = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut query) };
    if status != 0 {
        return Err(format!("PdhOpenQueryW status=0x{status:08x}"));
    }
    let result = f(query);
    // SAFETY: `query` 由 PdhOpenQueryW 開成功
    unsafe { PdhCloseQuery(query) };
    result
}

fn collect(query: PDH_HQUERY) -> Result<(), String> {
    // SAFETY: `query` 有效；即時計數器收一次就有值
    let status = unsafe { PdhCollectQueryData(query) };
    if status != 0 {
        return Err(format!("PdhCollectQueryData status=0x{status:08x}"));
    }
    Ok(())
}

fn read_gpu_memory(query: PDH_HQUERY, pids: &[u32]) -> Result<(Option<u64>, Option<u64>), String> {
    // 一個計數器失敗不影響另一個：獨顯只需要專用的
    let failed = |e: String| log(format!("[local] reading GPU process memory: {e}"));
    let process = |name: &str| format!("\\GPU Process Memory(*)\\{name}");
    let dedicated = add_counter(query, &process("Dedicated Usage"))
        .map_err(failed)
        .ok();
    let shared = add_counter(query, &process("Shared Usage"))
        .map_err(failed)
        .ok();
    collect(query)?;
    let prefixes: Vec<String> = pids.iter().map(|p| format!("pid_{p}_")).collect();
    let sum = |c: Option<PDH_HCOUNTER>| c.and_then(|c| sum_for(c, &prefixes).map_err(failed).ok());
    Ok((sum(dedicated), sum(shared)))
}

fn add_counter(query: PDH_HQUERY, path: &str) -> Result<PDH_HCOUNTER, String> {
    let wide: Vec<u16> = format!("{path}\0").encode_utf16().collect();
    let mut counter: PDH_HCOUNTER = std::ptr::null_mut();
    // SAFETY: `wide` 以 NUL 結尾，輸出指標有效
    let status = unsafe { PdhAddEnglishCounterW(query, wide.as_ptr(), 0, &mut counter) };
    if status != 0 {
        return Err(format!(
            "PdhAddEnglishCounterW({path}) status=0x{status:08x}"
        ));
    }
    Ok(counter)
}

/// 計數器裡名稱以 `prefixes`（小寫）任一個開頭的實例加總。
fn sum_for(counter: PDH_HCOUNTER, prefixes: &[String]) -> Result<u64, String> {
    let (mut bytes, mut count) = (0u32, 0u32);
    // SAFETY: 先問需要的緩衝區大小，緩衝區指標可為 null
    let status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_LARGE,
            &mut bytes,
            &mut count,
            std::ptr::null_mut(),
        )
    };
    if status != PDH_MORE_DATA {
        return Err(format!(
            "PdhGetFormattedCounterArrayW size status=0x{status:08x}"
        ));
    }
    // 以 u64 對齊配置：項目內含指標與 i64
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
    let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
    // SAFETY: 緩衝區至少 `bytes` 位元組且對齊
    let status = unsafe {
        PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut bytes, &mut count, items)
    };
    if status != 0 {
        return Err(format!(
            "PdhGetFormattedCounterArrayW status=0x{status:08x}"
        ));
    }
    let mut total = 0u64;
    for i in 0..count as usize {
        // SAFETY: PDH 填了 `count` 個項目，名稱字串放在同一塊緩衝區內
        let item = unsafe { &*items.add(i) };
        let name = unsafe { wide_str(item.szName) };
        if instance_matches(&name, prefixes) {
            // SAFETY: 以 PDH_FMT_LARGE 取值，聯合體內是 largeValue
            let value = unsafe { item.FmtValue.Anonymous.largeValue };
            total += u64::try_from(value).unwrap_or(0);
        }
    }
    Ok(total)
}

/// 實例名稱是否以 `prefixes`（小寫）任一個開頭。
///
/// 名稱裡 LUID 的十六進位在程式內讀到是大寫（PowerShell 的 Get-Counter 顯示小寫），所以不分大小寫。
fn instance_matches(name: &str, prefixes: &[String]) -> bool {
    let name = name.to_ascii_lowercase();
    prefixes.iter().any(|p| name.starts_with(p.as_str()))
}

/// 以 NUL 結尾的 UTF-16 字串。
///
/// # Safety
/// `ptr` 須指向以 NUL 結尾的有效字串（或為 null）。
unsafe fn wide_str(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: 呼叫端保證以 NUL 結尾
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: 前 `len` 個元素都在字串內
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// 程序的 working set（位元組）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct WorkingSet {
    /// 程序私有、不能直接丟掉的部分。
    private: u64,
    /// 含對映檔案等可共用分頁的整體。
    total: u64,
}

/// 程序目前佔用的實體記憶體；讀不到回 0。
fn working_set(pid: u32) -> WorkingSet {
    // SAFETY: 只要查詢權限；失敗回 null
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        log(format!(
            "[local] cannot open llama-server pid={pid} to read its memory"
        ));
        return WorkingSet::default();
    }
    // SAFETY: 結構清零並設好 cb，handle 有效；EX2 開頭與 PROCESS_MEMORY_COUNTERS 相同
    let mut counters: PROCESS_MEMORY_COUNTERS_EX2 = unsafe { std::mem::zeroed() };
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            handle,
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    // SAFETY: handle 由 OpenProcess 開成功
    unsafe { CloseHandle(handle) };
    if ok == 0 {
        log(format!(
            "[local] reading memory of llama-server pid={pid} failed"
        ));
        return WorkingSet::default();
    }
    WorkingSet {
        private: counters.PrivateWorkingSetSize as u64,
        total: counters.WorkingSetSize as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 實際讀一次本程序：私有的 working set 大於 0 且不超過整體，GPU 計數器讀得到（沒用 GPU 時為 0）。
    #[test]
    fn reads_the_current_process() {
        let reading = process_usage(&[std::process::id()], false);
        assert!(reading.usage.ram > 0, "{reading:?}");
        assert!(reading.usage.ram <= reading.resident, "{reading:?}");
        assert!(reading.gpu_read, "{reading:?}");
        let (dedicated, shared) = gpu_memory(&[std::process::id()]).unwrap();
        assert!(dedicated.is_some() && shared.is_some());
        assert_eq!(process_usage(&[], false).usage, Usage::default());
    }

    /// LUID 的十六進位大小寫不同也對得上；pid 前綴不會誤配到更長的 pid。
    #[test]
    fn instance_names_match_regardless_of_hex_case() {
        let adapter = vec!["luid_0x00000000_0x00016a0d_".to_string()];
        assert!(instance_matches(
            "luid_0x00000000_0x00016A0D_phys_0",
            &adapter
        ));
        assert!(!instance_matches(
            "luid_0x00000000_0x000195C8_phys_0",
            &adapter
        ));
        let pid = vec!["pid_12_".to_string()];
        assert!(instance_matches("pid_12_luid_0x0_0x16A0D_phys_0", &pid));
        assert!(!instance_matches("pid_123_luid_0x0_0x16A0D_phys_0", &pid));
    }

    /// RAM 只算私有的 working set、`resident` 算整體；獨顯不加共享 GPU 記憶體、只要專用的讀得到；內顯的專用＋共享都算 RAM，兩個都要讀得到。
    #[test]
    fn integrated_gpu_memory_counts_as_ram() {
        let ws = WorkingSet {
            private: 10,
            total: 40,
        };
        let read = |vram, ram, resident, gpu_read| Reading {
            usage: Usage { vram, ram },
            resident,
            gpu_read,
        };
        assert_eq!(combine(Some(3), Some(2), ws, false), read(3, 10, 40, true));
        assert_eq!(combine(Some(3), None, ws, false), read(3, 10, 40, true));
        assert_eq!(combine(None, Some(2), ws, false), read(0, 10, 40, false));
        assert_eq!(combine(Some(3), Some(2), ws, true), read(0, 15, 45, true));
        assert_eq!(combine(Some(3), None, ws, true), read(0, 13, 43, false));
    }
}
