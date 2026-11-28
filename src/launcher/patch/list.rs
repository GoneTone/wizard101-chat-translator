//! 解析 `LatestFileList.bin`，並挑出本次要檢查的檔案。

use std::path::Path;

use crate::launcher::install::InstallSource;
use crate::logs::log;

/// 清單裡一個檔案的資料列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub package: String,
    pub src: String,
    pub tar: String,
    pub file_type: u32,
    pub size: u32,
    pub crc: u32,
}

impl FileEntry {
    /// 本機存放路徑：`tar` 非空用 `tar`，否則 `src`。
    pub fn local_path(&self) -> &str {
        if self.tar.is_empty() {
            &self.src
        } else {
            &self.tar
        }
    }
}

const STR: u8 = 9;
const UINT: u8 = 3;
/// 結構裡指向表名的偽欄位，資料列不含它。
const TARGET_TABLE: &str = "_TargetTable";

struct Field {
    name: String,
    ty: u8,
}

struct Schema {
    table: String,
    fields: Vec<Field>,
}

/// 資料列要用到的欄位位置；缺任一欄（如 `About`）的表格整張略過。
struct Columns {
    src: usize,
    tar: usize,
    file_type: usize,
    size: usize,
    crc: usize,
}

impl Schema {
    fn column(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }

    /// 找出資料列要用的欄位；缺任一欄回 `None`，欄位型別不符回錯誤。
    fn columns(&self) -> Result<Option<Columns>, String> {
        let typed = |name: &str, ty: u8| -> Result<Option<usize>, String> {
            match self.column(name) {
                Some(i) if self.fields[i].ty != ty => Err(format!(
                    "column {name} in {} has type {} (expected {ty})",
                    self.table, self.fields[i].ty
                )),
                found => Ok(found),
            }
        };
        let (src, tar) = (typed("SrcFileName", STR)?, typed("TarFileName", STR)?);
        let (file_type, size) = (typed("FileType", UINT)?, typed("Size", UINT)?);
        let crc = typed("CRC", UINT)?;
        Ok(match (src, tar, file_type, size, crc) {
            (Some(src), Some(tar), Some(file_type), Some(size), Some(crc)) => Some(Columns {
                src,
                tar,
                file_type,
                size,
                crc,
            }),
            _ => None,
        })
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or_else(|| format!("truncated data at offset {}", self.pos))?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// u16 長度＋latin1 字串（每個位元組對應一個字元）。
    fn string(&mut self) -> Result<String, String> {
        let len = self.u16()? as usize;
        Ok(self.take(len)?.iter().map(|&b| b as char).collect())
    }
}

enum Value {
    Str(String),
    Uint(u32),
}

impl Value {
    fn into_string(self) -> String {
        match self {
            Value::Str(s) => s,
            Value::Uint(_) => String::new(),
        }
    }

    fn uint(&self) -> u32 {
        match self {
            Value::Uint(n) => *n,
            Value::Str(_) => 0,
        }
    }
}

/// 結構記錄：一串「名稱＋型別＋旗標」，最後一個名稱後面沒有型別，就是表名。
fn parse_schema(c: &mut Cursor, end: usize) -> Result<Schema, String> {
    let mut fields = Vec::new();
    while c.pos < end {
        let name = c.string()?;
        if c.pos == end {
            return Ok(Schema {
                table: name,
                fields,
            });
        }
        let ty = c.u8()?;
        c.u8()?;
        if ty != STR && ty != UINT {
            return Err(format!("unknown field type {ty} for {name}"));
        }
        if name != TARGET_TABLE {
            fields.push(Field { name, ty });
        }
    }
    Err("schema without table name".into())
}

fn parse_row(c: &mut Cursor, end: usize, schema: &Schema) -> Result<Vec<Value>, String> {
    let mut row = Vec::with_capacity(schema.fields.len());
    for f in &schema.fields {
        row.push(match f.ty {
            STR => Value::Str(c.string()?),
            _ => Value::Uint(c.u32()?),
        });
    }
    if c.pos != end {
        return Err(format!(
            "row in {} does not end at record end",
            schema.table
        ));
    }
    Ok(row)
}

/// 伺服器給的路徑是否安全：非空、相對、無磁碟機／UNC 前綴、無 `..` 區段。
fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && path.split(['/', '\\']).all(|part| part != "..")
}

