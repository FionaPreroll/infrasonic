// SPDX-License-Identifier: GPL-3.0-only
package io.github.fionapreroll.infrasonic

import android.Manifest
import android.graphics.Bitmap
import android.os.Build
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.services.storage.TestStorage
import io.github.fionapreroll.infrasonic.core.PlayState
import io.github.fionapreroll.infrasonic.data.Graph
import io.github.fionapreroll.infrasonic.ui.MainActivity
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement

/**
 * The app from the first start to playing an album, against a server on the host
 * (`-Pandroid.testInstrumentationRunnerArguments.server=http://10.0.2.2:4533`).
 * Saves screenshots of the main screens.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(AndroidJUnit4::class)
class AppTest {
    /** The notification permission dialog would cover the app (and take the window focus). */
    @get:Rule(order = 0)
    val notifications = TestRule { base, _ ->
        object : Statement() {
            override fun evaluate() {
                if (Build.VERSION.SDK_INT >= 33) {
                    val inst = InstrumentationRegistry.getInstrumentation()
                    inst.uiAutomation.grantRuntimePermission(inst.targetContext.packageName, Manifest.permission.POST_NOTIFICATIONS)
                }
                base.evaluate()
            }
        }
    }

    @get:Rule(order = 1)
    val compose = createAndroidComposeRule<MainActivity>()

    /** What the screen showed when a step failed. */
    @get:Rule(order = 2)
    val onFailure = object : TestWatcher() {
        override fun failed(e: Throwable?, description: Description?) {
            runCatching { screenshot("failed") }
        }
    }

    private val server: String? = InstrumentationRegistry.getArguments().getString("server")

    /** A text field by its label ("Password" is also the name of an authentication method). */
    private fun field(label: String) = compose.onNode(hasText(label) and hasSetTextAction())

    private fun screenshot(name: String) {
        runCatching { compose.waitForIdle() }
        val bmp = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot() ?: return
        TestStorage().openOutputFile("screenshots/$name.png").use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    @Test
    fun setUpServerAndPlayAnAlbum() {
        assumeTrue("no server given", server != null)
        if (Graph.servers.profiles.value.isEmpty()) {
            compose.waitUntilAtLeastOneExists(hasText("Server address"), 10_000)
            field("Server address").performTextReplacement(server!!)
            field("Name (optional)").performTextReplacement("Test server")
            field("User name").performTextReplacement("admin")
            field("Password").performTextReplacement("admin")
            compose.onNodeWithText("Test").performClick()
            compose.waitUntilAtLeastOneExists(hasText("Works:", substring = true), 20_000)
            screenshot("1-server")
            compose.onNodeWithText("Save").performClick()
        }

        compose.waitUntilAtLeastOneExists(hasText("Sine Studies"), 30_000)
        Thread.sleep(1500) // covers
        screenshot("2-albums")
        compose.onAllNodesWithText("Sine Studies")[0].performClick()
        compose.waitUntilAtLeastOneExists(hasText("Low A"), 20_000)
        compose.onAllNodesWithText("Play")[0].performClick()

        compose.waitUntil(30_000) {
            Graph.player.state.value?.state == PlayState.PLAYING && Graph.player.positionNow() > 1.0
        }
        screenshot("3-album")
        compose.onNodeWithTag("miniplayer").performClick()
        compose.waitUntilAtLeastOneExists(hasText("Now playing"), 10_000)
        screenshot("4-now-playing")
        compose.onAllNodesWithText("Now playing")[0].assertExists()

        compose.activityRule.scenario.onActivity { it.onBackPressedDispatcher.onBackPressed() }
        compose.onAllNodesWithText("More")[0].performClick()
        compose.onNodeWithText("Settings").performClick()
        compose.waitUntilAtLeastOneExists(hasText("Theme"), 10_000)
        screenshot("5-settings")
    }
}
