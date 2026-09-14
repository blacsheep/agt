//! Translate local process shutdown into the same cancellation path used by the scheduler.
use agt_core::CancellationToken;
#[cfg(unix)]
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(unix)]
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub(crate) fn cancellation() -> CancellationToken {
    let token = CancellationToken::new();
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
            libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
        }
        let child = token.clone();
        std::thread::spawn(move || {
            while !STOP.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            child.cancel();
        });
    }
    token
}
