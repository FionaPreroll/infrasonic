#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Regenerate android/gradle/verification-metadata.xml after a dependency or plugin changed:
# runs every build CI runs (both flavors, lint, unit and device test APKs, the bundle) and
# records the SHA-256 of everything Gradle downloads. Check the diff before committing it: new
# entries should be exactly the artifacts you changed.
#
#   scripts/gradle-verification.sh [extra Gradle arguments]
#
# Needs what an Android build needs (scripts/android-sdk.sh). Tasks that fail for lack of a
# device or servers do not matter here.
set -euo pipefail
cd "$(dirname "$0")/.."
init=$(mktemp --suffix=.init.gradle.kts)
trap 'rm -f "$init"' EXIT
cat > "$init" <<'KTS'
// Also what only CI resolves (androidTestUtil on the emulator and the like).
allprojects {
    tasks.register("resolveAllConfigurations") {
        notCompatibleWithConfigurationCache("resolves configurations at execution")
        doLast {
            project.configurations.filter { it.isCanBeResolved }.forEach { c ->
                try { c.resolve() } catch (e: Exception) { logger.info("skip ${c.name}: ${e.message}") }
            }
        }
    }
}
KTS
android/gradlew -p android -I "$init" --write-verification-metadata sha256 --continue \
  --no-configuration-cache --console=plain "$@" \
  :app:assembleGithubDebug :app:assembleGithubDebugAndroidTest :app:lintGithubDebug :app:lintStoreDebug \
  :app:testGithubDebugUnitTest :app:assembleGithubRelease :app:assembleStoreRelease \
  :app:bundleStoreRelease :app:resolveAllConfigurations || true
git diff --stat -- android/gradle/verification-metadata.xml
