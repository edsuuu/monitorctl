use crate::log::log;
use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};

/// Log native crashes (access violations in FFI etc.) that never reach Rust's panic hook.
pub(crate) fn crash_handler() {
    unsafe extern "system" fn filter(exception: *const EXCEPTION_POINTERS) -> i32 {
        let record = unsafe { &*(*exception).ExceptionRecord };
        log(format!("CRASH exceção 0x{:08X} em {:?}\n{}", record.ExceptionCode.0, record.ExceptionAddress, std::backtrace::Backtrace::force_capture()));
        0 // EXCEPTION_CONTINUE_SEARCH: let Windows handle/report it as usual
    }
    unsafe { SetUnhandledExceptionFilter(Some(filter)) };
}
