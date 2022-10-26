use nix::unistd;
use std::io::Write;
use std::os::unix::io::RawFd;

pub struct FdFile {
    fd: RawFd,
}

impl FdFile {
    pub fn new(fd: RawFd) -> FdFile {
        FdFile { fd }
    }
}

impl Write for FdFile {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = unistd::write(self.fd, buf).expect("failed to write");
        Ok(len)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        unistd::fsync(self.fd).ok();
        Ok(())
    }
}
