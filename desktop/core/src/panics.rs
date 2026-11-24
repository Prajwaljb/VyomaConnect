//! Panics into the log. Windows' agent has no console and the Mac's runs under launchd, so a
//! thread that panicked (media, toasts, the cable's watch, a cast) just stopped, leaving its
//! feature dead with nothing in the log to say why.

/// Logs every panic (thread, place, message) through `tracing`, then does what Rust does anyway.
/// Call once, after the log is set up.
pub fn log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let payload = info.payload();
        let what = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        tracing::error!("panic in thread {} at {place}: {what}", thread.name().unwrap_or("unnamed"));
        default(info);
    }));
}
