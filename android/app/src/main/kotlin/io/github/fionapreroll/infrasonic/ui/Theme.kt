// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import io.github.fionapreroll.infrasonic.data.ThemeMode

private val Violet = Color(0xFF6A3DF0)
private val VioletLight = Color(0xFF9B7BFF)

private val Light = lightColorScheme(primary = Violet, secondary = Color(0xFF5C5470), tertiary = Color(0xFF8A4F7D))
private val Dark = darkColorScheme(primary = VioletLight, secondary = Color(0xFFCAC2DB), tertiary = Color(0xFFF0B6DE))

/** Pure black backgrounds for OLED screens. */
private fun ColorScheme.black() = copy(
    background = Color.Black,
    surface = Color.Black,
    surfaceContainerLowest = Color.Black,
    surfaceContainerLow = Color(0xFF0A0A0A),
    surfaceContainer = Color(0xFF111111),
    surfaceContainerHigh = Color(0xFF181818),
    surfaceContainerHighest = Color(0xFF202020),
)

@Composable
fun InfrasonicTheme(mode: ThemeMode, dynamic: Boolean, content: @Composable () -> Unit) {
    val dark = when (mode) {
        ThemeMode.System -> isSystemInDarkTheme()
        ThemeMode.Light -> false
        ThemeMode.Dark, ThemeMode.Black -> true
    }
    val context = LocalContext.current
    val base = when {
        dynamic && Build.VERSION.SDK_INT >= 31 -> if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> Dark
        else -> Light
    }
    MaterialTheme(colorScheme = if (mode == ThemeMode.Black) base.black() else base, content = content)
}
