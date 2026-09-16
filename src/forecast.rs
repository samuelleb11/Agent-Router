//! When the quota runs out, from the burn already recorded.
//!
//! The reset time is not the interesting number — Anthropic hands it to us in the
//! `anthropic-ratelimit-unified-*` headers and `requests` stores it. The interesting
//! number is whether the way you are working gets you to 100% *before* that reset.
//!
//! ## Why burn is measured against active time
//!
//! The obvious model — utilisation against wall-clock time — is badly wrong, and the
//! store has the counter-example. In the 7-day window that reset on 2026-09-11 there
//! is a **31-hour gap between `u=0.97` and `u=0.98` with no requests at all**: sleep,
//! not slow burn. Fitted against wall-clock time that window predicted exhaustion
//! 15–33 hours early at every point, then flipped to "idle, never" once the gap was
//! inside the lookback. A wall-clock rate is really measuring what fraction of the
//! lookback you happened to be awake.
//!
//! Summing only the gaps that look like work (`IDLE_GAP_MS` or shorter) fixes it. The
//! same window, fitted the same way at eight points across 34 hours, held a steady
//! 1.17%/active-hour and landed within ~45 minutes of work of the truth. So the unit
//! here is **hours of work**, not hours on the clock; a calendar date is derived from
//! it through a duty cycle and is the softer of the two numbers.
//!
//! ## Why duty is a range
//!
//! Burn is a measurement; duty is a forecast, and the two do not deserve the same
//! typeface either. Holding one scalar duty constant from now to reset extrapolates
//! whatever you happened to be doing this week — and a week of long days, projected
//! flat across four more, is how a dead heat gets rendered in red. The account that
//! prompted this was at 49% with 1.68%/active-hour and 32.8% duty: a crossing 2.2
//! hours before a reset 94.7 hours away, from a record whose complete days ran 3.5 to
//! 8.5 hours of work. A 2.4% change in duty flipped the verdict.
//!
//! So `Duty` carries the spread of the days actually on record, and the projection is
//! run three times. The verdict is only [`Verdict::Crosses`] when even a quiet week
//! exhausts the window; when the band straddles the reset it says so
//! ([`Verdict::Tight`]) rather than picking a side of a coin flip.
//!
//! One known limitation: the burn rate depends on the model — Opus eats a window far
//! faster than Haiku — and this does not weight by model family. The traffic it was
//! fitted on was almost entirely Opus.

/// A gap longer than this is idle, not work. Claude Code's turns are seconds apart;
/// five minutes is well clear of thinking time and well under a coffee break.
pub const IDLE_GAP_MS: i64 = 300_000;

/// Utilisation arrives rounded to whole percent, so one step could be anything from
/// 0.5% to 1.5% of real burn. Two steps before we believe the slope.
const MIN_DELTA: f64 = 0.02;

/// …and enough work behind it that the denominator is not noise either.
const MIN_ACTIVE_MS: i64 = 600_000;

const HOUR_MS: f64 = 3_600_000.0;

#[derive(Clone, Copy, Debug)]
pub struct WindowKind {
    pub name: &'static str,
    pub span_ms: i64,
}

pub const FIVE_HOUR: WindowKind = WindowKind { name: "five_hour", span_ms: 5 * 3_600_000 };
pub const SEVEN_DAY: WindowKind = WindowKind { name: "seven_day", span_ms: 7 * 86_400_000 };

/// One request: when it went out, and the utilisation its response reported. Requests
/// that carried no rate-limit header still count — they are work, and work is the
/// denominator — so `util` is optional.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub at_ms: i64,
    pub util: Option<f64>,
}

/// Where the burn estimate came from, so the UI can say so rather than presenting a
/// guess and a measurement in the same typeface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    /// Fitted on the window being forecast.
    ThisWindow,
    /// The window is too young to have a slope; this is the account's own recent average.
    TrailingWindows,
    /// Nothing to go on.
    None,
}

/// How much of the clock goes to work, as a range rather than a number.
///
/// `mid` is the estimate; `low` and `high` bracket it with the spread of the days
/// actually on record, so a run of long days is not mistaken for a new normal. A
/// window forecast against the session you are in has no spread to speak of — use
/// [`Duty::flat`] there and the projection collapses to a single ray.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Duty {
    /// A quieter stretch than the one behind you.
    pub low: f64,
    /// The central estimate, and the one every other number is derived from.
    pub mid: f64,
    /// A busier one.
    pub high: f64,
}

