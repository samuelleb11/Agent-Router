// One router, one password. Plain HTTP on the LAN — the router's own trust model — or
// HTTPS to a hosted router; the base URL says which, and nothing here depends on it.

package ca.whnet.agentrouter.api

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.io.IOException
import java.net.URLEncoder
import java.util.concurrent.TimeUnit
import kotlin.coroutines.coroutineContext

/** Why a call did not produce an answer. Each case carries a sentence a person can act on. */
sealed class RouterFailure(message: String) : Exception(message) {
    /** The router was never reached. */
    class Unreachable(val why: String) : RouterFailure("Can't reach the router — $why")

    /** 401: the router has a password and this is not it. */
    data object Unauthorized : RouterFailure("Wrong password for this router")

    /**
     * 403 with the router's own message: nobody has set a password on it yet. The message
     * is for a router on a Mac; [HOSTED] says it for a hosted one, wherever
     * [ca.whnet.agentrouter.data.RouterState.describe] is told the router is.
     */
    data object NoPasswordSet :
        RouterFailure("This router has no password yet — set one on its dashboard, on the Mac") {
        /** A hosted router's password is never set from a dashboard: only its deployment sets it. */
        const val HOSTED = "This router has no password yet — a hosted router's password is set by its deployment"
    }

    /**
     * An error status that says why in the router's own words — `error`, or the live
     * endpoints' `reason` — which are the message as they are. A refused change answers this
     * way: a password too short, a key that does not sign, a sign-in already running.
     */
    class Refused(val code: Int, val words: String) : RouterFailure(words)

    /** An error status with nothing to say for itself: a guard's plain text, a proxy's page. */
    class Status(val code: Int, val body: String) :
        RouterFailure("Router returned $code" + if (body.isEmpty()) "" else ": $body")

    class Decoding(val why: String) : RouterFailure("Unexpected reply from the router — $why")
}

/** Lenient on purpose: the router adds fields faster than this app learns about them. */
val routerJson = Json {
    ignoreUnknownKeys = true
    explicitNulls = false
    coerceInputValues = true
}

class RouterClient(baseUrl: String, private val password: String?) {

    private val base = baseUrl.trimEnd('/')

