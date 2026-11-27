# SPDX-License-Identifier: GPL-3.0-only
# JNA finds classes, fields and callbacks by reflection; the UniFFI bindings use it.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class io.github.fionapreroll.infrasonic.core.** { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
