// Temporary Phase-2 real-device verification harness.
// Exercises the ACTUAL product SSH control plane (TOFU / trust / changed /
// auth failures / detection) against a real Jetson, headlessly.
// Usage: printf '<password>\n' | cargo run --bin probe_verify -- <host> <username>

use std::env;
use std::time::Duration;

use jetson_remote_lib::bootstrap::checker;
use jetson_remote_lib::bootstrap::types::RemoteEnvironmentState;
use jetson_remote_lib::device::detector::{self, DetectOutcome};
use jetson_remote_lib::device::types::JetsonDevice;
use jetson_remote_lib::ssh::client as ssh;
use jetson_remote_lib::ssh::error::SshError;
use jetson_remote_lib::ssh::types::{HostKeyInfo, SshConfig, SshConnectionInput};
use jetson_remote_lib::trust::TrustStoreFile;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let host = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "192.168.100.164".into());
    let username = args.get(2).cloned().unwrap_or_else(|| "seeed".into());

    let mut password = String::new();
    std::io::stdin().read_line(&mut password)?;
    let password = password.trim().to_string();

    let input = SshConnectionInput {
        host: host.clone(),
        port: 22,
        username,
        password: Some(password),
    };

    let results = run(&host, &input, 22).await;

    println!("\n===== REAL-DEVICE VERIFY SUMMARY =====");
    for (name, ok, detail) in &results {
        println!(
            "[{}] {} — {}",
            if *ok { "PASS" } else { "FAIL" },
            name,
            detail
        );
    }
    let all_ok = results.iter().all(|(_, ok, _)| *ok);
    println!(
        "\n{}/{} checks passed",
        results.iter().filter(|(_, ok, _)| *ok).count(),
        results.len()
    );
    std::process::exit(if all_ok { 0 } else { 1 });
}

