package org.mtrx.app.model

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network

/** Says when the device's network may be back, while started. */
interface NetworkMonitor {
    /** Call [back] each time a network becomes the default (also the one now), until [stop]; from any thread. */
    fun start(back: () -> Unit)

    fun stop()
}

/** The device's default network, as [ConnectivityManager] reports it. Started and stopped on one thread. */
class ConnectivityMonitor(context: Context) : NetworkMonitor {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private var callback: ConnectivityManager.NetworkCallback? = null

    override fun start(back: () -> Unit) {
        if (callback != null) return
        val callback = object : ConnectivityManager.NetworkCallback() {
            // On a system thread.
            override fun onAvailable(network: Network) = back()
        }
        connectivity.registerDefaultNetworkCallback(callback)
        this.callback = callback
    }

    override fun stop() {
        callback?.let(connectivity::unregisterNetworkCallback)
        callback = null
    }
}