    companion object {
        /** Any username satisfies the router; only the password counts. */
        fun authorization(password: String): String {
            val raw = "app:$password".toByteArray()
            return "Basic " + android.util.Base64.encodeToString(raw, android.util.Base64.NO_WRAP)
        }

        private val calls = OkHttpClient.Builder()
            .connectTimeout(6, TimeUnit.SECONDS)
            .readTimeout(12, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .build()

        /**
         * For event streams, which live as long as the screen that opened them. The router
         * sends a status line at least every few seconds while quiet, so a 45 s read timeout
         * only fires when something is actually wrong.
         */
        private val streams = OkHttpClient.Builder()
            .connectTimeout(6, TimeUnit.SECONDS)
            .readTimeout(45, TimeUnit.SECONDS)
            .retryOnConnectionFailure(false)
            .build()

        /**
         * The live endpoints refuse with `{"ok":false,"reason":…}` and the rest with
         * `{"error":…}` (the wrapper switches send both); either is better than the raw body.
         */
        fun reasonIn(body: String): String? = runCatching {
            val o = routerJson.parseToJsonElement(body) as? JsonObject ?: return null
            (o["reason"] ?: o["error"])?.jsonPrimitive?.content
        }.getOrNull()

        private val JSON = "application/json".toMediaTypeOrNull()
    }

    private fun request(path: String, method: String, body: String?): Request {
        val b = Request.Builder().url(base + path).header("accept", "application/json")
        if (!password.isNullOrEmpty()) b.header("authorization", authorization(password))
        if (method == "GET") b.get() else b.post((body ?: "{}").toRequestBody(JSON))
        return b.build()
    }

    /** The body text of a call, with every failure already turned into a [RouterFailure]. */
    private fun send(req: Request): String {
        val response = try {
            calls.newCall(req).execute()
        } catch (e: IOException) {
            throw RouterFailure.Unreachable(e.message ?: "no route")
        }
        response.use {
            val body = it.body?.string().orEmpty()
            val trimmed = body.trim()
            if (it.isSuccessful) return body
            // A refusal arrives as an error status with its reason inside — a router from
            // before that answered the same refusal as a 200 carrying `error`, which is why
            // [OkReply.problem] still looks there. Either way the screen shows those words.
            val words = reasonIn(body)?.takeIf { w -> w.isNotEmpty() }
            when {
                it.code == 401 -> throw RouterFailure.Unauthorized
                it.code == 403 && trimmed.contains("no password set") -> throw RouterFailure.NoPasswordSet
                words != null -> throw RouterFailure.Refused(it.code, words)
                else -> throw RouterFailure.Status(it.code, trimmed.take(160))
            }
        }
    }

    suspend fun rawGet(path: String): String = withContext(Dispatchers.IO) {
        coroutineContext.ensureActive()
        send(request(path, "GET", null))
    }

    suspend fun rawPost(path: String, json: String): String = withContext(Dispatchers.IO) {
        coroutineContext.ensureActive()
        send(request(path, "POST", json))
    }

    /** `get` with a query string; null and empty values are left out, as the dashboard does. */
    fun query(path: String, params: Map<String, String?>): String {
        val kept = params.filterValues { !it.isNullOrEmpty() }.toSortedMap()
        if (kept.isEmpty()) return path
        val q = kept.entries.joinToString("&") { (k, v) ->
            URLEncoder.encode(k, "UTF-8") + "=" + URLEncoder.encode(v, "UTF-8")
        }
        return "$path?$q"
    }

    /**
     * Server-sent events from [path], until the connection ends or the collector stops.
     * Bytes are split into lines here rather than by a line reader — see [SseLineSplitter]
     * for why that is load-bearing and not a detail.
     *
     * The read blocks inside the socket, where coroutine cancellation cannot reach it, so
     * a collector that stops cancels the call itself. Without that, a stream abandoned on
     * a link that died quietly — the Mac asleep, the phone off the LAN — held its reader
     * until the 45 s read timeout, and the feed's retry waited behind it the whole time.
     */
    fun events(path: String): Flow<SseEvent> = callbackFlow {
        val b = Request.Builder().url(base + path).header("accept", "text/event-stream")
        if (!password.isNullOrEmpty()) b.header("authorization", authorization(password))
        val call = streams.newCall(b.get().build())
        launch(Dispatchers.IO) {
            val response = try {
                call.execute()
            } catch (e: IOException) {
                ensureActive()
                throw RouterFailure.Unreachable(e.message ?: "no route")
            }
            try {
                response.use {
                    when {
                        it.isSuccessful -> Unit
                        it.code == 401 -> throw RouterFailure.Unauthorized
                        it.code == 403 -> throw RouterFailure.NoPasswordSet
                        else -> throw RouterFailure.Status(it.code, "")
                    }
                    val source = it.body?.source() ?: throw RouterFailure.Unreachable("no body")
                    val splitter = SseLineSplitter()
                    val parser = SseParser()
                    while (!source.exhausted()) {
                        ensureActive()
                        val line = splitter.feed(source.readByte().toInt() and 0xff) ?: continue
                        send(parser.feed(line) ?: continue)
                    }
                }
            } catch (e: IOException) {
                // Cancelling the call is how a collector that stopped closes the socket, and
                // the blocked read reports that as an IOException. That is the stop working,
                // not the router failing, so it ends as the cancellation it is; a real
                // failure mid-stream goes to the collector as it always did.
                ensureActive()
                throw e
            }
            // The router ended the stream: the flow ends with it, normally.
            channel.close()
        }
        awaitClose { call.cancel() }
    }
}

/** Decode a reply, turning a shape mismatch into something the UI can show. */
inline fun <reified T> decodeReply(body: String): T = try {
    routerJson.decodeFromString<T>(body)
} catch (e: Exception) {
    throw RouterFailure.Decoding(e.message?.take(160) ?: "could not read it")
}

suspend inline fun <reified T> RouterClient.get(path: String): T = decodeReply(rawGet(path))

suspend inline fun <reified B, reified T> RouterClient.post(path: String, body: B): T =
    decodeReply(rawPost(path, routerJson.encodeToString(body)))

suspend inline fun <reified T> RouterClient.postEmpty(path: String): T =
    decodeReply(rawPost(path, "{}"))
