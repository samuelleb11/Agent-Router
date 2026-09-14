// Quota, the forecast, accounts and the dashboard's own rollups.
//
// Split from Models.kt only for size; the same rule applies — every snake_case key is
// spelled out, and anything SQLite sums is a Double.

package ca.whnet.agentrouter.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * `GET /api/quota`. Utilisation is 0–100 here (the store keeps 0–1; the endpoint scales to
 * match Anthropic's own usage API). [Window.resetsAt] is either ISO 8601 or `@<epoch>` when
 * it came from response headers rather than the API — parse both.
 */
@Serializable
data class Quota(
    @SerialName("five_hour") val fiveHour: Window? = null,
    @SerialName("seven_day") val sevenDay: Window? = null,
    val severity: String? = null,
    val source: String? = null,
    @SerialName("account_id") val accountId: String? = null,
) {
    @Serializable
    data class Window(
        val utilization: Double? = null,
        @SerialName("resets_at") val resetsAt: String? = null,
    ) {
        val fraction: Double? get() = utilization?.let { (it / 100).coerceIn(0.0, 1.0) }

        /** Epoch millis, from either spelling, or null. */
        val resetMs: Long?
            get() {
                val s = resetsAt ?: return null
                if (s.startsWith("@")) return s.drop(1).toDoubleOrNull()?.times(1000)?.toLong()
                return runCatching {
                    java.time.Instant.parse(s).toEpochMilli()
                }.getOrNull()
            }
    }
}

/** `GET /api/forecast`: every account's windows projected to their end, plus a rollup. */
@Serializable
data class Forecast(
    @SerialName("generated_at") val generatedAt: Long = 0,
    val accounts: List<ForecastAccount> = emptyList(),
    val global: Global? = null,
) {
    @Serializable
    data class ForecastAccount(
        val id: String,
        val label: String = "",
        val email: String? = null,
        val plan: String? = null,
        val active: Int = 0,
        val usage: Usage = Usage(),
        val week: Usage = Usage(),
        val status: String? = null,
        val pool: Int? = null,
        @SerialName("five_hour") val fiveHour: Window? = null,
        @SerialName("seven_day") val sevenDay: Window? = null,
    ) {
        val isActive: Boolean get() = active == 1
        val title: String get() = email ?: label
    }

    @Serializable
    data class Usage(
        val requests: Double? = null,
        @SerialName("prompt_tokens") val promptTokens: Double? = null,
        @SerialName("output_tokens") val outputTokens: Double? = null,
        val cost: Double? = null,
    )

    /**
     * One rate-limit window. Three shapes share it: absent (no reading), an expired stub,
     * and the full projection.
     */
    @Serializable
    data class Window(
        val expired: Boolean? = null,
        val used: Double? = null,
        @SerialName("reset_at") val resetAt: Long? = null,
        @SerialName("window_start") val windowStart: Long? = null,
        @SerialName("burn_per_active_hour") val burnPerActiveHour: Double? = null,
        @SerialName("active_hours_left") val activeHoursLeft: Double? = null,
        @SerialName("duty_fraction") val dutyFraction: Double? = null,
        /** The same figure with the spread either side. Absent from a router that predates it. */
        val duty: Duty? = null,
        @SerialName("exhausts_at") val exhaustsAt: Long? = null,
        @SerialName("exhausts_at_early") val exhaustsAtEarly: Long? = null,
        @SerialName("exhausts_at_late") val exhaustsAtLate: Long? = null,
        @SerialName("resets_first") val resetsFirst: Boolean? = null,
        /** `crosses`, `tight` or `resets-first`. Absent from an older router. */
        val verdict: String? = null,
        @SerialName("projected_end_util") val projectedEndUtil: Double? = null,
        @SerialName("projected_end_util_low") val projectedEndUtilLow: Double? = null,
        @SerialName("projected_end_util_high") val projectedEndUtilHigh: Double? = null,
        val basis: String? = null,
        val readings: Int? = null,
        val series: List<Point> = emptyList(),
    ) {
        val isExpired: Boolean get() = expired ?: false

        /** What the band says, falling back to the boolean an older router sends. */
        val call: String get() = verdict ?: if (exhaustsAt != null) "crosses" else "resets-first"

        /** Whether there is a spread worth drawing at all — a session window has none. */
        val hasBand: Boolean get() = duty?.let { it.low < it.high } ?: false
    }

    /**
     * How much of the clock goes to work, as a range. `mid` is the estimate; the edges are
     * the spread of the days on record, which is why a tight verdict is tight.
     */
    @Serializable
    data class Duty(val low: Double = 0.0, val mid: Double = 0.0, val high: Double = 0.0)

    @Serializable
    data class Point(val t: Double = 0.0, val u: Double = 0.0)

    @Serializable
    data class Global(
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        @SerialName("output_tokens") val outputTokens: Double = 0.0,
        val cost: Double = 0.0,
        val accounts: Int = 0,
        @SerialName("combined_headroom") val combinedHeadroom: Headroom? = null,
        @SerialName("pace_30d") val pace30d: Pace? = null,
    )

    @Serializable
    data class Headroom(
        @SerialName("active_hours_left") val activeHoursLeft: Double? = null,
        val pools: Int = 0,
        @SerialName("first_to_exhaust") val firstToExhaust: First? = null,
    )

    @Serializable
    data class First(
        @SerialName("account_id") val accountId: String = "",
        val window: String = "",
        val at: Long = 0,
        @SerialName("in_ms") val inMs: Long = 0,
    )

    @Serializable
    data class Pace(
        val cost: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val basis: String = "",
    )
}

/**
 * One row of `GET /api/accounts`. [ready] says whether the router could produce a credential
 * for it right now; [detail] says what kind, or why not.
 */
@Serializable
data class Account(
    val id: String,
    val label: String = "",
    val email: String? = null,
    @SerialName("org_name") val orgName: String? = null,
    @SerialName("subscription_type") val subscriptionType: String? = null,
    val active: Int = 0,
    val source: String? = null,
    val ready: Boolean? = null,
    val detail: String? = null,
    @SerialName("expires_at") val expiresAt: Long? = null,
    @SerialName("has_token") val hasToken: Boolean? = null,
    @SerialName("usage_key") val usageKey: Int? = null,
) {
    val isActive: Boolean get() = active == 1
    val title: String get() = email ?: label
    val hasUsageKey: Boolean get() = (usageKey ?: 0) == 1
}

@Serializable
data class ActivateResponse(val ok: Boolean = false, val active: String = "")

@Serializable data class AccountRefBody(val id: String)
@Serializable data class AccountEditBody(
    val id: String,
    val label: String? = null,
    val email: String? = null,
    val token: String? = null,
)

/** `GET /api/setup`: how Claude Code on that Mac is pointed at the router. */
@Serializable
data class SetupStatus(
    val service: Service = Service(),
    val url: String = "",
    @SerialName("global_settings_path") val globalSettingsPath: String = "",
    val managed: List<Managed> = emptyList(),
) {
    @Serializable
    data class Service(
        val registered: Boolean = false,
        val supported: Boolean = false,
        val platform: String = "",
    )

    @Serializable
    data class Managed(
        val path: String = "",
        val scope: String = "",
        val value: String? = null,
    )

    val globalRouted: Boolean get() = managed.any { it.scope == "global" }
}

@Serializable data class ServiceBody(val enable: Boolean)
@Serializable data class ConfigToggleBody(val enable: Boolean, val project: String? = null)
@Serializable data class PasswordBody(val password: String? = null, val clear: Boolean? = null)

/**
 * `GET /api/summary`, scoped the way the dashboard scopes it. Everything SQLite sums is a
 * Double; timestamps are milliseconds.
 */
@Serializable
data class Summary(
    val totals: Totals = Totals(),
    val context: List<ContextRow> = emptyList(),
    val models: List<ModelRow> = emptyList(),
    val threads: List<ThreadRow> = emptyList(),
    @SerialName("by_account") val byAccount: List<AccountSplitRow> = emptyList(),
    val timeline: Timeline = Timeline(),
    @SerialName("quota_history") val quotaHistory: List<QuotaPoint> = emptyList(),
    val latency: Latency? = null,
    @SerialName("stop_reasons") val stopReasons: List<StopReason> = emptyList(),
) {
    @Serializable
    data class Totals(
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val cost: Double = 0.0,
        @SerialName("input_tokens") val inputTokens: Double = 0.0,
        @SerialName("output_tokens") val outputTokens: Double = 0.0,
        @SerialName("thinking_tokens") val thinkingTokens: Double = 0.0,
        @SerialName("cache_read_tokens") val cacheReadTokens: Double = 0.0,
        @SerialName("cache_write_tokens") val cacheWriteTokens: Double = 0.0,
        val errors: Double = 0.0,
        @SerialName("subagent_requests") val subagentRequests: Double = 0.0,
        val sessions: Double = 0.0,
        val projects: Double = 0.0,
    )

    @Serializable
    data class ContextRow(
        val component: String = "",
        @SerialName("est_tokens") val estTokens: Double = 0.0,
        val chars: Double = 0.0,
        val requests: Double = 0.0,
    )

    @Serializable
    data class ModelRow(
        val model: String? = null,
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val cost: Double = 0.0,
        @SerialName("output_tokens") val outputTokens: Double = 0.0,
        @SerialName("cache_read_tokens") val cacheReadTokens: Double = 0.0,
        val errors: Double = 0.0,
    )

    @Serializable
    data class ThreadRow(
        val thread: String = "",
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val cost: Double = 0.0,
        val agents: Double = 0.0,
    )

    @Serializable
    data class AccountSplitRow(
        val id: String = "",
        val label: String = "",
        val active: Int = 0,
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val cost: Double = 0.0,
        val sessions: Double = 0.0,
    )

    @Serializable
    data class Timeline(
        val start: Long? = null,
        val end: Long? = null,
        @SerialName("bucket_ms") val bucketMs: Long = 0,
        val rows: List<TimelineRow> = emptyList(),
    )

    @Serializable
    data class Latency(
        @SerialName("ttfb_p50") val ttfbP50: Double? = null,
        @SerialName("ttfb_p95") val ttfbP95: Double? = null,
        @SerialName("duration_p50") val durationP50: Double? = null,
        @SerialName("duration_p95") val durationP95: Double? = null,
        @SerialName("duration_max") val durationMax: Double? = null,
    )

    @Serializable
    data class StopReason(
        @SerialName("stop_reason") val stopReason: String = "",
        val requests: Double = 0.0,
    )

    /** A rate-limit reading captured from a response header, over the range. */
    @Serializable
    data class QuotaPoint(
        val t: Double = 0.0,
        @SerialName("rl_5h") val rl5h: Double? = null,
        @SerialName("rl_7d") val rl7d: Double? = null,
    )

    @Serializable
    data class TimelineRow(
        val t: Double = 0.0,
        val requests: Double = 0.0,
        @SerialName("prompt_tokens") val promptTokens: Double = 0.0,
        val cost: Double = 0.0,
    )
}
