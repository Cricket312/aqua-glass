//! Shared, per-service request limiter.
//!
//! * **Pacing**: a token bucket per service (Spotify ≈ 3 req/s sustained, small
//!   burst). Background work (sync, radio, transfer) may only use the bucket
//!   while it keeps headroom for interactive requests (search, play, like).
//! * **One global cooldown**: when the service answers 429, every thread waits
//!   until `Retry-After` has passed (exactly as requested; exponential back-off
//!   when the header is missing). No thread keeps retrying during a cooldown.
//! * Interactive requests don't hang for long cooldowns: they fail fast with a
//!   "try again in N s" error; background requests wait (bounded).

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Tokens added per second.
    pub rate: f64,
    /// Bucket size (burst).
    pub burst: f64,
    /// Tokens background requests must leave for interactive ones.
    pub background_reserve: f64,
}

pub fn limits_for(host: &str) -> Limits {
    if host.contains("spotify.com") {
        Limits { rate: 3.0, burst: 6.0, background_reserve: 3.0 }
    } else {
        Limits { rate: 5.0, burst: 10.0, background_reserve: 2.0 }
    }
}

#[derive(Debug)]
pub struct State {
    pub limits: Limits,
    tokens: f64,
    refilled: Instant,
    /// No requests before this instant (set by 429 responses).
    pub blocked_until: Option<Instant>,
    /// Consecutive 429s without Retry-After (for exponential back-off).
    strikes: u32,
    pub total_429: u64,
    pub last_retry_after: Option<Duration>,
}

impl State {
    pub fn new(limits: Limits, now: Instant) -> State {
        State { limits, tokens: limits.burst, refilled: now, blocked_until: None, strikes: 0, total_429: 0, last_retry_after: None }
    }

    fn refill(&mut self, now: Instant) {
        let dt = now.saturating_duration_since(self.refilled).as_secs_f64();
        self.tokens = (self.tokens + dt * self.limits.rate).min(self.limits.burst);
        self.refilled = now;
    }

    /// How long the caller must wait before it may send (0 = send now; a token is consumed).
    pub fn reserve(&mut self, now: Instant, background: bool) -> Duration {
        if let Some(b) = self.blocked_until {
            if now < b {
                return b - now;
            }
            self.blocked_until = None;
        }
        self.refill(now);
        let need = if background { 1.0 + self.limits.background_reserve } else { 1.0 };
        if self.tokens >= need {
            self.tokens -= 1.0;
            Duration::ZERO
        } else {
            Duration::from_secs_f64(((need - self.tokens) / self.limits.rate).max(0.01))
        }
    }

    /// Register a 429. Returns the cooldown applied.
    pub fn penalize(&mut self, now: Instant, retry_after: Option<Duration>) -> Duration {
        self.total_429 += 1;
        let wait = match retry_after {
            Some(d) => {
                self.strikes = 0;
                d + Duration::from_millis(250)
            }
            None => {
                self.strikes += 1;
                Duration::from_secs((2u64 << self.strikes.min(5)).min(60))
            }
        };
        self.last_retry_after = Some(wait);
        let until = now + wait;
        // never shorten an existing cooldown
        self.blocked_until = Some(self.blocked_until.map_or(until, |b| b.max(until)));
        self.tokens = 0.0;
        self.refilled = now;
        wait
    }

    pub fn success(&mut self) {
        self.strikes = 0;
    }

    pub fn cooldown_left(&self, now: Instant) -> Option<Duration> {
        self.blocked_until.filter(|b| *b > now).map(|b| b - now)
    }
}

static STATES: OnceLock<Mutex<HashMap<String, State>>> = OnceLock::new();

fn states() -> &'static Mutex<HashMap<String, State>> {
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.to_string())).unwrap_or_default()
}

thread_local! {
    static BACKGROUND: Cell<bool> = const { Cell::new(false) };
}

/// Mark the current thread's requests as background work for the guard's lifetime.
pub struct BackgroundGuard(bool);

pub fn background() -> BackgroundGuard {
    BackgroundGuard(BACKGROUND.with(|b| b.replace(true)))
}

impl Drop for BackgroundGuard {
    fn drop(&mut self) {
        let prev = self.0;
        BACKGROUND.with(|b| b.set(prev));
    }
}

pub fn is_background() -> bool {
    BACKGROUND.with(|b| b.get())
}

/// Longest an interactive / background request will wait for a cooldown.
pub const INTERACTIVE_MAX_WAIT: Duration = Duration::from_secs(8);
pub const BACKGROUND_MAX_WAIT: Duration = Duration::from_secs(120);

