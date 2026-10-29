# BouncyCastle is reached via JCA reflection for certificate generation.
-keep class org.bouncycastle.jcajce.provider.** { *; }
-keep class org.bouncycastle.jce.provider.** { *; }
-dontwarn org.bouncycastle.**
-dontwarn javax.naming.**

# Room (used by WorkManager, which Glance widgets depend on) instantiates <Name>_Impl reflectively.
-keep class * extends androidx.room.RoomDatabase { <init>(); }

# WorkManager (Glance runs widget sessions through it) creates these reflectively by class name.
# Without them every widget update fails and widgets sit on the loading layout forever.
-keep class * extends androidx.work.InputMerger { <init>(); }
-keep class * extends androidx.work.ListenableWorker { <init>(android.content.Context, androidx.work.WorkerParameters); }
