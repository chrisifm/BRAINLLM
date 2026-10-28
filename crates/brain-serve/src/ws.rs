//! Minimal WebSocket (RFC 6455): handshake and server → client text frames.

use crate::sha1::{base64, sha1};
use std::io::{self, Read, Write};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// `Sec-WebSocket-Accept` value for a `Sec-WebSocket-Key`.
pub fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{}{GUID}", key.trim()).as_bytes()))
}

pub fn handshake_response(key: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    )
}

/// Unmasked text frame (the server never masks).
pub fn text_frame(payload: &str) -> Vec<u8> {
    let p = payload.as_bytes();
    let mut f = Vec::with_capacity(p.len() + 10);
    f.push(0x81);
    match p.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= u16::MAX as usize => {
            f.push(126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(p);
    f
}

/// Reads client frames until a close or error; answers pings and hands each text frame to `on_text`.
pub fn drain_client<S: Read + Write>(s: &mut S, on_text: &mut dyn FnMut(String)) -> io::Result<()> {
    loop {
        let mut hdr = [0u8; 2];
        s.read_exact(&mut hdr)?;
        let opcode = hdr[0] & 0x0f;
        let masked = hdr[1] & 0x80 != 0;
        let mut len = (hdr[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            s.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            s.read_exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        if len > 1 << 20 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
        }
        let mut mask = [0u8; 4];
        if masked {
            s.read_exact(&mut mask)?;
        }
        let mut data = vec![0u8; len as usize];
        s.read_exact(&mut data)?;
        if masked {
            for (i, b) in data.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match opcode {
            0x8 => {
                let _ = s.write_all(&[0x88, 0]);
                return Ok(());
            }
            0x1 => on_text(String::from_utf8_lossy(&data).into_owned()),
            0x9 => {
                let mut pong = vec![0x8a, data.len() as u8];
                pong.extend_from_slice(&data);
                s.write_all(&pong)?;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6455_accept_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frame_lengths() {
        assert_eq!(text_frame("hi"), vec![0x81, 2, b'h', b'i']);
        let big = "x".repeat(300);
        let f = text_frame(&big);
        assert_eq!(&f[..4], &[0x81, 126, 0x01, 0x2c]);
        assert_eq!(f.len(), 304);
    }
}
