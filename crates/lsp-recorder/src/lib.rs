//! LSP message framing, shared by `lsp-recorder` and `lsp-replay`.

use std::io::{BufRead, Write};

use anyhow::{Context, Result, bail};

/// Reads one message. Returns the header and the body, or `None` at the end of the stream.
pub fn read_message(reader: &mut impl BufRead) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut header = Vec::new();
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            if header.is_empty() {
                return Ok(None);
            }
            bail!("end of stream in a message header");
        }
        header.extend_from_slice(line.as_bytes());
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let mut body = vec![0; length.context("no Content-Length header")?];
    reader.read_exact(&mut body)?;
    Ok(Some((header, body)))
}

pub fn write_message(writer: &mut impl Write, body: &[u8]) -> Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()?;
    Ok(())
}
