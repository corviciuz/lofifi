#[cfg(target_os = "linux")]
pub fn silent_get_output_stream() -> eyre::Result<rodio::OutputStream, crate::player::Error> {
    use libc::freopen;
    use rodio::OutputStreamBuilder;
    use std::ffi::CString;

    extern "C" {
        static stderr: *mut libc::FILE;
    }

    let mode = CString::new("w")?;

    let null = CString::new("/dev/null")?;

    unsafe {
        freopen(null.as_ptr(), mode.as_ptr(), stderr);
    }

    let stream = OutputStreamBuilder::open_default_stream()?;

    let tty = CString::new("/dev/tty")?;

    unsafe {
        freopen(tty.as_ptr(), mode.as_ptr(), stderr);
    }

    Ok(stream)
}
