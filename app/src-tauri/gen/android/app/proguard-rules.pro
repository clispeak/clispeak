# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# Classes the Rust side reaches by name through JNI.
#
# `voicecast-engine` looks these up with `find_class("com/voicecast/app/…")`
# and calls their members with `call_static_method`. R8 sees no Kotlin or Java
# caller for any of them, so in a release build it renames or deletes them and
# the app dies on launch:
#
#   java.lang.NoSuchMethodError: no static method
#   "Lcom/voicecast/app/Speech;.setVoice(Ljava/lang/String;)Z"
#
# Only the release build minifies, and every real test this project has run on
# a phone was a debug build — so the crash lived only in the artefact meant for
# other people. See issue #41.
-keep class com.voicecast.app.Speech { *; }
-keep class com.voicecast.app.Invites { *; }
-keep class com.voicecast.app.Battery { *; }

# The service the node runs in, named in the manifest and started from native
# code rather than from Kotlin.
-keep class com.voicecast.app.NodeService { *; }
