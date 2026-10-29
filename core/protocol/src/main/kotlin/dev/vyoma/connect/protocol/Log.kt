package dev.vyoma.connect.protocol

/** Minimal logging seam: the Android app routes this to logcat, tests to stdout. */
object VLog {
    enum class Level { DEBUG, INFO, WARN, ERROR }

    fun interface Sink {
        fun log(level: Level, tag: String, message: String, error: Throwable?)
    }

    @Volatile
    var sink: Sink = Sink { level, tag, message, error ->
        println("${level.name[0]}/$tag: $message" + (error?.let { " — $it" } ?: ""))
    }

    fun d(tag: String, message: String) = sink.log(Level.DEBUG, tag, message, null)
    fun i(tag: String, message: String) = sink.log(Level.INFO, tag, message, null)
    fun w(tag: String, message: String, error: Throwable? = null) = sink.log(Level.WARN, tag, message, error)
    fun e(tag: String, message: String, error: Throwable? = null) = sink.log(Level.ERROR, tag, message, error)
}
