//! 由清單位置與檔案資料列組出單一檔案的下載網址。

use super::list::FileEntry;
use super::server::ListLocation;

/// FileType 4 的檔案在伺服器上以 gzip 壓縮存放。
const GZIP_FILE_TYPE: u32 = 4;

/// 該檔下載後是否為 gzip，需解壓才是清單記載的內容。
pub fn is_gzip(entry: &FileEntry) -> bool {
    entry.file_type == GZIP_FILE_TYPE
}

/// 檔案的下載網址：`prefix/src`，gzip 檔加 `.gz`，最後接 `suffix`。
pub fn file_url(loc: &ListLocation, entry: &FileEntry) -> String {
    let gz = if is_gzip(entry) { ".gz" } else { "" };
    format!(
        "{}/{}{gz}{}",
        loc.prefix,
        entry.src.replace('\\', "/"),
        loc.suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc() -> ListLocation {
        ListLocation {
            version: 1,
            size: 0,
            crc: 0,
            url: String::new(),
            prefix: "http://host/pre".into(),
            suffix: "?q=1".into(),
        }
    }

    fn entry(src: &str, file_type: u32) -> FileEntry {
        FileEntry {
            package: String::new(),
            src: src.into(),
            tar: String::new(),
            file_type,
            size: 0,
            crc: 0,
        }
    }

    /// 只有 FileType 4 加 `.gz`，其他型別不加。
    #[test]
    fn file_url_adds_gz_only_for_type_4() {
        assert_eq!(
            file_url(&loc(), &entry("a/b.dat", 4)),
            "http://host/pre/a/b.dat.gz?q=1"
        );
        assert_eq!(
            file_url(&loc(), &entry("a/b.dat", 1)),
            "http://host/pre/a/b.dat?q=1"
        );
        assert!(is_gzip(&entry("x", 4)));
        assert!(!is_gzip(&entry("x", 5)));
    }

    /// `src` 的反斜線換成斜線。
    #[test]
    fn file_url_uses_forward_slashes() {
        assert_eq!(
            file_url(&loc(), &entry(r"Data\GameData\x.wad", 1)),
            "http://host/pre/Data/GameData/x.wad?q=1"
        );
    }
}
