package org.mtrx.app.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.mtrx.app.model.Account
import org.mtrx.app.ui.ChoiceRow
import uniffi.mtrx.MediaQuality
import uniffi.mtrx.MediaQualityChoice
import uniffi.mtrx.mediaQualityChoices

/** How this phone sends photos and videos, Reduced or Original (docs/reducing-media.md). */
@Composable
fun MediaSection(account: Account) {
    val quality by account.mediaQuality.value.collectAsStateWithLifecycle()
    val choices = remember { mediaQualityChoices() }
    MediaQualityRow(quality, choices, onChoose = account.mediaQuality::set)
}

/** "Photos and videos": [quality], one of [choices], with what it does below it. */
@Composable
fun MediaQualityRow(quality: MediaQuality, choices: List<MediaQualityChoice>, onChoose: (MediaQuality) -> Unit) {
    ChoiceRow(
        "Photos and videos",
        quality,
        choices.map { it.quality to it.title },
        choices.firstOrNull { it.quality == quality }?.explanation,
        enabled = true,
        onChoose = onChoose,
    )
}