/// 檢查一列的套件名與路徑；任一不安全回錯誤。
fn check_entry(e: &FileEntry) -> Result<(), String> {
    let package_ok = is_safe_relative(&e.package) && !e.package.contains(['/', '\\']);
    if !package_ok || !is_safe_relative(&e.src) || (!e.tar.is_empty() && !is_safe_relative(&e.tar))
    {
        return Err(format!(
            "unsafe path in file list: package={:?} src={:?} tar={:?}",
            e.package, e.src, e.tar
        ));
    }
    Ok(())
}

/// 解析清單；只回有 `CRC` 欄位的表格的列，順序同檔案。
pub fn parse_file_list(bytes: &[u8]) -> Result<Vec<FileEntry>, String> {
    let mut c = Cursor { bytes, pos: 0 };
    c.u32()?;
    let mut current: Option<(Schema, Option<Columns>)> = None;
    let mut out = Vec::new();
    while c.pos < bytes.len() {
        let kind = c.take(2)?;
        if kind != [2, 1] && kind != [2, 2] {
            c.take(2)?;
            continue;
        }
        let start = c.pos - 2;
        let end = start
            .checked_add(c.u16()? as usize)
            .filter(|&e| e <= bytes.len() && e >= c.pos)
            .ok_or_else(|| format!("record at offset {start} runs past the end"))?;
        if kind == [2, 1] {
            let schema = parse_schema(&mut c, end)?;
            let cols = schema.columns()?;
            current = Some((schema, cols));
        } else {
            let (schema, cols) = current.as_ref().ok_or("data row before any schema")?;
            let row = parse_row(&mut c, end, schema)?;
            if let Some(cols) = cols {
                let mut row: Vec<Option<Value>> = row.into_iter().map(Some).collect();
                let mut take = |i: usize| row[i].take().unwrap_or(Value::Uint(0));
                let entry = FileEntry {
                    package: schema.table.clone(),
                    src: take(cols.src).into_string(),
                    tar: take(cols.tar).into_string(),
                    file_type: take(cols.file_type).uint(),
                    size: take(cols.size).uint(),
                    crc: take(cols.crc).uint(),
                };
                check_entry(&entry)?;
                out.push(entry);
            }
        }
        if c.pos != end {
            return Err(format!("record at offset {start} has trailing bytes"));
        }
    }
    Ok(out)
}

/// Steam 安裝裡由 Steam 管理、不跟清單比對的檔案。
const STEAM_MANAGED: [&str; 2] = ["Bin/PatchConfig.xml", "Bin/Wizard101Installer.vdf"];

/// 官網版要檢查 `Base` 與本機套件，Steam 版全部；兩者都排除官方啟動器自己（`PatchClient`）。
pub fn select<'a>(
    entries: &'a [FileEntry],
    source: InstallSource,
    local_packages: &[String],
) -> Vec<&'a FileEntry> {
    entries
        .iter()
        .filter(|e| e.package != "PatchClient")
        // Steam 管理這兩個檔，內容可與清單不同；官方 Steam 啟動器也不檢查
        .filter(|e| {
            source != InstallSource::Steam
                || !STEAM_MANAGED
                    .iter()
                    .any(|p| e.local_path().eq_ignore_ascii_case(p))
        })
        .filter(|e| {
            source == InstallSource::Steam
                || e.package == "Base"
                || local_packages.contains(&e.package)
        })
        .collect()
}