async fn run(host: &str, input: &SshConnectionInput, port: u16) -> Vec<(String, bool, String)> {
    let mut results: Vec<(String, bool, String)> = vec![];
    let config = SshConfig::default();
    let store_dir = std::env::temp_dir().join(format!("jr-verify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store_dir);
    std::fs::create_dir_all(&store_dir).expect("mkdir");

    // 1. Fresh connect → HostKeyUnknown (TOFU prompt), capture the real key.
    let mut store = TrustStoreFile::load(store_dir).unwrap();
    let real_key: HostKeyInfo = match ssh::connect(input, None, &config).await {
        Ok(ssh::SshConnectOutcome::HostKeyUnknown(key)) => {
            results.push((
                "fresh connect → HostKeyUnknown".into(),
                true,
                format!("{} {}", key.algorithm, key.fingerprint),
            ));
            key
        }
        Ok(_) => {
            results.push((
                "fresh connect → HostKeyUnknown".into(),
                false,
                "server key was accepted without trust (TOFU broken)".into(),
            ));
            return results;
        }
        Err(e) => {
            results.push((
                "fresh connect → HostKeyUnknown".into(),
                false,
                format!("connect error: {e}"),
            ));
            return results;
        }
    };

    // 2. Trust + reconnect → authenticate + detect → device info.
    store.save(&real_key).unwrap();
    match ssh::connect(input, store.get_fingerprint(host, port).as_deref(), &config).await {
        Ok(ssh::SshConnectOutcome::Connected(mut s)) => {
            match s
                .authenticate_password(&input.username, input.password.as_deref().unwrap_or_default())
                .await
            {
                Ok(()) => match detector::detect(&mut s).await {
                    Ok(DetectOutcome::Device(d)) => {
                        let dev = JetsonDevice::from_detection(host, d);
                        results.push((
                            "trust + auth + detect → device info".into(),
                            true,
                            format!(
                                "model={} | jetpack={} | l4t={} | ubuntu={} | arch={}",
                                dev.model.unwrap_or_default(),
                                dev.jetpack_version.unwrap_or_default(),
                                dev.l4t_version.unwrap_or_default(),
                                dev.ubuntu_version.unwrap_or_default(),
                                dev.architecture.unwrap_or_default()
                            ),
                        ));
                    }
                    Ok(DetectOutcome::NotJetson) => results.push((
                        "trust + auth + detect".into(),
                        false,
                        "reported NotJetson (should be a Jetson)".into(),
                    )),
                    Err(e) => results.push((
                        "trust + auth + detect".into(),
                        false,
                        format!("detect error: {e}"),
                    )),
                },
                Err(e) => results.push(("trust + auth".into(), false, format!("auth error: {e}"))),
            }
        }
        Ok(_) => results.push((
            "trust + reconnect".into(),
            false,
            "expected Connected after trust".into(),
        )),
        Err(e) => results.push((
            "trust + reconnect".into(),
            false,
            format!("connect error: {e}"),
        )),
    }

    // 3. Wrong password → AuthenticationFailed.
    let mut bad = input.clone();
    bad.password = Some("definitely-wrong-password".into());
    match ssh::connect(&bad, store.get_fingerprint(host, port).as_deref(), &config).await {
        Ok(ssh::SshConnectOutcome::Connected(mut s)) => {
            match s.authenticate_password(&bad.username, bad.password.as_deref().unwrap_or_default()).await {
                Err(SshError::AuthRejected) => results.push((
                    "wrong password → AuthenticationFailed".into(),
                    true,
                    "rejected as expected".into(),
                )),
                other => results.push((
                    "wrong password → AuthenticationFailed".into(),
                    false,
                    format!("unexpected auth result: {other:?}"),
                )),
            }
        }
        other => results.push((
            "wrong password → AuthenticationFailed".into(),
            false,
            format!("unexpected connect: {}", describe_connect(&other)),
        )),
    }

    // 4. Wrong IP → connect failure (timeout / refused).
    let mut bad_ip = input.clone();
    bad_ip.host = "192.168.100.200".into(); // unoccupied on this subnet
    let fast = SshConfig {
        connect_timeout: Duration::from_secs(4),
        ..Default::default()
    };
    match ssh::connect(&bad_ip, None, &fast).await {
        Err(SshError::Timeout) | Err(SshError::Connect(_)) => results.push((
            "wrong IP → unreachable".into(),
            true,
            "connect failed as expected".into(),
        )),
        other => results.push((
            "wrong IP → unreachable".into(),
            false,
            format!("unexpected: {}", describe_connect(&other)),
        )),
    }

    // 5. Tampered trusted fingerprint → HostKeyChanged (never silently accepted).
    let store2_dir = std::env::temp_dir().join(format!("jr-verify2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store2_dir);
    std::fs::create_dir_all(&store2_dir).unwrap();
    let mut store2 = TrustStoreFile::load(store2_dir).unwrap();
    let fake = HostKeyInfo {
        host: host.to_string(),
        port,
        algorithm: "ssh-ed25519".into(),
        fingerprint: "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
    };
    store2.save(&fake).unwrap();
    match ssh::connect(
        input,
        store2.get_fingerprint(host, port).as_deref(),
        &config,
    )
    .await
    {
        Ok(ssh::SshConnectOutcome::HostKeyChanged { current, expected }) => results.push((
            "tampered store → HostKeyChanged".into(),
            true,
            format!("prev={expected} cur={}", current.fingerprint),
        )),
        other => results.push((
            "tampered store → HostKeyChanged".into(),
            false,
            format!("unexpected: {}", describe_connect(&other)),
        )),
    }

    // 6. Check environment → Ready (fast path; no provision happens here).
    match ssh::connect(input, store.get_fingerprint(host, port).as_deref(), &config).await {
        Ok(ssh::SshConnectOutcome::Connected(mut s)) => {
            match s
                .authenticate_password(&input.username, input.password.as_deref().unwrap_or_default())
                .await
            {
                Ok(()) => match checker::check(&mut s).await {
                    Ok(report) => results.push((
                        "check environment → Ready".into(),
                        report.state == RemoteEnvironmentState::Ready,
                        format!("state={:?} issues={}", report.state, report.issues.len()),
                    )),
                    Err(e) => results.push((
                        "check environment → Ready".into(),
                        false,
                        format!("check error: {e}"),
                    )),
                },
                Err(e) => results.push((
                    "check environment → Ready".into(),
                    false,
                    format!("auth error: {e}"),
                )),
            }
        }
        other => results.push((
            "check environment → Ready".into(),
            false,
            format!("unexpected: {}", describe_connect(&other)),
        )),
    }

    results
}

fn describe_connect<T>(_: &T) -> &'static str {
    "see branch (no Debug on SshConnectOutcome)"
}
