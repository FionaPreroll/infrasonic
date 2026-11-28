// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic.ui

import android.Manifest
import android.graphics.Bitmap
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.ParcelFileDescriptor
import android.view.Display
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.core.content.getSystemService
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.children
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.services.storage.TestStorage
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TestRule
import org.junit.runner.RunWith
import org.junit.runners.model.Statement

/** The emulator's "Display cutout" developer option: a tall notch at the top. */
private const val TALL_NOTCH = "com.android.internal.display.cutout.emulation.tall"

/** The visualizer fills the whole screen of a phone with a notch, and its texts and buttons keep clear of the notch. */
@SdkSuppress(minSdkVersion = 30)
@RunWith(AndroidJUnit4::class)
class VisualizerNotchTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    /** A notch while the test runs (before the app starts, which would restart for it otherwise). */
    @get:Rule(order = 0)
    val notch = TestRule { base, _ ->
        object : Statement() {
            override fun evaluate() {
                // The notification permission dialog would take the window focus.
                if (Build.VERSION.SDK_INT >= 33) {
                    instrumentation.uiAutomation.grantRuntimePermission(
                        instrumentation.targetContext.packageName,
                        Manifest.permission.POST_NOTIFICATIONS,
                    )
                }
                shell("cmd overlay enable $TALL_NOTCH")
                try {
                    waitFor("the notch") { hasNotch() }
                    base.evaluate()
                } finally {
                    shell("cmd overlay disable $TALL_NOTCH")
                    waitFor("the notch to go") { !hasNotch() }
                }
            }
        }
    }

    @get:Rule(order = 1)
    val compose = createAndroidComposeRule<MainActivity>()

    private fun shell(command: String) {
        ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { it.readBytes() }
    }

    private fun hasNotch(): Boolean {
        val displays = instrumentation.targetContext.getSystemService<DisplayManager>()
        return displays?.getDisplay(Display.DEFAULT_DISPLAY)?.cutout != null
    }

    private fun waitFor(what: String, condition: () -> Boolean) {
        val until = System.currentTimeMillis() + 30_000
        while (!condition()) {
            check(System.currentTimeMillis() < until) { "no $what after 30 s" }
            Thread.sleep(50)
        }
    }

    private fun View.surface(): SurfaceView? = when (this) {
        is SurfaceView -> this
        is ViewGroup -> children.firstNotNullOfOrNull { it.surface() }
        else -> null
    }

    /** The bottom of the notch, once the visualizer's surface covers the whole screen and the status bar is hidden. */
    private fun notchBesideSurface(): Int? {
        var notch: Int? = null
        compose.activityRule.scenario.onActivity {
            val root = it.window.decorView
            val surface = root.surface() ?: return@onActivity
            val at = IntArray(2).also(surface::getLocationInWindow)
            val insets = ViewCompat.getRootWindowInsets(root) ?: return@onActivity
            if (at[1] == 0 && surface.height == root.height && !insets.isVisible(WindowInsetsCompat.Type.statusBars())) {
                notch = insets.getInsets(WindowInsetsCompat.Type.displayCutout()).top
            }
        }
        return notch
    }

    @Test
    fun fillsTheScreenBesideTheNotch() {
        openRoute.tryEmit(Routes.VISUALIZER)
        compose.waitUntil("the visualizer fills the screen", 30_000) { notchBesideSurface() != null }
        val notch = checkNotNull(notchBesideSurface())
        assertTrue("the app sees no notch", notch > 0)
        // The controls hide 4 s after a touch; the clock stands still from here on.
        compose.mainClock.autoAdvance = false
        compose.onRoot().performTouchInput { click() }
        compose.mainClock.advanceTimeBy(500)
        val back = compose.onNodeWithContentDescription("Back").fetchSemanticsNode().boundsInWindow
        assertTrue("back button at ${back.top} px, the notch ends at $notch px", back.top >= notch)

        val bmp = instrumentation.uiAutomation.takeScreenshot() ?: return
        TestStorage().openOutputFile("screenshots/visualizer-notch.png").use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
