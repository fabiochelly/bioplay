# libmpv rappelle ces méthodes par JNI, depuis son fil d'événements.
-keep class dev.jdtech.mpv.** { *; }
-keepclassmembers class app.bioplay.Core { native <methods>; }
