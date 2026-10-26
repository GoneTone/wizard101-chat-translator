//! llama-server 啟動參數與就緒輪詢（假 Process、wiremock，不啟動真程序）。

use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::hf::hf_client;
use wizard101_chat_translator::local::server::{Mtp, Process, ServerPlan, args, wait_ready};

struct Fake {
    exit: Option<i32>,
    killed: bool,
}

impl Process for Fake {
    fn exited(&mut self) -> Option<i32> {
        self.exit
    }
    fn kill(&mut self) {
        self.killed = true;
    }
    fn tail(&self) -> Vec<String> {
        vec!["error: unknown model architecture".into()]
    }
}

fn plan(mtp: Mtp, idle_minutes: u32) -> ServerPlan {
    ServerPlan {
        model_id: "m".into(),
        exe: "x".into(),
        model: r"C:\m\a.gguf".into(),
        mtp,
        idle_minutes,
    }
}

#[test]
fn args_with_draft_mtp_and_idle_sleep() {
    let p = plan(Mtp::Draft(r"C:\m\mtp.gguf".into()), 15);
    assert_eq!(
        args(&p, 4321),
        [
            "-m",
            r"C:\m\a.gguf",
            "--host",
            "127.0.0.1",
            "--port",
            "4321",
            "-c",
            "8192",
            "--parallel",
            "1",
            "-lv",
            "4",
            "--sleep-idle-seconds",
            "900",
            "--spec-type",
            "draft-mtp",
            "--spec-draft-model",
            r"C:\m\mtp.gguf"
        ]
    );
}

#[test]
fn builtin_mtp_has_no_draft_model() {
    let a = args(&plan(Mtp::Builtin, 0), 1);
    assert!(a.ends_with(&["--spec-type".into(), "draft-mtp".into()]));
}

#[test]
fn never_unload_omits_sleep() {
    let a = args(&plan(Mtp::Off, 0), 1);
    assert!(!a.contains(&"--sleep-idle-seconds".to_string()));
    assert!(!a.contains(&"--spec-type".to_string()));
}

#[tokio::test]
async fn ready_when_health_returns_200() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(2)
        .mount(&server)
        .await;
    let mut p = Fake {
        exit: None,
        killed: false,
    };
    let http = hf_client(&server.uri());
    wait_ready(&http, &server.uri(), &mut p, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(!p.killed);
}

#[tokio::test]
async fn early_exit_reports_code_and_tail() {
    let server = MockServer::start().await;
    let mut p = Fake {
        exit: Some(1),
        killed: false,
    };
    let http = hf_client(&server.uri());
    let err = wait_ready(&http, &server.uri(), &mut p, Duration::from_secs(60))
        .await
        .unwrap_err();
    assert!(err.contains("code 1"), "{err}");
    assert!(err.contains("error: unknown model architecture"), "{err}");
}

#[tokio::test]
async fn timeout_kills_process_and_reports_tail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let mut p = Fake {
        exit: None,
        killed: false,
    };
    let http = hf_client(&server.uri());
    let err = wait_ready(&http, &server.uri(), &mut p, Duration::from_millis(600))
        .await
        .unwrap_err();
    assert!(p.killed);
    assert!(err.contains("did not become ready"), "{err}");
    assert!(err.contains("unknown model architecture"), "{err}");
}

#[test]
fn real_launcher_captures_tail_of_a_failing_process() {
    use std::time::Instant;
    use wizard101_chat_translator::local::server::{Launcher, RealLauncher};
    // 測試執行檔本身不認得 llama 參數：印錯誤到 stderr 後以非零碼結束。
    let mut p = plan(Mtp::Off, 0);
    p.exe = std::env::current_exe().unwrap();
    let mut proc = RealLauncher.launch(&p, 1).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let code = loop {
        if let Some(c) = proc.exited() {
            break c;
        }
        assert!(Instant::now() < deadline, "process did not exit");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_ne!(code, 0);
    let tail = proc.tail();
    assert!(
        tail.iter().any(|l| l.contains("Unrecognized option")),
        "{tail:?}"
    );
}
