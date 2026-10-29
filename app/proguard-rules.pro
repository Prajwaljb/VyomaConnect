# BouncyCastle is reached via JCA reflection for certificate generation.
-keep class org.bouncycastle.jcajce.provider.** { *; }
-keep class org.bouncycastle.jce.provider.** { *; }
-dontwarn org.bouncycastle.**
-dontwarn javax.naming.**

# Room (used by WorkManager, which Glance widgets depend on) instantiates <Name>_Impl reflectively.
-keep class * extends androidx.room.RoomDatabase { <init>(); }
