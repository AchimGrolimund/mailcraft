//! An SMTP server that answers with any bytes, through a whole send.
#![no_main]

use std::io::{Cursor, Read, Write};

/// Reads the server's bytes; what the client writes goes nowhere.
struct Server(Cursor<Vec<u8>>);

impl Read for Server {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Server {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(mut smtp) = mailcraft_proto::smtp::Smtp::connect(Server(Cursor::new(data.to_vec()))) else { return };
    let _ = smtp.hello("localhost");
    let _ = smtp.auth("ada@example.org", "hunter2");
    let _ = smtp.auth_xoauth2("ada@example.org", "tok.en-123");
    let _ = smtp.send("ada@example.org", &["bob@example.org"], b"Subject: x\r\n\r\n.\r\n");
    let _ = smtp.quit();
});
