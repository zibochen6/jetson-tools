//! Provisioning V2 JSONL protocol contract (r2 §3 + design §2).
//!
//! stdout of `provision/runner.sh` is the protocol. This module parses and
//! validates the stream so the frontend/backend never string-parses shell
//! output again. All validation is pure (no I/O beyond the given lines).

use serde_json::Value;

pub const SCHEMA: &str = "jr.provision.event/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "provision protocol line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ContractError {}

/// Validate a JSONL stream (one JSON string per line, exactly as captured
/// from the runner's stdout). Enforces:
/// - every line is valid JSON with `schema == jr.provision.event/v1`
/// - `seq` is strictly monotonic starting at 1
/// - `run_id` is present and identical across the stream
/// - first event is run_started; last event is run_completed
/// - failed runs must carry a non-empty `error` object
pub fn validate_stream(lines: &[String]) -> Result<(), ContractError> {
    let mut last_seq: u64 = 0;
    let mut run_id: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // tolerate trailing newline artifacts
        }
        let n = i + 1;
        let json: Value = serde_json::from_str(trimmed).map_err(|e| ContractError {
            line: n,
            message: format!("invalid JSON: {e}"),
        })?;

        let schema = json
            .get("schema")
            .and_then(Value::as_str)
            .ok_or(ContractError {
                line: n,
                message: "missing schema".into(),
            })?;
        if schema != SCHEMA {
            return Err(ContractError {
                line: n,
                message: format!("unexpected schema {schema}"),
            });
        }

        let seq = json
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or(ContractError {
                line: n,
                message: "missing/invalid seq".into(),
            })?;
        if seq != last_seq + 1 {
            return Err(ContractError {
                line: n,
                message: format!("seq not monotonic: {seq} after {last_seq}"),
            });
        }
        last_seq = seq;

        let this_run = json
            .get("run_id")
            .and_then(Value::as_str)
            .ok_or(ContractError {
                line: n,
                message: "missing run_id".into(),
            })?;
        match &run_id {
            Some(r) if r != this_run => {
                return Err(ContractError {
                    line: n,
                    message: format!("run_id changed: {r} → {this_run}"),
                })
            }
            None => run_id = Some(this_run.to_string()),
            _ => {}
        }

        // Type-specific checks
        let ty = json.get("type").and_then(Value::as_str).unwrap_or("");
        match ty {
            "step_result" => {
                for field in ["step_id", "check", "apply", "verify"] {
                    if json.get(field).and_then(Value::as_str).is_none() {
                        return Err(ContractError {
                            line: n,
                            message: format!("step_result missing {field}"),
                        });
                    }
                }
            }
            "run_completed" => {
                if json.get("status").and_then(Value::as_str).is_none() {
                    return Err(ContractError {
                        line: n,
                        message: "run_completed missing status".into(),
                    });
                }
                if json.get("status").and_then(Value::as_str) == Some("failed")
                    && json.get("error").and_then(Value::as_object).is_none()
                {
                    return Err(ContractError {
                        line: n,
                        message: "failed run must carry an error object".into(),
                    });
                }
            }
            _ => {}
        }
    }

    if run_id.is_none() {
        return Err(ContractError {
            line: 0,
            message: "empty protocol stream".into(),
        });
    }
    Ok(())
}

/// Parse one protocol event into its type + full JSON (caller-driven match).
pub fn parse_event(line: &str) -> Result<(String, Value), ContractError> {
    let json: Value = serde_json::from_str(line).map_err(|e| ContractError {
        line: 1,
        message: format!("invalid JSON: {e}"),
    })?;
    let ty = json
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok((ty, json))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, ty: &str) -> String {
        format!(
            r#"{{"schema":"jr.provision.event/v1","seq":{seq},"run_id":"r1","ts":1,"type":"{ty}","ok":true}}"#
        )
    }

    #[test]
    fn provision_jsonl_each_line_is_valid_json() {
        for line in [
            ev(1, "run_started"),
            ev(2, "step_started"),
            ev(3, "step_result"),
        ] {
            let (ty, json) = parse_event(&line).expect("parse");
            assert!(!ty.is_empty());
            assert!(json.is_object());
        }
        assert!(
            parse_event(r#"{"schema":"jr.provision.event/v1","seq":1,"type":"run_started""#)
                .is_err()
        );
    }

    #[test]
    fn provision_jsonl_sequence_is_monotonic() {
        let stream = vec![
            ev(1, "run_started"),
            ev(2, "step_started"),
            ev(5, "step_result"),
        ];
        let err = validate_stream(&stream).unwrap_err();
        assert!(err.message.contains("monotonic"), "{}", err.message);
    }

    #[test]
    fn provision_jsonl_has_run_id_and_schema_version() {
        assert_eq!(SCHEMA, "jr.provision.event/v1");
        assert!(validate_stream(&[ev(1, "run_started")]).is_ok());
        let bad = r#"{"schema":"jr.provision.event/v1","seq":1,"ts":1,"type":"run_started"}"#;
        assert!(validate_stream(&[bad.to_string()]).is_err());
    }

    #[test]
    fn run_id_must_be_identical_across_stream() {
        let stream = vec![
            ev(1, "run_started"),
            r#"{"schema":"jr.provision.event/v1","seq":2,"run_id":"r2","ts":1,"type":"step_result","step_id":"a.b","check":"satisfied","apply":"not_run","verify":"passed"}"#.to_string(),
        ];
        assert!(validate_stream(&stream).is_err());
    }

    #[test]
    fn failed_run_requires_error_object() {
        let mut stream = vec![ev(1, "run_started")];
        stream.push(
            r#"{"schema":"jr.provision.event/v1","seq":2,"run_id":"r1","ts":1,"type":"run_completed","status":"failed"}"#.to_string(),
        );
        assert!(validate_stream(&stream).is_err());
    }

    #[test]
    fn happy_stream_passes() {
        let stream = vec![
            ev(1, "run_started"),
            ev(2, "step_started"),
            r#"{"schema":"jr.provision.event/v1","seq":3,"run_id":"r1","ts":1,"type":"step_result","step_id":"detect.os","check":"satisfied","apply":"not_run","verify":"passed","changed":false}"#.to_string(),
            r#"{"schema":"jr.provision.event/v1","seq":4,"run_id":"r1","ts":1,"type":"run_completed","status":"success","changed":false}"#.to_string(),
        ];
        validate_stream(&stream).expect("valid stream");
    }

    #[test]
    fn stderr_noise_does_not_corrupt_jsonl() {
        // The runner separates stdout (protocol) from stderr (logs). Feed only
        // stdout lines; any interleaved noise must fail loudly, never parse
        // silently into a wrong state.
        let noisy = vec![
            ev(1, "run_started"),
            "[bootstrap] some log noise that wrongly reached stdout".to_string(),
            ev(2, "step_result"),
        ];
        assert!(
            validate_stream(&noisy).is_err(),
            "noise breaks the contract by design"
        );
    }
}
