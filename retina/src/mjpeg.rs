//! Reading JPEG frames out of an MJPEG stream: the `multipart/x-mixed-replace` body the camera
//! sends.
//!
//! Each part's `Content-Length` is read and exactly that many bytes taken. Scanning for the
//! JPEG markers instead works until a frame happens to contain the marker bytes.

use std::io::{self, BufRead, ErrorKind};

/// The next frame. Boundary lines, blank lines and the part's other headers are skipped. Any
/// error means the stream is lost or out of step, and is the caller's to reconnect from.
pub fn next_jpeg(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut length = None;
    let mut line = String::new();
    let length = loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(ErrorKind::UnexpectedEof.into());
        }
        let header = line.trim();
        if header.is_empty() {
            match length {
                Some(length) => break length,
                None => continue,
            }
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            let value = value.trim();
            length = Some(value.parse::<usize>().map_err(|_| {
                io::Error::new(ErrorKind::InvalidData, format!("Content-Length {value}"))
            })?);
        }
    };
    let mut jpeg = vec![0; length];
    reader.read_exact(&mut jpeg)?;
    Ok(jpeg)
}
