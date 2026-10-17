//! 本機 OCR：模型解出、失敗分類與重試、細長區域，以及（手動）實圖比對。

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use wizard101_chat_translator::region::ocr::{
    Models, OcrError, ensure_extracted, recognize, recognize_with,
};
use wizard101_chat_translator::util::sha1_head;

const FILES: [&str; 4] = [
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_small.onnx",
    "ch_ppocr_mobile_v2.0_cls_mobile.onnx",
    "ppocrv6_keys.txt",
];

fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ocr")
}

fn models() -> &'static Models {
    static MODELS: OnceLock<Models> = OnceLock::new();
    MODELS.get_or_init(|| {
        let read = |name: &str| -> &'static [u8] {
            Box::leak(fs::read(assets().join(name)).unwrap().into_boxed_slice())
        };
        Models {
            det: read(FILES[0]),
            rec: read(FILES[1]),
            cls: read(FILES[2]),
            keys: read(FILES[3]),
        }
    })
}

fn black_png(width: u32, height: u32) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbImage::new(width, height)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[test]
fn extracts_models_once_and_rewrites_on_hash_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let m = models();
    let dir = ensure_extracted(tmp.path(), m).unwrap();
    assert_eq!(
        dir,
        tmp.path().join(sha1_head(&[m.det, m.rec, m.cls, m.keys]))
    );
    for (name, bytes) in FILES.iter().zip([m.det, m.rec, m.cls, m.keys]) {
        assert_eq!(fs::read(dir.join(name)).unwrap(), bytes, "{name}");
    }

    let det = dir.join(FILES[0]);
    let written = fs::metadata(&det).unwrap().modified().unwrap();
    assert_eq!(ensure_extracted(tmp.path(), m).unwrap(), dir);
    assert_eq!(
        fs::metadata(&det).unwrap().modified().unwrap(),
        written,
        "identical content must not be rewritten"
    );

    fs::write(&det, b"corrupt").unwrap();
    ensure_extracted(tmp.path(), m).unwrap();
    assert_eq!(fs::read(&det).unwrap(), m.det);
}

#[test]
fn extraction_failure_is_ocr_unavailable_and_retried() {
    let tmp = tempfile::tempdir().unwrap();
    // 以一般檔案當父目錄：底下建不了資料夾，等同唯讀／磁碟滿的解出失敗。
    let blocked = tmp.path().join("not-a-dir");
    fs::write(&blocked, b"").unwrap();
    let png = black_png(64, 64);

    let mut engine = None;
    let err = recognize_with(&mut engine, Some(models()), &blocked.join("ocr"), &png).unwrap_err();
    assert!(matches!(err, OcrError::Unavailable(_)), "{err:?}");
    assert!(engine.is_none(), "a failed load must not be cached");

    let writable = tmp.path().join("ocr");
    assert_eq!(
        recognize_with(&mut engine, Some(models()), &writable, &png),
        Ok(String::new())
    );
    assert!(engine.is_some());
}

#[test]
fn a_recognition_error_is_a_failure_not_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut engine = None;
    let err = recognize_with(&mut engine, None, tmp.path(), b"not a png").unwrap_err();
    assert!(matches!(err, OcrError::Failed(_)), "{err:?}");
    assert!(engine.is_some(), "a working engine stays cached");
}

#[test]
fn thin_strip_goes_through_vertical_padding_without_panicking() {
    assert_eq!(recognize(&black_png(420, 18)), Ok(String::new()));
}

/// 手動執行：`W101_OCR_SAMPLES` 指向含 `*.png` 與同名 `*.txt`（期望文字）的資料夾。
#[test]
#[ignore]
fn samples_match_expected_text() {
    let dir = PathBuf::from(std::env::var_os("W101_OCR_SAMPLES").expect("set W101_OCR_SAMPLES"));
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let png = entry.unwrap().path();
        if png.extension().is_none_or(|e| e != "png") {
            continue;
        }
        let expected = fs::read_to_string(png.with_extension("txt")).unwrap();
        let got = recognize(&fs::read(&png).unwrap()).unwrap();
        assert_eq!(
            got,
            expected.trim_end_matches(['\r', '\n']),
            "{}",
            png.display()
        );
        checked += 1;
    }
    assert!(checked > 0, "no samples in {}", dir.display());
}