/// 讀安裝目錄的 `LocalPackagesList.txt`；檔案不存在回空。
pub fn read_local_packages(root: &Path) -> Vec<String> {
    let text = match std::fs::read(root.join("LocalPackagesList.txt")) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log(format!("[patch] read LocalPackagesList.txt failed: {e}"));
            }
            return Vec::new();
        }
    };
    String::from_utf8_lossy(&text)
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn s(out: &mut Vec<u8>, v: &str) {
        out.extend((v.len() as u16).to_le_bytes());
        out.extend(v.bytes());
    }

    fn record(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut r = vec![2, kind];
        r.extend((body.len() as u16 + 4).to_le_bytes());
        r.extend(body);
        r
    }

    pub(crate) fn schema(table: &str, fields: &[(&str, u8)]) -> Vec<u8> {
        let mut b = Vec::new();
        for (n, t) in fields {
            s(&mut b, n);
            b.extend([*t, 0x28]);
        }
        s(&mut b, TARGET_TABLE);
        b.extend([STR, 0x28]);
        s(&mut b, table);
        record(1, &b)
    }

    pub(crate) const PKG_FIELDS: [(&str, u8); 8] = [
        ("SrcFileName", STR),
        ("TarFileName", STR),
        ("FileType", UINT),
        ("Size", UINT),
        ("HeaderSize", UINT),
        ("CompressedHeaderSize", UINT),
        ("CRC", UINT),
        ("HeaderCRC", UINT),
    ];

    pub(crate) fn row(src: &str, tar: &str, ft: u32, size: u32, crc: u32) -> Vec<u8> {
        let mut b = Vec::new();
        s(&mut b, src);
        s(&mut b, tar);
        for n in [ft, size, 0, 0, crc, 0] {
            b.extend(n.to_le_bytes());
        }
        record(2, &b)
    }

    fn sample() -> Vec<u8> {
        let mut f = 3u32.to_le_bytes().to_vec();
        f.extend(schema("Base", &PKG_FIELDS));
        f.extend([1, 0, 0, 0]);
        f.extend(row("Bin/a.dll", "", 3, 10, 0xAABB_CCDD));
        f.extend(row("Data/x", "Data/y", 4, 20, 7));
        f.extend([1, 0, 0, 0]);
        f.extend(schema("About", &[("Version", UINT)]));
        f.extend(record(2, &1u32.to_le_bytes()));
        f
    }

    fn entry(package: &str) -> FileEntry {
        FileEntry {
            package: package.into(),
            src: "f".into(),
            tar: String::new(),
            file_type: 3,
            size: 1,
            crc: 1,
        }
    }

    #[test]
    fn parses_package_rows_with_schema() {
        let got = parse_file_list(&sample()).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            FileEntry {
                package: "Base".into(),
                src: "Bin/a.dll".into(),
                tar: String::new(),
                file_type: 3,
                size: 10,
                crc: 0xAABB_CCDD,
            }
        );
        assert_eq!(got[0].local_path(), "Bin/a.dll");
        assert_eq!(got[1].local_path(), "Data/y");
        assert!(got.iter().all(|e| e.package != "About"));
    }

    fn list_with(package: &str, src: &str, tar: &str) -> Vec<u8> {
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(schema(package, &PKG_FIELDS));
        f.extend([1, 0, 0, 0]);
        f.extend(row(src, tar, 3, 1, 1));
        f
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        for (pkg, src, tar) in [
            ("", "a", ""),
            ("A/B", "a", ""),
            ("A\\B", "a", ""),
            ("..", "a", ""),
            ("P", "", ""),
            ("P", "/abs", ""),
            ("P", "\\\\srv\\share", ""),
            ("P", "C:/x", ""),
            ("P", "C:x", ""),
            ("P", "a/../b", ""),
            ("P", "a\\..\\b", ""),
            ("P", "a", "../b"),
            ("P", "a", "/abs"),
            ("P", "a", "D:\\x"),
        ] {
            let err = parse_file_list(&list_with(pkg, src, tar)).unwrap_err();
            assert!(err.contains("unsafe path"), "{pkg} {src} {tar}: {err}");
        }
    }

    #[test]
    fn normal_nested_path_is_accepted() {
        let got = parse_file_list(&list_with("Base", "Data/GameData/Foo.wad", "")).unwrap();
        assert_eq!(got[0].src, "Data/GameData/Foo.wad");
    }

    #[test]
    fn unknown_field_type_is_an_error() {
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(schema("Base", &[("X", 7)]));
        let err = parse_file_list(&f).unwrap_err();
        assert!(err.contains("unknown field type"), "{err}");
    }

    #[test]
    fn truncated_record_is_an_error() {
        let mut f = sample();
        f.truncate(f.len() - 3);
        let err = parse_file_list(&f).unwrap_err();
        assert!(err.contains("runs past the end"), "{err}");
    }

    #[test]
    fn data_row_before_any_schema_is_an_error() {
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(record(2, &1u32.to_le_bytes()));
        let err = parse_file_list(&f).unwrap_err();
        assert!(err.contains("before any schema"), "{err}");
    }

    #[test]
    fn row_not_ending_at_record_end_is_an_error() {
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(schema("About", &[("Version", UINT)]));
        f.extend(record(2, &[1, 0, 0, 0, 9]));
        let err = parse_file_list(&f).unwrap_err();
        assert!(err.contains("does not end at record end"), "{err}");
    }

    #[test]
    fn mistyped_column_is_an_error() {
        let mut fields = PKG_FIELDS;
        fields[6] = ("CRC", STR);
        let mut f = 1u32.to_le_bytes().to_vec();
        f.extend(schema("Base", &fields));
        let err = parse_file_list(&f).unwrap_err();
        assert!(err.contains("column CRC"), "{err}");
    }

    #[test]
    fn select_official_uses_base_and_local_packages() {
        let all = [
            entry("Base"),
            entry("Extra"),
            entry("Other"),
            entry("PatchClient"),
        ];
        let local = ["Extra".to_string(), "PatchClient".to_string()];
        let got = select(&all, InstallSource::Official, &local);
        let pk: Vec<_> = got.iter().map(|e| e.package.as_str()).collect();
        assert_eq!(pk, ["Base", "Extra"]);
    }

    #[test]
    fn select_steam_takes_all_but_patch_client() {
        let all = [entry("Base"), entry("Other"), entry("PatchClient")];
        let got = select(&all, InstallSource::Steam, &[]);
        assert_eq!(got.len(), 2);
    }

    /// Steam 管理的設定檔與安裝描述檔不比對，官方 Steam 啟動器也不檢查。
    #[test]
    fn select_steam_skips_steam_managed_files() {
        let config = FileEntry {
            src: "Steam/Bin/PatchConfig.xml".into(),
            tar: "Bin/PatchConfig.xml".into(),
            ..entry("Base")
        };
        let vdf = FileEntry {
            src: "Bin/Wizard101Installer.vdf".into(),
            ..entry("Base")
        };
        let all = [entry("Base"), config, vdf];
        assert_eq!(select(&all, InstallSource::Steam, &[]).len(), 1);
        assert_eq!(select(&all, InstallSource::Official, &[]).len(), 3);
    }

    #[test]
    fn local_packages_strip_cr_and_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_local_packages(dir.path()).is_empty());
        std::fs::write(dir.path().join("LocalPackagesList.txt"), "A\r\n\r\nB\n").unwrap();
        assert_eq!(read_local_packages(dir.path()), ["A", "B"]);
    }

    /// 對真實清單核對數量；用 `W101_FILE_LIST` 指定檔案，不進 CI。
    #[test]
    #[ignore]
    fn real_list_counts() {
        let path = std::env::var("W101_FILE_LIST").expect("W101_FILE_LIST");
        let entries = parse_file_list(&std::fs::read(path).unwrap()).unwrap();
        let count = |t| entries.iter().filter(|e| e.file_type == t).count();
        println!(
            "rows={} types 3:{} 1:{} 4:{} 5:{}",
            entries.len(),
            count(3),
            count(1),
            count(4),
            count(5)
        );
        assert_eq!(
            [count(3), count(1), count(4), count(5)],
            [3695, 154, 84, 40]
        );
        let pk: std::collections::HashSet<_> = select(&entries, InstallSource::Steam, &[])
            .into_iter()
            .map(|e| e.package.as_str())
            .collect();
        println!("steam packages={}", pk.len());
        assert_eq!(pk.len(), 3735);
    }
}
