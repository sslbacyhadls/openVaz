//! Сторожевой таймер Linux. Если главный цикл замирает, плата перезагружается.
//! На компьютере без `/dev/watchdog` панель просто работает дальше.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::fd::AsRawFd;

/// `WDIOC_SETTIMEOUT`: `_IOWR('W', 6, int)`.
pub const SET_TIMEOUT: libc::Ioctl = 0xC004_5706;

pub struct Watchdog {
    file: std::fs::File,
}

impl Watchdog {
    pub fn open(seconds: i32) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).open("/dev/watchdog")?;
        let mut timeout = seconds;
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), SET_TIMEOUT, &mut timeout) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }

    pub fn pet(&mut self) {
        let _ = self.file.write_all(&[0]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_ioctl_matches_the_linux_header() {
        let dir = 3u32 << 30;
        let size = 4u32 << 16;
        let kind = (b'W' as u32) << 8;
        let number = 6u32;
        assert_eq!(SET_TIMEOUT, (dir | size | kind | number) as libc::Ioctl);
    }
}