impl Duty {
    /// One number, no spread: the projection runs as a single ray, exactly as it did
    /// before the band existed.
    pub fn flat(d: f64) -> Self {
        let d = d.clamp(0.01, 1.0);
        Duty { low: d, mid: d, high: d }
    }

    /// An estimate with a spread either side, ordered and clamped to something a
    /// duty cycle can actually be.
    pub fn spread(mid: f64, sd: f64) -> Self {
        let mid = mid.clamp(0.01, 1.0);
        let sd = sd.max(0.0);
        Duty { low: (mid - sd).clamp(0.01, 1.0), mid, high: (mid + sd).clamp(0.01, 1.0) }
    }
}

/// What the band says about this window, as opposed to what the central estimate says.
///
/// The distinction the old boolean could not draw is [`Verdict::Tight`]: a crossing
/// that lands inside the spread of your own working days is a possibility, not a
/// prediction, and saying otherwise in red spends the user's attention on a coin flip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Even a quiet week exhausts this window before it resets.
    Crosses,
    /// The band straddles the reset. It could go either way.
    Tight,
    /// Even a busy week does not fill it.
    ResetsFirst,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Forecast {
    /// 0..1, as last reported.
    pub used: f64,
    /// Epoch **seconds**, as stored.
    pub reset_at: i64,
    pub window_start: i64,
    /// Utilisation (0..1) consumed per hour of actual work.
    pub burn_per_active_hour: f64,
    /// How much more work the remaining quota buys. `None` when nothing is burning.
    pub active_hours_left: Option<f64>,
    /// Work ÷ wall clock over the horizon that matches this window. The central
    /// estimate — `duty.mid` — kept under its old name for clients that predate the band.
    pub duty_fraction: f64,
    /// The same figure with the spread of your days either side of it.
    pub duty: Duty,
    /// Epoch **milliseconds**, or `None` when the window resets before you get there.
    /// The central estimate; `exhausts_at_early` and `_late` bracket it.
    pub exhausts_at: Option<i64>,
    /// The crossing if the days ahead are as busy as your busiest — so, the soonest.
    pub exhausts_at_early: Option<i64>,
    /// The crossing if they are as quiet as your quietest.
    pub exhausts_at_late: Option<i64>,
    /// Whether the *central* estimate resets before it fills. Prefer `verdict`, which
    /// can also say the two are too close to call.
    pub resets_first: bool,
    pub verdict: Verdict,
    /// Where this window ends up if nothing changes. Capped at 1.0.
    pub projected_end_util: f64,
    /// Where it ends up on a quieter week than this one, and on a busier one.
    pub projected_end_util_low: f64,
    pub projected_end_util_high: f64,
    pub basis: Basis,
    /// The hours of work the estimate was fitted on, for the UI to disclose.
    pub measured_active_hours: f64,
    pub readings: usize,
}

/// Time spent working, from a series of request times: every gap, capped at the point
/// where a gap stops being a pause and starts being an absence.
pub fn active_ms(times: &[i64]) -> i64 {
    times
        .windows(2)
        .map(|w| (w[1] - w[0]).clamp(0, IDLE_GAP_MS))
        .sum()
}

/// Utilisation per active hour across one window's readings, with the hours it was
/// fitted on. `None` when the readings cannot support a slope — too few, too flat, or
/// too little work behind them.
pub fn burn_from(readings: &[Reading]) -> Option<(f64, f64)> {
    let first = readings.iter().find(|r| r.util.is_some())?;
    let last = readings.iter().rev().find(|r| r.util.is_some())?;
    let delta = last.util? - first.util?;
    if delta < MIN_DELTA {
        return None;
    }
    let times: Vec<i64> = readings
        .iter()
        .filter(|r| r.at_ms >= first.at_ms && r.at_ms <= last.at_ms)
        .map(|r| r.at_ms)
        .collect();
    let act = active_ms(&times);
    if act < MIN_ACTIVE_MS {
        return None;
    }
    let hours = act as f64 / HOUR_MS;
    Some((delta / hours, hours))
}

