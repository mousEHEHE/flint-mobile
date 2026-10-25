# Not applied while isMinifyEnabled is false in build.gradle.kts.

# From gpui-mobile's example, whose manifest names NativeActivity directly.
-keep class android.app.NativeActivity { *; }
