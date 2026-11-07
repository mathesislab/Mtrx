package org.mtrx.app

import android.app.Application
import android.os.Build
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import java.io.File
import java.util.Locale
import kotlinx.coroutines.MainScope
import org.mtrx.app.model.Account
import org.mtrx.app.model.AccountState
import org.mtrx.app.model.AesGcmSealer
import org.mtrx.app.model.DataStorePush
import org.mtrx.app.model.DataStoreSections
import org.mtrx.app.model.DataStoreRoomSort
import org.mtrx.app.model.DataStoreSelectedSpace
import org.mtrx.app.model.DataStoreSharePresence
import org.mtrx.app.model.FileSecretBackend
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.model.PusherDevice
import org.mtrx.app.model.pushPreferences
import org.mtrx.app.model.viewPreferences
import org.mtrx.app.push.HttpGatewayDiscovery
import org.mtrx.app.push.MessageNotifications
import org.mtrx.app.push.PushHandler
import org.mtrx.app.push.UnifiedPushDistributors
import org.mtrx.ffi.AndroidPlatform
import uniffi.mtrx.Mtrx

/**
 * The app's process: the library, the account, push registration and the
 * push handler. Also runs first when the push service wakes the process.
 */
class MtrxApplication : Application() {
    lateinit var account: Account
        private set

    lateinit var push: PushRegistration
        private set

    lateinit var notifications: MessageNotifications
        private set

    lateinit var pushHandler: PushHandler
        private set

    override fun onCreate() {
        super.onCreate()
        // Before any other call into the library: its TLS verifier uses Android's.
        AndroidPlatform.initialize(this)
        val secrets = FileSecretBackend(File(noBackupFilesDir, "secrets"), AesGcmSealer.keystore())
        val mtrx = Mtrx(filesDir.absolutePath, cacheDir.absolutePath, secrets)
        val scope = MainScope()
        push = PushRegistration(
            UnifiedPushDistributors(this),
            DataStorePush(pushPreferences),
            HttpGatewayDiscovery,
            scope,
        ) { PusherDevice(Build.MODEL, Locale.getDefault().toLanguageTag()) }
        // A sender's avatar only while the app's session is open.
        notifications = MessageNotifications(this) { mxcUri ->
            (account.state.value as? AccountState.SignedIn)?.session?.images?.avatar(mxcUri, AVATAR_PX)
        }
        notifications.createChannel()
        account = Account(
            mtrx,
            scope,
            DataStoreSelectedSpace(viewPreferences),
            DataStoreRoomSort(viewPreferences),
            push,
            notifications,
            DataStoreSharePresence(viewPreferences),
            DataStoreSections(viewPreferences),
        )
        pushHandler = PushHandler(
            mtrx,
            notifications,
            foregroundRoom = account::foregroundRoom,
            onSignedOut = { push.signedOut() },
        )
        account.start()
        // Sync, and our presence, while the app is in the foreground.
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                account.setForeground(true)
            }

            override fun onStop(owner: LifecycleOwner) {
                account.setForeground(false)
            }
        })
    }

    private companion object {
        /** The sender's picture in a notification. */
        const val AVATAR_PX = 96
    }
}
