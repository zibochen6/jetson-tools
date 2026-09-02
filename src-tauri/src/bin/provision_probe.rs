// Temporary Phase-3 real-device provisioning harness (safe repair path).
// connect(TOFU) → auth → check → (if not Ready) provision → verify.
// Usage: printf '<password>\n' | cargo run --bin provision_probe -- <host> <username>

use std::env;

use jetson_remote_lib::bootstrap::checker;
use jetson_remote_lib::bootstrap::provisioner;
use jetson_remote_lib::bootstrap::types::{ProvisionStage, RemoteEnvironmentState};
use jetson_remote_lib::bootstrap::verifier;
use jetson_remote_lib::ssh::client as ssh;
use jetson_remote_lib::ssh::types::{SshConfig, SshConnectionInput};
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
    let config = SshConfig::default();

    // Fresh temp trust store → TOFU → save → reconnect.
    let store_dir = std::env::temp_dir().join(format!("jr-provision-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&store_dir);
    std::fs::create_dir_all(&store_dir)?;
    let mut store = TrustStoreFile::load(store_dir)?;

    match ssh::connect(&input, None, &config).await {
        Ok(ssh::SshConnectOutcome::HostKeyUnknown(key)) => {
            println!("[probe] TOFU captured {}", key.fingerprint);
            store.save(&key)?;
        }
        _ => {
            eprintln!("[probe] expected HostKeyUnknown on first connect");
            std::process::exit(2);
        }
    }

    let mut session =
        match ssh::connect(&input, store.get_fingerprint(&host, 22).as_deref(), &config).await {
            Ok(ssh::SshConnectOutcome::Connected(s)) => s,
            _ => {
                eprintln!("[probe] trusted reconnect failed");
                std::process::exit(2);
            }
        };
    session
        .authenticate_password(&input.username, input.password.as_deref().unwrap_or_default())
        .await?;

    let report = checker::check(&mut session).await?;
    println!(
        "[check] state={:?} issues={}",
        report.state,
        report.issues.len()
    );
    for i in &report.issues {
        println!("  - {i}");
    }

    if report.state == RemoteEnvironmentState::Ready {
        println!("[result] already Ready — fast path, no provision ran");
    } else {
        println!("[provision] state is {:?} — provisioning…", report.state);
        provisioner::provision(&mut session, input.password.as_deref().unwrap_or_default(), |ev| {
            if !matches!(
                ev.stage,
                ProvisionStage::Preflight | ProvisionStage::Uploading
            ) {
                println!("  [event] {:?}: {}", ev.stage, ev.message);
            }
        })
        .await?;
        let verified = verifier::verify(&mut session).await?;
        println!("[verify] state={:?}", verified.state);
    }

    Ok(())
}
