//! The WASI runner executes a real `wasm32-wasi` module, captures its output, and contains a
//! runaway — proven with hand-written WAT so there's no build step.
#![allow(clippy::expect_used)]

use std::time::Duration;

use agent_host::Limits;
use agent_sandbox::{RunOpts, Sandbox, SandboxError};

/// A minimal WASI command module: writes `hello` to fd 1 (stdout) via `fd_write`, then returns.
const HELLO_WAT: &str = r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 100) "hello")
  (func (export "_start")
    (i32.store (i32.const 0) (i32.const 100))  ;; iovec[0].buf = 100
    (i32.store (i32.const 4) (i32.const 5))    ;; iovec[0].len = 5
    ;; fd_write(fd=1, iovs=0, iovs_len=1, nwritten=20)
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))))
"#;

#[test]
fn runs_a_wasi_module_and_captures_stdout() {
    let wasm = wat::parse_str(HELLO_WAT).expect("assemble WAT");
    let sandbox = Sandbox::from_binary(&wasm).expect("compile");
    let result = sandbox.run(RunOpts::default()).expect("run");

    assert_eq!(result.stdout, b"hello");
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_code, 0);
    assert!(result.fuel_used > 0);
}

#[test]
fn a_fuel_bomb_is_contained() {
    // `_start` loops forever; a small fuel budget stops it deterministically as a typed error.
    let wasm = wat::parse_str(
        r#"(module
             (memory (export "memory") 1)
             (func (export "_start") (loop $l (br $l))))"#,
    )
    .expect("assemble WAT");
    let sandbox = Sandbox::from_binary(&wasm).expect("compile");
    let opts = RunOpts {
        limits: Limits::default().with_fuel(100_000),
        ..RunOpts::default()
    };
    assert!(matches!(sandbox.run(opts), Err(SandboxError::FuelExhausted)));
}

#[test]
fn a_wall_clock_runaway_is_contained() {
    // Effectively unbounded fuel, tight wall-clock budget — the epoch kill switch stops it.
    let wasm = wat::parse_str(
        r#"(module
             (memory (export "memory") 1)
             (func (export "_start") (loop $l (br $l))))"#,
    )
    .expect("assemble WAT");
    let sandbox = Sandbox::from_binary(&wasm).expect("compile");
    let opts = RunOpts {
        limits: Limits::default()
            .with_fuel(u64::MAX)
            .with_wall_budget(Duration::from_millis(20)),
        ..RunOpts::default()
    };
    assert!(matches!(sandbox.run(opts), Err(SandboxError::Timeout)));
}
