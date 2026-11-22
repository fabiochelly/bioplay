# libmpv rappelle ces méthodes par JNI, depuis son fil d'événements.
-keep class dev.jdtech.mpv.** { *; }
-keepclassmembers class app.bioplay.Core { native <methods>; }

# Partages réseau (smbj, dcerpc) : classes Java absentes d'Android, jamais utilisées ici
# (Kerberos, expressions EL du bus d'événements, RMI dans un chemin d'erreur).
-dontwarn java.rmi.**
-dontwarn javax.el.**
-dontwarn org.ietf.jgss.**
-dontwarn org.slf4j.impl.**
# Bus d'événements de smbj : abonnés trouvés par réflexion (@Handler).
-keep class com.hierynomus.** { *; }
-keep class net.engio.mbassy.** { *; }
-keep class com.rapid7.client.dcerpc.** { *; }
