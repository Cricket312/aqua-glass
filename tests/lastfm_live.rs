//! Live test against the real Last.fm API. Ignored by default (needs network + key):
//!
//!     set AQUA_LASTFM_KEY=<your key>          (PowerShell: $env:AQUA_LASTFM_KEY="<key>")
//!     cargo test --release --test lastfm_live -- --ignored --nocapture
//!
//! Without a Rust toolchain, run the same checks with the released exe:
//!     aqua-glass.exe --radio-selftest "Radiohead" "Karma Police"

use aqua_glass::radio::self_test;

fn key() -> String {
    std::env::var("AQUA_LASTFM_KEY").expect("set AQUA_LASTFM_KEY to a Last.fm API key")
}

#[test]
#[ignore]
fn real_lastfm_well_known_song() {
    let r = self_test(&key(), "Radiohead", "Karma Police", &[]);
    println!("{}", r.report);
    assert!(r.ok(), "{} checks failed", r.failed);
}

#[test]
#[ignore]
fn real_lastfm_other_genres() {
    for (a, t) in [("Daft Punk", "Get Lucky"), ("Miles Davis", "So What"), ("Sabrina Carpenter", "Espresso")] {
        let r = self_test(&key(), a, t, &[]);
        println!("{}", r.report);
        assert!(r.ok(), "{a} — {t}: {} checks failed", r.failed);
    }
}

#[test]
#[ignore]
fn real_lastfm_unknown_song_is_handled() {
    // a nonsense seed must not crash; Last.fm answers "not found" → empty results, failed checks are expected
    let r = self_test(&key(), "zzqx no such artist 9431", "nothing at all", &[]);
    println!("{}", r.report);
    assert!(r.report.contains("Result:") || r.report.contains("failed"));
}
