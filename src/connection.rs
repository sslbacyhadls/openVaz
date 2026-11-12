//! Desktop socket transport alongside the existing serial port and wire protocol.
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use crate::link::{parse_car_line, CarMessage, CarPort};

const BUFFER_LIMIT: usize = 8192;

pub enum CarConnection {
    Serial(CarPort),
    Socket(SocketPort),
}

impl CarConnection {
    pub fn fd(&self) -> RawFd {
        match self {
            Self::Serial(port) => port.fd(),
            Self::Socket(port) => port.stream.as_raw_fd(),
        }
    }

    pub fn send_command(&mut self, channel: &str, value: f32) -> io::Result<()> {
        match self {
            Self::Serial(port) => port.send_command(channel, value),
            Self::Socket(port) => {
                let line = format!("выход {channel} значение {value:.2}\n");
                if port.outgoing.len() + line.len() > BUFFER_LIMIT {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "автомобиль не читает команды",
                    ));
                }
                port.outgoing.extend_from_slice(line.as_bytes());
                port.flush()
            }
        }
    }

    pub fn poll_messages(&mut self) -> io::Result<Vec<CarMessage>> {
        match self {
            Self::Serial(port) => port.poll_messages(),
            Self::Socket(port) => port.poll_messages(),
        }
    }
}

pub struct SocketPort {
    stream: UnixStream,
    incoming: Vec<u8>,
    outgoing: Vec<u8>,
}

impl SocketPort {
    fn flush(&mut self) -> io::Result<()> {
        while !self.outgoing.is_empty() {
            match self.stream.write(&self.outgoing) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.outgoing.drain(..n);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn poll_messages(&mut self) -> io::Result<Vec<CarMessage>> {
        self.flush()?;
        let mut messages = Vec::new();
        let mut buf = [0u8; 512];
        // Bound work per update so an eager peer cannot starve window events.
        for _ in 0..16 {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.incoming.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
            while let Some(end) = self.incoming.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.incoming.drain(..=end).collect();
                if let Some(message) = parse_car_line(String::from_utf8_lossy(&line).trim()) {
                    messages.push(message);
                }
            }
            if self.incoming.len() > BUFFER_LIMIT {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "слишком длинная строка автомобиля",
                ));
            }
        }
        Ok(messages)
    }
}

// A blocking connect can hang when another panel/QEMU has a full accept queue.
fn socket_is_stale(path: &Path) -> io::Result<bool> {
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // Own the descriptor immediately, including on all error paths below.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for (dst, src) in address.sun_path.iter_mut().zip(bytes) {
        *dst = *src as libc::c_char;
    }
    let result = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    };
    Ok(result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::ConnectionRefused)
}

pub struct CarListener {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl CarListener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let listener = match UnixListener::bind(path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                // Never unlink a live panel/QEMU socket or a regular file.
                let metadata = fs::symlink_metadata(path)?;
                if !metadata.file_type().is_socket() || !socket_is_stale(path)? {
                    return Err(error);
                }
                fs::remove_file(path)?;
                UnixListener::bind(path)?
            }
            Err(error) => return Err(error),
        };
        let metadata = fs::symlink_metadata(path)?;
        let result = Self {
            listener,
            path: path.to_owned(),
            identity: (metadata.dev(), metadata.ino()),
        };
        result.listener.set_nonblocking(true)?;
        Ok(result)
    }

    pub fn accept(&self) -> io::Result<CarConnection> {
        let (stream, _) = self.listener.accept()?;
        stream.set_nonblocking(true)?;
        Ok(CarConnection::Socket(SocketPort {
            stream,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }))
    }
}

impl Drop for CarListener {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| {
            metadata.file_type().is_socket() && (metadata.dev(), metadata.ino()) == self.identity
        }) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("openvaz-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn socket_round_trips_commands_fragmented_messages_and_reconnects() {
        let path = path("roundtrip");
        let listener = CarListener::bind(&path).unwrap();
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
        );
        let mut peer = UnixStream::connect(&path).unwrap();
        let mut port = listener.accept().unwrap();
        port.send_command("вентилятор", 0.75).unwrap();
        let expected = "выход вентилятор значение 0.75\n".as_bytes();
        let mut bytes = vec![0; expected.len()];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        peer.write_all("охлаждение 8".as_bytes()).unwrap();
        assert!(port.poll_messages().unwrap().is_empty());
        peer.write_all("7\nмуфта 1\n".as_bytes()).unwrap();
        assert_eq!(
            port.poll_messages().unwrap(),
            vec![CarMessage::Coolant(87), CarMessage::Clutch(true)]
        );
        drop(peer);
        assert!(port.poll_messages().is_err());
        let _peer = UnixStream::connect(&path).unwrap();
        assert!(listener.accept().is_ok());
        drop(listener);
        assert!(!path.exists());
    }

    #[test]
    fn stale_socket_is_replaced_but_active_sockets_and_files_are_preserved() {
        let path = path("ownership");
        drop(UnixListener::bind(&path).unwrap());
        let listener = CarListener::bind(&path).unwrap();
        // Fill the accept queue too: another launch must still fail without blocking.
        for _ in 0..256 {
            assert!(
                matches!(CarListener::bind(&path), Err(error) if error.kind() == io::ErrorKind::AddrInUse)
            );
        }
        assert!(path.exists());
        drop(listener);
        fs::write(&path, "keep").unwrap();
        assert!(CarListener::bind(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep");
        fs::remove_file(path).unwrap();
    }
}
