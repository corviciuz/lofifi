#[cfg(target_os = "linux")]
pub fn silent_get_output_stream() -> eyre::Result<rodio::MixerDeviceSink, crate::player::Error> {
    use libc::freopen;
    use rodio::DeviceSinkBuilder;
    use std::ffi::CString;

    unsafe extern "C" {
        static stderr: *mut libc::FILE;
    }

    let mode = CString::new("w")?;

    let null = CString::new("/dev/null")?;

    unsafe {
        freopen(null.as_ptr(), mode.as_ptr(), stderr);
    }

    let stream = DeviceSinkBuilder::open_default_sink()?;

    let tty = CString::new("/dev/tty")?;

    unsafe {
        freopen(tty.as_ptr(), mode.as_ptr(), stderr);
    }

    Ok(stream)
}
