// Router passwords live here and nowhere else.
//
// The iPhone app puts them in the Keychain. Android has no equivalent vault, so this is the
// nearest honest thing: one AES-GCM key held in the AndroidKeyStore — hardware-backed where
// the phone has a TEE, and never extractable either way — used to encrypt each password into
// ordinary SharedPreferences. `androidx.security`'s EncryptedSharedPreferences would do the
// same thing, but it is deprecated and unmaintained, and it is about forty lines of wrapping.

package ca.whnet.agentrouter.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class Secrets(context: Context) {

    private val prefs = context.getSharedPreferences("router-passwords", Context.MODE_PRIVATE)

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "ca.whnet.agentrouter.passwords"
        const val TRANSFORM = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        gen.init(
            KeyGenParameterSpec.Builder(
                ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                // No unlock required: the router is polled in the background, and a
                // notification has to be able to name its router before anyone unlocks.
                .setUserAuthenticationRequired(false)
                .build()
        )
        return gen.generateKey()
    }

    /** Keyed by the router's id, so renaming it or moving it to a new address keeps it. */
    fun set(password: String?, id: String) {
        if (password.isNullOrEmpty()) {
            prefs.edit().remove(id).apply()
            return
        }
        val cipher = Cipher.getInstance(TRANSFORM).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.doFinal(password.toByteArray())
        val blob = cipher.iv + sealed
        prefs.edit().putString(id, Base64.encodeToString(blob, Base64.NO_WRAP)).apply()
    }

    fun get(id: String): String? {
        val stored = prefs.getString(id, null) ?: return null
        return runCatching {
            val blob = Base64.decode(stored, Base64.NO_WRAP)
            val cipher = Cipher.getInstance(TRANSFORM).apply {
                init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
            }
            String(cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES))
        }.getOrNull()
        // Null on failure rather than a throw: a restored backup or a cleared keystore
        // leaves undecryptable bytes behind, and "ask for the password again" is the only
        // useful answer to that.
    }

    fun remove(id: String) = prefs.edit().remove(id).apply()
}
