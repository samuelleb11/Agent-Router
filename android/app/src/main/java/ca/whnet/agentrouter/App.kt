package ca.whnet.agentrouter

import android.app.Application
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import ca.whnet.agentrouter.data.RouterStore
import ca.whnet.agentrouter.push.Notifications
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class App : Application() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        Notifications.ensureChannel(this)

        // Whatever the data message missed: banners for prompts that are no longer waiting
        // go as soon as the app is looked at. This is the half that survives the app being
        // force-stopped, or put to sleep by the phone, while a prompt was answered elsewhere.
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                scope.launch { Notifications.reconcile(this@App, RouterStore(this@App)) }
            }
        })
    }
}
