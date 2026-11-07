package org.mtrx.app

import android.content.Intent
import android.os.Bundle
import android.view.KeyEvent
import android.view.MotionEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import org.mtrx.app.model.Account
import org.mtrx.app.model.AccountState
import org.mtrx.app.model.PushRegistration
import org.mtrx.app.signin.SignInScreen
import org.mtrx.app.signin.SignInViewModel
import org.mtrx.app.ui.MtrxTheme
import org.mtrx.app.ui.SignedInNavigation

class MainActivity : ComponentActivity() {
    private val account get() = (application as MtrxApplication).account

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // A recreated activity has handled its intent already.
        if (savedInstanceState == null) openRoomOf(intent)
        val app = application as MtrxApplication
        setContent {
            MtrxTheme {
                MtrxApp(app.account, app.push)
            }
        }
    }

    // Touches and key presses anywhere in the activity: not idle.
    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        account.input()
        return super.dispatchTouchEvent(event)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        account.input()
        return super.dispatchKeyEvent(event)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        openRoomOf(intent)
    }

    /** A tapped notification opens its room. */
    private fun openRoomOf(intent: Intent?) {
        intent?.getStringExtra(EXTRA_ROOM_ID)?.let { account.requestRoom(it) }
    }

    companion object {
        /** The room a notification opens. */
        const val EXTRA_ROOM_ID = "org.mtrx.app.ROOM_ID"
    }
}

@Composable
private fun MtrxApp(account: Account, push: PushRegistration) {
    val state by account.state.collectAsStateWithLifecycle()
    when (val current = state) {
        AccountState.Loading -> Surface(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        }
        is AccountState.SignedOut -> {
            val viewModel: SignInViewModel = viewModel { SignInViewModel(account) }
            SignInScreen(viewModel, restoreError = current.error)
        }
        // A new navigation (and new screens) for each session.
        is AccountState.SignedIn -> key(current.session) {
            SignedInNavigation(account, current.session, push)
        }
    }
}