/// One duty cycle, carried to the reset: when the window would cross 100%, and where
/// it ends up if it does not. The crossing is `None` when the reset gets there first.
fn project(
    used: f64,
    burn: f64,
    duty: f64,
    now_ms: i64,
    reset_ms: i64,
) -> (Option<i64>, f64) {
    let hours_to_reset = ((reset_ms - now_ms) as f64 / HOUR_MS).max(0.0);
    let end = (used + burn * duty * hours_to_reset).min(1.0);
    let at = now_ms + ((1.0 - used) / burn / duty * HOUR_MS) as i64;
    (if at < reset_ms { Some(at) } else { None }, end)
}

/// Project one window to its end.
///
/// `readings` are this window's requests in time order; `reset_at` is epoch seconds as
/// stored; `fallback_burn` is the account's recent average for when this window is too
/// young to have a slope of its own; `duty` is the work-to-clock ratio over a horizon
/// the caller matched to the window — the current window for a 5-hour one, the trailing
/// week for a 7-day one. Measuring duty over the wrong horizon is the difference
/// between "you are mid-session at 90%" and "you work 3 hours a day".
pub fn forecast(
    readings: &[Reading],
    window: WindowKind,
    reset_at: i64,
    now_ms: i64,
    fallback_burn: Option<f64>,
    duty: Duty,
) -> Forecast {
    let used = readings
        .iter()
        .rev()
        .find_map(|r| r.util)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let reset_ms = reset_at.saturating_mul(1000);
    let window_start = reset_at - window.span_ms / 1000;
    let duty = Duty {
        low: duty.low.clamp(0.01, 1.0),
        mid: duty.mid.clamp(0.01, 1.0),
        high: duty.high.clamp(0.01, 1.0),
    };

    let (burn, basis, measured) = match burn_from(readings) {
        Some((b, h)) => (b, Basis::ThisWindow, h),
        None => match fallback_burn.filter(|b| *b > 0.0) {
            Some(b) => (b, Basis::TrailingWindows, 0.0),
            None => (0.0, Basis::None, 0.0),
        },
    };

    let mut out = Forecast {
        used,
        reset_at,
        window_start,
        burn_per_active_hour: burn,
        active_hours_left: None,
        duty_fraction: duty.mid,
        duty,
        exhausts_at: None,
        exhausts_at_early: None,
        exhausts_at_late: None,
        resets_first: true,
        verdict: Verdict::ResetsFirst,
        projected_end_util: used,
        projected_end_util_low: used,
        projected_end_util_high: used,
        basis,
        measured_active_hours: measured,
        readings: readings.len(),
    };

    if used >= 1.0 {
        // Already there. Say so rather than projecting a crossing in the past.
        out.active_hours_left = Some(0.0);
        out.exhausts_at = Some(now_ms);
        out.exhausts_at_early = Some(now_ms);
        out.exhausts_at_late = Some(now_ms);
        out.resets_first = false;
        out.verdict = Verdict::Crosses;
        out.projected_end_util = 1.0;
        out.projected_end_util_low = 1.0;
        out.projected_end_util_high = 1.0;
        return out;
    }
    if burn <= 0.0 {
        return out;
    }

    out.active_hours_left = Some((1.0 - used) / burn);

    // Three passes over the same arithmetic. A busier day spends the window sooner, so
    // `high` gives the earliest crossing and `low` the latest.
    let (mid_at, mid_end) = project(used, burn, duty.mid, now_ms, reset_ms);
    let (early_at, high_end) = project(used, burn, duty.high, now_ms, reset_ms);
    let (late_at, low_end) = project(used, burn, duty.low, now_ms, reset_ms);

    out.exhausts_at = mid_at;
    out.exhausts_at_early = early_at;
    out.exhausts_at_late = late_at;
    out.resets_first = mid_at.is_none();
    out.projected_end_util = mid_end;
    out.projected_end_util_low = low_end;
    out.projected_end_util_high = high_end;

    // The quiet end crossing means every end does; the busy end failing to means none
    // does. Anything between is a coin flip, and gets named as one.
    out.verdict = match (late_at.is_some(), early_at.is_some()) {
        (true, _) => Verdict::Crosses,
        (false, false) => Verdict::ResetsFirst,
        (false, true) => Verdict::Tight,
    };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    /// Requests every `step` from `start`, walking utilisation `from`→`to` linearly.
    fn run(start: i64, step: i64, n: usize, from: f64, to: f64) -> Vec<Reading> {
        (0..n)
            .map(|i| Reading {
                at_ms: start + i as i64 * step,
                // Quantised to whole percent, exactly as the header reports it.
                util: Some(((from + (to - from) * i as f64 / (n - 1) as f64) * 100.0).round() / 100.0),
            })
            .collect()
    }

    #[test]
    fn active_time_counts_pauses_but_not_absences() {
        let t = [0, MIN, 2 * MIN, 2 * MIN + 31 * 3_600_000, 2 * MIN + 31 * 3_600_000 + MIN];
        // Two one-minute gaps, one overnight gap capped at the idle threshold, one more minute.
        assert_eq!(active_ms(&t), 3 * MIN + IDLE_GAP_MS);
        assert_eq!(active_ms(&[]), 0);
        assert_eq!(active_ms(&[42]), 0);
    }

    /// The regression this module exists to prevent: the 31-hour hole in the 7-day
    /// window that reset on 2026-09-11 must not change the rate we fit.
    #[test]
    fn an_overnight_gap_does_not_change_the_burn_rate() {
        let dense = run(0, MIN, 121, 0.90, 0.96);
        let (a, _) = burn_from(&dense).expect("a slope");

        // The same work, same utilisation, with 31 idle hours dropped into the middle.
        let mut split: Vec<Reading> = dense[..60].to_vec();
        for r in &dense[60..] {
            split.push(Reading { at_ms: r.at_ms + 31 * 3_600_000, ..*r });
        }
        let (b, _) = burn_from(&split).expect("a slope");

        // Wall-clock fitting would report b at about 1/16 of a. Active time sees one
        // extra capped gap and nothing else.
        let drift = (a - b).abs() / a;
        assert!(drift < 0.06, "burn moved {:.1}% across the gap: {a} vs {b}", drift * 100.0);
    }

    #[test]
    fn a_window_too_young_to_have_a_slope_falls_back_to_the_recent_average() {
        // One 1% step, twenty minutes of work: under MIN_DELTA, so not a slope.
        let young = run(0, MIN, 21, 0.00, 0.01);
        assert!(burn_from(&young).is_none(), "one quantisation step is not a measurement");

        let f = forecast(&young, FIVE_HOUR, 18_000, 20 * MIN, Some(0.06), Duty::flat(0.9));
        assert_eq!(f.basis, Basis::TrailingWindows);
        assert_eq!(f.burn_per_active_hour, 0.06);

        // With nothing to fall back on it must refuse rather than invent.
        let g = forecast(&young, FIVE_HOUR, 18_000, 20 * MIN, None, Duty::flat(0.9));
        assert_eq!(g.basis, Basis::None);
        assert!(g.exhausts_at.is_none() && g.active_hours_left.is_none());
    }

    #[test]
    fn a_slow_window_resets_before_it_fills() {
        // 5.6%/active-hour is this account's median; a 5-hour window cannot reach 100%.
        let rs = run(0, MIN, 121, 0.00, 0.11);
        let now = 120 * MIN;
        let f = forecast(&rs, FIVE_HOUR, 18_000, now, None, Duty::flat(1.0));
        assert_eq!(f.basis, Basis::ThisWindow);
        assert!(f.resets_first, "5.5%/h cannot fill a 5-hour window");
        assert!(f.exhausts_at.is_none());
        assert!(f.projected_end_util < 1.0, "got {}", f.projected_end_util);
        // Three hours left at ~5.5%/h on top of 11% used lands near a third.
        assert!((f.projected_end_util - 0.28).abs() < 0.05, "got {}", f.projected_end_util);
        // The quota itself is nowhere near spent — it is the clock that stops you.
        assert!(f.active_hours_left.unwrap() > 5.0);
    }

    #[test]
    fn a_fast_window_names_the_hour_it_runs_out() {
        let rs = run(0, MIN, 121, 0.50, 0.80);
        let now = 120 * MIN;
        let f = forecast(&rs, FIVE_HOUR, 18_000, now, None, Duty::flat(1.0));
        assert!(!f.resets_first);
        let at = f.exhausts_at.expect("a crossing");
        // 15%/active-hour, 20% left, working flat out: about 80 minutes.
        let mins = (at - now) as f64 / 60_000.0;
        assert!((mins - 80.0).abs() < 8.0, "got {mins:.0} min");
        assert_eq!(f.projected_end_util, 1.0);
    }

    #[test]
    fn a_low_duty_cycle_pushes_the_date_out_without_moving_the_work_left() {
        let rs = run(0, MIN, 121, 0.00, 0.20);
        let now = 120 * MIN;
        let flat_out = forecast(&rs, SEVEN_DAY, 604_800, now, None, Duty::flat(1.0));
        let part_time = forecast(&rs, SEVEN_DAY, 604_800, now, None, Duty::flat(0.125));

        // The quota buys the same amount of work either way …
        let (a, b) = (flat_out.active_hours_left.unwrap(), part_time.active_hours_left.unwrap());
        assert!((a - b).abs() < 1e-9);
        // … but three hours a day takes eight times as long to spend it.
        let span = |f: &Forecast| (f.exhausts_at.unwrap() - now) as f64;
        assert!((span(&part_time) / span(&flat_out) - 8.0).abs() < 0.01);
    }

    #[test]
    fn an_exhausted_window_reports_now_rather_than_a_crossing_in_the_past() {
        let rs = run(0, MIN, 121, 0.90, 1.00);
        let now = 200 * MIN;
        let f = forecast(&rs, SEVEN_DAY, 604_800, now, None, Duty::flat(0.15));
        assert_eq!(f.used, 1.0);
        assert_eq!(f.exhausts_at, Some(now));
        assert_eq!(f.active_hours_left, Some(0.0));
        assert!(!f.resets_first);
    }

    /// The regression this band exists to prevent. The live 7-day window on 2026-09-14:
    /// 49% used, 1.68%/active-hour, a trailing week running 32.8% duty, and a reset
    /// 94.7 hours out. The central estimate crosses 2.2 hours early — but the complete
    /// days on record ran 3.5 to 8.5 hours of work, and a week at the quiet end of that
    /// does not fill the window at all. Calling that "runs out" in red is a coin flip
    /// with a verdict on it.
    #[test]
    fn a_crossing_inside_the_spread_of_your_own_days_is_tight_not_critical() {
        // Utilisation 0.00 → 0.49 over 29.1 active hours: the window as it stood.
        let rs = run(0, MIN, 1747, 0.00, 0.49);
        let now = 1746 * MIN;
        let reset_at = (now + (94.7 * HOUR_MS) as i64) / 1000;
        let duty = Duty::spread(0.328, 0.049);

        let f = forecast(&rs, SEVEN_DAY, reset_at, now, None, duty);
        assert_eq!(f.basis, Basis::ThisWindow);
        assert_eq!(f.verdict, Verdict::Tight, "a 2 h margin on a 95 h projection is not a verdict");

        // The central estimate is unchanged — this is about what we say, not a new number.
        assert!(!f.resets_first);
        let early_by = (reset_at * 1000 - f.exhausts_at.unwrap()) as f64 / HOUR_MS;
        assert!((early_by - 2.2).abs() < 1.5, "central crossing {early_by:.1} h before reset");

        // A quieter week does not fill it, and lands about where the live data said: ≈93%.
        assert!(f.exhausts_at_late.is_none());
        assert!((f.projected_end_util_low - 0.93).abs() < 0.03, "got {}", f.projected_end_util_low);
        // A busier one runs out with most of a day to spare.
        assert!(f.exhausts_at_early.unwrap() < f.exhausts_at.unwrap());
        assert_eq!(f.projected_end_util_high, 1.0);
    }

    #[test]
    fn a_band_that_crosses_at_every_duty_is_still_critical() {
        let rs = run(0, MIN, 121, 0.50, 0.80);
        let now = 120 * MIN;
        // 15%/active-hour with 20% left is 1.3 hours of work, and 3 hours of window to
        // spend it in: even the quiet edge of the band gets there.
        let f = forecast(&rs, FIVE_HOUR, 18_000, now, None, Duty::spread(0.7, 0.2));
        assert_eq!(f.verdict, Verdict::Crosses);
        assert!(!f.resets_first);
        assert!(f.exhausts_at_late.unwrap() >= f.exhausts_at.unwrap());
        assert!(f.exhausts_at_early.unwrap() <= f.exhausts_at.unwrap());
    }

    #[test]
    fn a_band_that_crosses_at_no_duty_says_so_without_hedging() {
        // 5.5%/active-hour cannot fill a 5-hour window even flat out.
        let rs = run(0, MIN, 121, 0.00, 0.11);
        let now = 120 * MIN;
        let f = forecast(&rs, FIVE_HOUR, 18_000, now, None, Duty::spread(0.7, 0.3));
        assert_eq!(f.verdict, Verdict::ResetsFirst);
        assert!(f.resets_first);
        assert!(f.exhausts_at.is_none() && f.exhausts_at_early.is_none());
        assert!(f.projected_end_util_low < f.projected_end_util);
        assert!(f.projected_end_util < f.projected_end_util_high);
    }

    /// A band with no width must leave every number exactly where the old single-ray
    /// projection put it — the 5-hour window still forecasts against the session it is
    /// in, and must not start hedging because the 7-day one learned to.
    #[test]
    fn a_flat_band_reproduces_the_single_ray_answer() {
        for (from, to, duty) in [(0.50, 0.80, 1.0), (0.00, 0.11, 1.0), (0.00, 0.20, 0.125)] {
            let rs = run(0, MIN, 121, from, to);
            let now = 120 * MIN;
            let f = forecast(&rs, SEVEN_DAY, 604_800, now, None, Duty::flat(duty));
            assert_eq!(f.exhausts_at, f.exhausts_at_early);
            assert_eq!(f.exhausts_at, f.exhausts_at_late);
            assert_eq!(f.projected_end_util, f.projected_end_util_low);
            assert_eq!(f.projected_end_util, f.projected_end_util_high);
            assert_ne!(f.verdict, Verdict::Tight, "no spread, so nothing to be unsure about");
            assert_eq!(f.resets_first, f.verdict == Verdict::ResetsFirst);
        }
    }

    #[test]
    fn the_band_is_ordered_and_stays_inside_what_a_duty_cycle_can_be() {
        let d = Duty::spread(0.1, 0.4);
        assert!(d.low <= d.mid && d.mid <= d.high);
        assert!(d.low >= 0.01 && d.high <= 1.0);
        assert_eq!(Duty::spread(0.9, 0.5).high, 1.0);
        assert_eq!(Duty::flat(0.0).mid, 0.01, "a zero duty would divide by nothing");
    }

    /// The backtest, as a test. Readings taken from the real window that reached 100%
    /// (one account, 7-day window, reset 2026-09-11 13:00): burn fitted during the
    /// session on 09-09/09-10 must still be predicting the truth 34 hours later.
    #[test]
    fn the_window_that_really_filled_is_predicted_within_an_hour_of_work() {
        // First time each percent appeared, in the session before the 31-hour gap.
        let steps: [(i64, f64); 7] = [
            (1_788_995_336, 0.90), (1_788_997_140, 0.91), (1_788_997_620, 0.92),
            (1_788_998_700, 0.93), (1_789_000_680, 0.94), (1_789_004_820, 0.95),
            (1_789_008_600, 0.96),
        ];
        // Dense traffic throughout, which is what the store shows: 217–543 requests/hour.
        let mut rs = Vec::new();
        for w in steps.windows(2) {
            let (t0, u0) = w[0];
            let (t1, _) = w[1];
            let mut t = t0;
            while t < t1 {
                rs.push(Reading { at_ms: t * 1000, util: Some(u0) });
                t += 20;
            }
        }
        rs.push(Reading { at_ms: steps[6].0 * 1000, util: Some(0.96) });

        let (burn, hours) = burn_from(&rs).expect("a slope");
        assert!(hours > 3.0, "fitted on {hours:.1} h of work");
        // The measured figure from the live store was 1.17–2.15%/active-hour.
        assert!((0.010..=0.025).contains(&burn), "got {:.4}/active-h", burn);

        // 4% of the window left. Actual: it took 1.8 more hours of work to reach 100%.
        let left = (1.0 - 0.96) / burn;
        assert!((left - 1.8).abs() < 1.0, "predicted {left:.2} h of work left, actual 1.8 h");
    }
}
