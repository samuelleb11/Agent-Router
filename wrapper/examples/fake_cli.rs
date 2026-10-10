//! Stand-in for `claude` in the wrapper's Windows lab (`tests/lab_windows.rs`). Never the
//! real thing, and never shipped: `cargo build` leaves examples alone, `cargo test` builds
//! this one beside the tests, on every platform.
//!
//! ```text
//! fake_cli <session id> <copy> [linger seconds] [exit code]
//! ```
//!
//! It writes its pid to `<copy>.pid`, then announces the session the way the CLI's
//! `system/init` does. It echoes its stdin to its stdout, and into `<copy>`, until EOF. Then
//! it says it saw the EOF, stays up `linger` seconds, as a CLI finishing a turn does, and
//! exits with `exit code`. Everything it says, the test first wrote into the wrapper. This
//! is what `/bin/sh` and `tee` are to the unix lab, without a shell Windows lacks.

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [id, copy, rest @ ..] = args.as_slice() else {
        // Run with nothing to stand in for, as a bare `cargo run --example` would: nothing to do.
        return;
    };
    let linger: f64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let code: i32 = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

    std::fs::write(format!("{copy}.pid"), std::process::id().to_string()).expect("pid file");
    let mut out = std::io::stdout().lock();
    let init = format!(
        r#"{{"type":"system","subtype":"init","cwd":"/lab","session_id":"{id}","tools":[]}}"#
    );
    out.write_all(format!("{init}\n").as_bytes())
        .and_then(|()| out.flush())
        .expect("stdout");

    let mut file = std::fs::File::create(copy).expect("the stdin copy");
    let mut input = std::io::stdin().lock();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if out.write_all(&buf[..n]).and_then(|()| out.flush()).is_err() {
            break;
        }
        file.write_all(&buf[..n])
            .and_then(|()| file.flush())
            .expect("the stdin copy");
    }
    drop(file);

    let _ = out
        .write_all(b"{\"type\":\"lab\",\"event\":\"stdin-closed\"}\n")
        .and_then(|()| out.flush());
    std::thread::sleep(std::time::Duration::from_secs_f64(linger));
    std::process::exit(code);
}
