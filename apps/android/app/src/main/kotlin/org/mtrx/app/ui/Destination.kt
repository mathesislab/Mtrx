package org.mtrx.app.ui

import android.net.Uri
import android.os.Bundle
import androidx.compose.runtime.Composable
import androidx.navigation.NamedNavArgument
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.NavOptionsBuilder
import androidx.navigation.NavType
import androidx.navigation.compose.composable
import androidx.navigation.navArgument

/** A screen of a signed-in session, with what it shows. */
sealed interface Destination {
    /** The home, its tabs. */
    data object Home : Destination

    /** Room info of [roomId]. */
    data class Info(val roomId: String) : Destination

    /** The overview of space [spaceId]. */
    data class Space(val spaceId: String) : Destination

    /** Joining a room from the directory. */
    data object Join : Destination

    /** A new room, or a [space]; in [parent], if any. */
    data class New(val space: Boolean, val parent: String?) : Destination

    data object Recovery : Destination

    data object Verification : Destination
}

/**
 * A kind of destination: its route [pattern] with its [arguments], and
 * the destination of an entry with them ([read]; null if one is missing).
 */
class Route<D : Destination> private constructor(
    val pattern: String,
    val arguments: List<NamedNavArgument>,
    val read: (Bundle) -> D?,
) {
    companion object {
        val HOME = Route("home", emptyList()) { Destination.Home }

        val INFO = Route("info/{$ROOM_ID}", listOf(text(ROOM_ID))) { args ->
            args.getString(ROOM_ID)?.let(Destination::Info)
        }

        val SPACE = Route("space/{$SPACE_ID}", listOf(text(SPACE_ID))) { args ->
            args.getString(SPACE_ID)?.let(Destination::Space)
        }

        val JOIN = Route("join", emptyList()) { Destination.Join }

        val NEW = Route(
            "new?$IS_SPACE={$IS_SPACE}&$PARENT={$PARENT}",
            listOf(
                navArgument(IS_SPACE) {
                    type = NavType.BoolType
                    defaultValue = false
                },
                navArgument(PARENT) {
                    type = NavType.StringType
                    nullable = true
                    defaultValue = null
                },
            ),
        ) { args -> Destination.New(args.getBoolean(IS_SPACE), args.getString(PARENT)) }

        val RECOVERY = Route("settings/recovery", emptyList()) { Destination.Recovery }

        val VERIFICATION = Route("settings/verification", emptyList()) { Destination.Verification }
    }
}

private const val ROOM_ID = "roomId"
private const val SPACE_ID = "spaceId"
private const val IS_SPACE = "space"
private const val PARENT = "parent"

private fun text(name: String) = navArgument(name) { type = NavType.StringType }

/** The route to this destination, its arguments encoded, as its [Route] reads it: the one place routes are written. */
fun Destination.route(): String = when (this) {
    Destination.Home -> Route.HOME.pattern
    is Destination.Info -> "info/${Uri.encode(roomId)}"
    is Destination.Space -> "space/${Uri.encode(spaceId)}"
    Destination.Join -> Route.JOIN.pattern
    is Destination.New -> "new?$IS_SPACE=$space" + parent?.let { "&$PARENT=${Uri.encode(it)}" }.orEmpty()
    Destination.Recovery -> Route.RECOVERY.pattern
    Destination.Verification -> Route.VERIFICATION.pattern
}

/** Go to [to]. */
fun NavHostController.go(to: Destination, options: NavOptionsBuilder.() -> Unit = {}) = navigate(to.route(), options)

/** Back to the entry of [route] (kept), if on the stack. */
fun NavHostController.popBackTo(route: Route<*>): Boolean = popBackStack(route.pattern, inclusive = false)

/** The screen of [route]'s destinations: [content] with the entry's. */
fun <D : Destination> NavGraphBuilder.screen(route: Route<D>, content: @Composable (D) -> Unit) {
    composable(route.pattern, route.arguments) { entry ->
        val destination = route.read(entry.arguments ?: Bundle.EMPTY) ?: return@composable
        content(destination)
    }
}
