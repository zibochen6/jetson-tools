use super::types::RdpConnectionConfig;

/// The full FreeRDP command line, split into two parts with a hard security
/// boundary between them:
/// - `argv`: the public process argv — must NEVER contain the password.
/// - `stdin`: the payload fed to the child's stdin — the only channel that
///   carries `/p:<password>`.
pub struct FreeRdpArguments {
    pub argv: Vec<String>,
    pub stdin: Vec<String>,
}

/// Serialize the stdin argument list, one argument per line (FreeRDP 3.31
/// `/args-from:stdin` contract, verified against the real binary).
pub fn stdin_payload(args: &FreeRdpArguments) -> Vec<u8> {
    let mut joined = args.stdin.join("\n");
    joined.push('\n');
    joined.into_bytes()
}

/// Build the sidecar invocation. `argv` holds only `[binary, /args-from:stdin]`
/// (the option "cannot be combined with any other" per `--help`); every option —
/// including the password — is written to stdin one line at a time.
pub fn build(binary: &str, config: &RdpConnectionConfig, title: &str) -> FreeRdpArguments {
    let mut stdin = vec![
        format!("/v:{}:{}", config.host, config.port),
        format!("/u:{}", config.username),
        format!("/p:{}", config.password), // NEVER in argv — stdin only
        "/cert:tofu".to_string(),          // TOFU, never /cert:ignore in product
        format!("/cert:name:{}", config.certificate_name),
        format!("/t:{title}"),
    ];
    if config.clipboard {
        stdin.push("+clipboard".to_string());
    }
    if config.dynamic_resolution {
        stdin.push("+dynamic-resolution".to_string());
    }

    FreeRdpArguments {
        argv: vec![binary.to_string(), "/args-from:stdin".to_string()],
        stdin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RdpConnectionConfig {
        RdpConnectionConfig {
            certificate_name: "192.168.100.164".into(),
            host: "192.168.100.164".into(),
            port: 3389,
            username: "seeed".into(),
            password: "hunter2".into(),
            dynamic_resolution: true,
            clipboard: true,
        }
    }

    #[test]
    fn password_never_in_argv() {
        let args = build("/opt/homebrew/bin/sdl-freerdp", &config(), "Jetson Remote");
        assert_eq!(
            args.argv,
            vec!["/opt/homebrew/bin/sdl-freerdp", "/args-from:stdin"]
        );
        let argv_joined = args.argv.join(" ");
        assert!(!argv_joined.contains("hunter2"));
    }

    #[test]
    fn password_carried_only_via_stdin() {
        let args = build("sdl-freerdp", &config(), "Jetson Remote");
        assert!(args.stdin.iter().any(|l| l == "/p:hunter2"));
        assert!(args
            .stdin
            .iter()
            .all(|l| l != "/p:hunter2" || l.starts_with("/p:")));
    }

    #[test]
    fn cert_is_tofu_not_ignore() {
        let args = build("sdl-freerdp", &config(), "title");
        assert!(args.stdin.iter().any(|l| l == "/cert:tofu"));
        assert!(!args.stdin.join("\n").contains("cert:ignore"));
    }

    #[test]
    fn required_options_present() {
        let args = build("sdl-freerdp", &config(), "Jetson Remote — 1.2.3.4");
        let joined = args.stdin.join("\n");
        assert!(joined.contains("/v:192.168.100.164:3389"));
        assert!(joined.contains("/cert:name:192.168.100.164"));
        assert!(joined.contains("/u:seeed"));
        assert!(joined.contains("+clipboard"));
        assert!(joined.contains("+dynamic-resolution"));
        assert!(joined.contains("/t:Jetson Remote — 1.2.3.4"));
    }

    #[test]
    fn flags_off_when_disabled() {
        let mut c = config();
        c.clipboard = false;
        c.dynamic_resolution = false;
        let args = build("sdl-freerdp", &c, "t");
        assert!(!args.stdin.iter().any(|l| l == "+clipboard"));
        assert!(!args.stdin.iter().any(|l| l == "+dynamic-resolution"));
    }

    #[test]
    fn stdin_payload_is_one_per_line() {
        let args = build("sdl-freerdp", &config(), "t");
        let payload = String::from_utf8(stdin_payload(&args)).unwrap();
        let lines: Vec<&str> = payload.lines().collect();
        assert_eq!(lines.len(), args.stdin.len());
        assert!(lines.contains(&"/cert:tofu"));
    }
}