/// Wait for permission to send. Err(remaining) when the wait would exceed the caller's budget.
pub fn acquire(host: &str) -> Result<(), Duration> {
    let bg = is_background();
    let max = if bg { BACKGROUND_MAX_WAIT } else { INTERACTIVE_MAX_WAIT };
    let started = Instant::now();
    loop {
        let wait = {
            let mut m = states().lock().unwrap();
            let now = Instant::now();
            let st = m.entry(host.to_string()).or_insert_with(|| State::new(limits_for(host), now));
            st.reserve(now, bg)
        };
        if wait.is_zero() {
            return Ok(());
        }
        if started.elapsed() + wait > max {
            return Err(wait);
        }
        std::thread::sleep(wait.min(Duration::from_secs(5)));
    }
}

pub fn on_429(host: &str, retry_after: Option<Duration>) -> Duration {
    let mut m = states().lock().unwrap();
    let now = Instant::now();
    let st = m.entry(host.to_string()).or_insert_with(|| State::new(limits_for(host), now));
    let w = st.penalize(now, retry_after);
    log::warn!("{host}: rate limited (429), pausing all requests for {:.1}s", w.as_secs_f32());
    w
}

pub fn on_success(host: &str) {
    if let Some(st) = states().lock().unwrap().get_mut(host) {
        st.success();
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub cooldown_left: Option<Duration>,
    pub total_429: u64,
    pub last_retry_after: Option<Duration>,
}

/// Status for the UI (e.g. "Spotify asked Aqua Glass to slow down").
pub fn status(host_contains: &str) -> Status {
    let m = states().lock().unwrap();
    let now = Instant::now();
    m.iter().filter(|(h, _)| h.contains(host_contains)).fold(Status::default(), |mut s, (_, st)| {
        s.total_429 += st.total_429;
        s.last_retry_after = s.last_retry_after.max(st.last_retry_after);
        s.cooldown_left = s.cooldown_left.max(st.cooldown_left(now));
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim() -> Limits {
        Limits { rate: 3.0, burst: 6.0, background_reserve: 3.0 }
    }

    #[test]
    fn paces_after_burst() {
        let t0 = Instant::now();
        let mut s = State::new(lim(), t0);
        for _ in 0..6 {
            assert!(s.reserve(t0, false).is_zero());
        }
        let w = s.reserve(t0, false);
        assert!(w > Duration::from_millis(300) && w < Duration::from_millis(400), "{w:?}");
        // a third of a second later one token is back
        assert!(s.reserve(t0 + Duration::from_millis(340), false).is_zero());
    }

    #[test]
    fn background_leaves_headroom_for_interactive() {
        let t0 = Instant::now();
        let mut s = State::new(lim(), t0);
        let mut bg = 0;
        while s.reserve(t0, true).is_zero() {
            bg += 1;
        }
        assert_eq!(bg, 3, "background stops while 3 tokens remain");
        for _ in 0..3 {
            assert!(s.reserve(t0, false).is_zero(), "interactive still goes through");
        }
    }

    #[test]
    fn honours_retry_after_for_everyone_and_never_shortens() {
        let t0 = Instant::now();
        let mut s = State::new(lim(), t0);
        let w = s.penalize(t0, Some(Duration::from_secs(40)));
        assert!(w >= Duration::from_secs(40));
        // nobody may send during the cooldown, even with a full bucket's worth of time passing
        let left = s.reserve(t0 + Duration::from_secs(10), false);
        assert!(left >= Duration::from_secs(30), "{left:?}");
        // a shorter Retry-After arriving later doesn't shorten the cooldown
        s.penalize(t0 + Duration::from_secs(1), Some(Duration::from_secs(2)));
        assert!(s.reserve(t0 + Duration::from_secs(5), false) > Duration::from_secs(30));
        // after it expires requests flow again (but the bucket starts empty › paced)
        let after = t0 + Duration::from_secs(41);
        assert!(s.reserve(after, false) < Duration::from_secs(1));
        assert_eq!(s.total_429, 2);
    }

    #[test]
    fn exponential_backoff_without_header() {
        let t0 = Instant::now();
        let mut s = State::new(lim(), t0);
        let a = s.penalize(t0, None);
        let b = s.penalize(t0, None);
        let c = s.penalize(t0, None);
        assert!(a < b && b < c, "{a:?} {b:?} {c:?}");
        for _ in 0..10 {
            s.penalize(t0, None);
        }
        assert!(s.last_retry_after.unwrap() <= Duration::from_secs(60));
        s.success();
        assert_eq!(s.penalize(t0, None), Duration::from_secs(4));
    }

    #[test]
    fn background_flag_is_scoped() {
        assert!(!is_background());
        {
            let _g = background();
            assert!(is_background());
        }
        assert!(!is_background());
    }
}
