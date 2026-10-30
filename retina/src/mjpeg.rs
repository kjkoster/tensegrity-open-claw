//! Reading JPEG frames out of an MJPEG stream: the `multipart/x-mixed-replace` body the camera
//! sends, or a file the same body was recorded into.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn part(boundary: &str, body: &[u8]) -> Vec<u8> {
        let mut part = format!(
            "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        part.extend_from_slice(body);
        part.extend_from_slice(b"\r\n");
        part
    }

    #[test]
    fn reads_every_part_whole_however_its_bytes_look() {
        // The second body holds what looks like a boundary and a blank line, which a reader
        // scanning for markers or lines would split on.
        let first = b"\xff\xd8one\xff\xd9".to_vec();
        let second = b"\xff\xd8\r\n--myboundary\r\n\r\ntwo\xff\xd9".to_vec();
        let mut stream = part("myboundary", &first);
        stream.extend(part("myboundary", &second));
        let mut reader = &stream[..];

        assert_eq!(next_jpeg(&mut reader).unwrap(), first);
        assert_eq!(next_jpeg(&mut reader).unwrap(), second);
        assert_eq!(
            next_jpeg(&mut reader).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn a_stream_cut_short_is_an_error() {
        let stream = part("myboundary", b"\xff\xd8whole\xff\xd9");
        let mut reader = &stream[..stream.len() - 5];
        assert!(next_jpeg(&mut reader).is_err());
    }
}
