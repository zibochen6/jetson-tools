use russh::ChannelMsg;

use super::error::SshError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Present only if the server reported an exit status (russh exposes it as
    /// an explicit message; a closed channel without a status leaves `None`).
    pub exit_code: Option<u32>,
}

/// Abstraction over "run a command on the Jetson". Phase 2 is a single
/// implementation over russh; Phase 3 provisioning and tests get mock impls.
#[allow(async_fn_in_trait)] // native async trait; we use concrete types, not dyn
pub trait RemoteExecutor {
    #[allow(dead_code)] // Phase 3 provisioning uses plain exec for status/version probes
    async fn exec(&mut self, cmd: &str) -> Result<RemoteCommandResult, SshError>;
    async fn exec_with_stdin(
        &mut self,
        cmd: &str,
        stdin: &[u8],
    ) -> Result<RemoteCommandResult, SshError>;

    /// Stream stdout line-by-line during execution (for provisioning progress)
    /// while still accumulating the final stdout/stderr/exit status.
    /// Default impl line-splits the buffered result; `SshSession` overrides it
    /// with true in-flight streaming and the longer provision timeout.
    async fn exec_with_stdin_lines<F: FnMut(&str) + Send>(
        &mut self,
        cmd: &str,
        stdin: &[u8],
        mut on_line: F,
    ) -> Result<RemoteCommandResult, SshError> {
        let r = self.exec_with_stdin(cmd, stdin).await?;
        let text = String::from_utf8_lossy(&r.stdout);
        for line in text.lines() {
            on_line(line);
        }
        Ok(r)
    }
}

/// Pure aggregation of `ChannelMsg`s into stdout/stderr/exit status.
/// Unit-testable without any network — feed it messages directly.
#[derive(Debug)]
pub struct StreamCollector {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<u32>,
    max_bytes: usize,
}

impl StreamCollector {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: None,
            max_bytes,
        }
    }

    /// Consume one channel message. Returns `Err(OutputTooLarge)` if stdout or
    /// stderr would exceed the limit (malicious/large remote output guard).
    pub fn collect(&mut self, msg: &ChannelMsg) -> Result<(), SshError> {
        match msg {
            ChannelMsg::Data { data } => self.push(data, true)?,
            ChannelMsg::ExtendedData { data, .. } => self.push(data, false)?,
            ChannelMsg::ExitStatus { exit_status } => self.exit_code = Some(*exit_status),
            _ => {}
        }
        Ok(())
    }

    fn push(&mut self, data: &[u8], is_stdout: bool) -> Result<(), SshError> {
        let buf = if is_stdout {
            &mut self.stdout
        } else {
            &mut self.stderr
        };
        if buf.len() + data.len() > self.max_bytes {
            return Err(SshError::OutputTooLarge);
        }
        buf.extend_from_slice(data);
        Ok(())
    }

    pub fn finish(self) -> RemoteCommandResult {
        RemoteCommandResult {
            stdout: self.stdout,
            stderr: self.stderr,
            exit_code: self.exit_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::error::SshError;

    #[test]
    fn collects_stdout_stderr_exit_status() {
        let mut c = StreamCollector::new(1024);
        c.collect(&ChannelMsg::Data {
            data: b"hello".to_vec().into(),
        })
        .unwrap();
        c.collect(&ChannelMsg::ExtendedData {
            data: b"err".to_vec().into(),
            ext: 1,
        })
        .unwrap();
        c.collect(&ChannelMsg::ExitStatus { exit_status: 7 })
            .unwrap();
        c.collect(&ChannelMsg::Eof).unwrap();

        let r = c.finish();
        assert_eq!(r.stdout, b"hello");
        assert_eq!(r.stderr, b"err");
        assert_eq!(r.exit_code, Some(7));
    }

    #[test]
    fn exit_status_remains_none_when_absent() {
        let mut c = StreamCollector::new(1024);
        c.collect(&ChannelMsg::Data {
            data: b"x".to_vec().into(),
        })
        .unwrap();
        c.collect(&ChannelMsg::Eof).unwrap();
        assert_eq!(c.finish().exit_code, None);
    }

    #[test]
    fn rejects_output_over_limit() {
        let mut c = StreamCollector::new(4);
        assert!(matches!(
            c.collect(&ChannelMsg::Data {
                data: b"12345".to_vec().into()
            }),
            Err(SshError::OutputTooLarge)
        ));
        // buffer unchanged after rejection
        assert!(c.stdout.is_empty());
        // exactly at the limit is accepted…
        c.collect(&ChannelMsg::Data {
            data: b"1234".to_vec().into(),
        })
        .unwrap();
        // …one byte over is rejected
        assert!(matches!(
            c.collect(&ChannelMsg::Data {
                data: b"x".to_vec().into()
            }),
            Err(SshError::OutputTooLarge)
        ));
    }
}
