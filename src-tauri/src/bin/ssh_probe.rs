// Temporary Phase-2 proof-of-connection dev binary.
// Usage: printf '<password>\n' | cargo run --bin ssh_probe -- <host> <username>
// Reads the password from stdin ONLY — never from argv, never committed to a file.
// This is NOT product code; it verifies the russh chain (connect → auth → detect.sh).

use std::env;

use russh::client::{self, Handler};
use russh::keys::PublicKeyOrCertificate;

struct Client;

impl Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Accept-all for the spike only. Product code uses TOFU (see ssh/handler.rs).
        Ok(true)
    }
}

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
    if password.is_empty() {
        eprintln!("error: password required on stdin");
        std::process::exit(2);
    }

    let config = client::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(15)),
        ..Default::default()
    };

    let mut session = client::connect(config.into(), (host.as_str(), 22), Client).await?;
    eprintln!("[probe] TCP+handshake ok");

    let auth = session.authenticate_password(&username, &password).await?;
    eprintln!("[probe] authenticate_password -> {auth:?}");
    if !auth.success() {
        eprintln!("[probe] AUTHENTICATION REJECTED");
        return Ok(());
    }

    let script = include_str!("../../../scripts/remote/detect.sh");

    let mut channel = session.channel_open_session().await?;
    channel.exec(true, "sh -s").await?;
    channel.data(script.as_bytes()).await?;
    channel.eof().await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status: Option<u32> = None;
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(15), channel.wait())
            .await
            .map_err(|_| "read timeout")?;
        match msg {
            Some(russh::ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
            Some(russh::ChannelMsg::ExtendedData { data, .. }) => stderr.extend_from_slice(&data),
            Some(russh::ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
            Some(russh::ChannelMsg::Close) | None => break,
            Some(_) => {}
        }
    }

    eprintln!("[probe] exit_status = {exit_status:?}");
    eprintln!("[probe] stdout_bytes = {}", stdout.len());
    println!("STDOUT:\n{}", String::from_utf8_lossy(&stdout));
    eprintln!("STDERR:\n{}", String::from_utf8_lossy(&stderr));

    Ok(())
}
