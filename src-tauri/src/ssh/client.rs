use std::sync::{Arc, Mutex};

use russh::client::{self, Handle};
use russh::ChannelMsg;

use super::error::SshError;
use super::executor::{RemoteCommandResult, RemoteExecutor, StreamCollector};
use super::handler::{ClientHandler, TrustState};
use super::types::{HostKeyInfo, SshConfig, SshConnectionInput};

/// Outcome of the ephemeral connect step, before authentication.
pub enum SshConnectOutcome {
    Connected(SshSession),
    HostKeyUnknown(HostKeyInfo),
    HostKeyChanged {
        current: HostKeyInfo,
        expected: String,
    },
}

/// An ephemeral SSH session for a single probe. Phase 2 closes it after the
/// detection round-trip; a persistent session manager is deferred to Phase 3.
pub struct SshSession {
    handle: Handle<ClientHandler>,
    config: SshConfig,
}

/// Connect (TCP + SSH handshake with TOFU host-key verification). Never
/// performs authentication — the caller decides that after this returns.
pub async fn connect(
    input: &SshConnectionInput,
    expected_fingerprint: Option<&str>,
    config: &SshConfig,
) -> Result<SshConnectOutcome, SshError> {
    let trust = Arc::new(Mutex::new(TrustState {
        expected_fingerprint: expected_fingerprint.map(str::to_string),
        captured: None,
    }));
    let handler = ClientHandler {
        host: input.host.clone(),
        port: input.port,
        trust: trust.clone(),
    };

    let rconf = client::Config {
        inactivity_timeout: Some(config.command_timeout),
        ..Default::default()
    };

    let connect_fut = client::connect(Arc::new(rconf), (input.host.as_str(), input.port), handler);

    let result = tokio::time::timeout(config.connect_timeout, connect_fut).await;

    let handle = match result {
        Err(_elapsed) => return Err(SshError::Timeout),
        Ok(Err(e)) => {
            // A false check_server_key aborts the handshake and surfaces here;
            // the captured key tells us it was OUR TOFU rejection, not an error.
            let captured = trust.lock().unwrap().captured.clone();
            match (expected_fingerprint, captured) {
                (None, Some(info)) => return Ok(SshConnectOutcome::HostKeyUnknown(info)),
                (Some(exp), Some(info)) => {
                    return Ok(SshConnectOutcome::HostKeyChanged {
                        current: info,
                        expected: exp.to_string(),
                    });
                }
                _ => return Err(SshError::Connect(e)),
            }
        }
        Ok(Ok(handle)) => handle,
    };

    Ok(SshConnectOutcome::Connected(SshSession {
        handle,
        config: config.clone(),
    }))
}

impl SshSession {
    pub async fn authenticate_password(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<(), SshError> {
        let auth = tokio::time::timeout(
            self.config.command_timeout,
            self.handle.authenticate_password(username, password),
        )
        .await
        .map_err(|_| SshError::Timeout)?
        .map_err(SshError::Connect)?;

        if auth.success() {
            Ok(())
        } else {
            Err(SshError::AuthRejected)
        }
    }

    async fn exec_inner(
        &mut self,
        cmd: &str,
        stdin: Option<&[u8]>,
    ) -> Result<RemoteCommandResult, SshError> {
        let mut channel = tokio::time::timeout(
            self.config.command_timeout,
            self.handle.channel_open_session(),
        )
        .await
        .map_err(|_| SshError::Timeout)?
        .map_err(SshError::Connect)?;

        channel.exec(true, cmd).await.map_err(SshError::Connect)?;

        if let Some(data) = stdin {
            channel.data(data).await.map_err(SshError::Connect)?;
            channel.eof().await.map_err(SshError::Connect)?;
        }

        let mut collector = StreamCollector::new(self.config.max_output_bytes);
        loop {
            let msg = tokio::time::timeout(self.config.command_timeout, channel.wait())
                .await
                .map_err(|_| SshError::Timeout)?;
            match msg {
                // Read past Eof so we still capture ExitStatus (OpenSSH sends
                // the exit-status request after channel EOF). Break only when
                // the channel is actually closing.
                Some(ChannelMsg::Close) | None => break,
                Some(m) => collector.collect(&m)?,
            }
        }
        Ok(collector.finish())
    }
}

impl RemoteExecutor for SshSession {
    async fn exec(&mut self, cmd: &str) -> Result<RemoteCommandResult, SshError> {
        self.exec_inner(cmd, None).await
    }

    async fn exec_with_stdin(
        &mut self,
        cmd: &str,
        stdin: &[u8],
    ) -> Result<RemoteCommandResult, SshError> {
        self.exec_inner(cmd, Some(stdin)).await
    }

    async fn exec_with_stdin_lines<F: FnMut(&str) + Send>(
        &mut self,
        cmd: &str,
        stdin: &[u8],
        mut on_line: F,
    ) -> Result<RemoteCommandResult, SshError> {
        let mut channel = tokio::time::timeout(
            self.config.provision_timeout,
            self.handle.channel_open_session(),
        )
        .await
        .map_err(|_| SshError::Timeout)?
        .map_err(SshError::Connect)?;

        channel.exec(true, cmd).await.map_err(SshError::Connect)?;
        channel.data(stdin).await.map_err(SshError::Connect)?;
        channel.eof().await.map_err(SshError::Connect)?;

        let mut collector = StreamCollector::new(self.config.max_output_bytes);
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let msg = tokio::time::timeout(self.config.provision_timeout, channel.wait())
                .await
                .map_err(|_| SshError::Timeout)?;
            match msg {
                Some(ChannelMsg::Data { data }) => {
                    collector.collect(&ChannelMsg::Data { data: data.clone() })?;
                    pending.extend_from_slice(&data);
                    drain_lines(&mut pending, &mut on_line);
                }
                Some(ChannelMsg::Close) | None => break,
                Some(m) => collector.collect(&m)?,
            }
        }
        flush_line(&mut pending, &mut on_line);
        Ok(collector.finish())
    }
}

/// Split complete `\n`-terminated lines out of the pending buffer, invoking
/// `on_line` for each (used to stream provisioning progress mid-command).
fn drain_lines<F: FnMut(&str)>(pending: &mut Vec<u8>, on_line: &mut F) {
    while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = pending.drain(..=pos).collect();
        let s = String::from_utf8_lossy(&line);
        let s = s.trim_end_matches(['\r', '\n']);
        if !s.is_empty() {
            on_line(s);
        }
    }
}

/// Flush a final unterminated line (e.g. a tail without a trailing newline).
fn flush_line<F: FnMut(&str)>(pending: &mut Vec<u8>, on_line: &mut F) {
    if pending.is_empty() {
        return;
    }
    let s = String::from_utf8_lossy(pending);
    let s = s.trim_end_matches(['\r', '\n']);
    if !s.is_empty() {
        on_line(s);
    }
    pending.clear();
}
