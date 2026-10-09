use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use tuifolio::model::{Asset, AssetKind, AssetMetadataSource, Price, StoreData};

fn invalid_usd_store() -> StoreData {
    let mut data = StoreData {
        next_id: 3,
        ..StoreData::default()
    };
    for (id, symbol, kind) in [(1, "EUR", AssetKind::Fiat), (2, "USD", AssetKind::Crypto)] {
        data.assets.push(Asset {
            id,
            symbol: symbol.into(),
            name: symbol.into(),
            kind,
            yahoo_symbol: None,
            tradingview_symbol: None,
            valuation_currency: None,
            metadata_source: AssetMetadataSource::User,
        });
    }
    data.prices.push(Price {
        asset_id: 1,
        timestamp: chrono::Utc::now(),
        price: rust_decimal::Decimal::ONE,
        currency: "USD".into(),
        source: "manual".into(),
    });
    data
}

#[test]
fn cli_refresh_rejects_stored_crypto_usd_without_aborting_or_rewriting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let original = serde_json::to_vec(&invalid_usd_store()).unwrap();
    std::fs::write(&path, &original).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tuifolio"))
        .args(["--store", path.to_str().unwrap(), "sync-prices"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("USD must use the Cash asset type"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn startup_refresh_keeps_web_server_alive_and_manual_refresh_reports_configuration_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let logs = directory.path().join("server.log");
    let original = serde_json::to_vec(&invalid_usd_store()).unwrap();
    std::fs::write(&path, &original).unwrap();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_tuifolio"))
            .args([
                "--store",
                path.to_str().unwrap(),
                "web",
                "--port",
                "0",
                "--price-refresh-seconds",
                "1",
            ])
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&logs).unwrap())
            .spawn()
            .unwrap(),
    );
    let stdout = server.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(url) = line.strip_prefix("Tuifolio: ") {
                let _ = send.send(url.to_string());
                break;
            }
        }
    });
    let base = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("server did not start");
    reader.join().unwrap();

    // Wait for the actual startup refresh, rather than merely testing an HTTP handler.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited during startup refresh"
        );
        if std::fs::read_to_string(&logs)
            .unwrap()
            .contains("USD must use the Cash asset type")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "startup refresh did not report its error"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/assets"))
            .send()
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    for _ in 0..2 {
        let response = client
            .post(format!("{base}/assets/sync"))
            .header("Origin", &base)
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            response
                .text()
                .unwrap()
                .contains("USD must use the Cash asset type")
        );
    }
    assert!(server.0.try_wait().unwrap().is_none());
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
