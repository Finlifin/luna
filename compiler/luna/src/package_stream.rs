//! Versioned, bounded stdio framing for a resident package compiler.
use crate::{
    compilation_cache::CompilationCache,
    package_cli::{self, Action},
};
use std::{
    ffi::OsString,
    io::{BufRead, Write},
};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_ARGUMENTS: usize = 4096;

/// One request per LF-terminated line. Each argument is hex-encoded native
/// bytes, separated by one ASCII space; `-` encodes an empty argument.
fn decode(frame: &[u8]) -> Result<Vec<OsString>, String> {
    if frame.is_empty() {
        return Err("empty package stream request".into());
    }
    let mut arguments = Vec::new();
    for word in frame.split(|byte| *byte == b' ') {
        if arguments.len() == MAX_ARGUMENTS {
            return Err("too many package stream arguments".into());
        }
        let bytes = if word == b"-" {
            Vec::new()
        } else {
            if word.is_empty() || word.len() % 2 != 0 {
                return Err("invalid hexadecimal argument".into());
            }
            let digit = |value: u8| match value {
                b'0'..=b'9' => Some(value - b'0'),
                b'a'..=b'f' => Some(value - b'a' + 10),
                b'A'..=b'F' => Some(value - b'A' + 10),
                _ => None,
            };
            word.chunks_exact(2)
                .map(|pair| {
                    Ok(digit(pair[0]).ok_or("invalid hexadecimal argument")? * 16
                        + digit(pair[1]).ok_or("invalid hexadecimal argument")?)
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            arguments.push(OsString::from_vec(bytes));
        }
        #[cfg(not(unix))]
        {
            arguments.push(
                String::from_utf8(bytes)
                    .map_err(|_| "this platform requires UTF-8 stream arguments")?
                    .into(),
            );
        }
    }
    Ok(arguments)
}

/// Drains oversized records without allocating their full length, so the next
/// line remains an independent request. EOF may only terminate between frames.
fn frame(input: &mut impl BufRead) -> std::io::Result<Option<Result<Vec<u8>, String>>> {
    let mut result = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if result.is_empty() && !oversized {
                None
            } else {
                Some(Err("unterminated package stream request".into()))
            });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.unwrap_or(buffer.len());
        if !oversized {
            if count > MAX_FRAME_BYTES - result.len() {
                oversized = true;
                result.clear();
            } else {
                result.extend_from_slice(&buffer[..count]);
            }
        }
        input.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(if oversized {
                Err("package stream request exceeds 1 MiB".into())
            } else {
                Ok(result)
            }));
        }
    }
}

enum Payload {
    Error(String),
    Artifact(
        std::sync::Arc<crate::compilation_cache::CachedCompilation>,
        bool,
    ),
}
impl std::fmt::Display for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(error) => f.write_str(error),
            Self::Artifact(compilation, true) => write!(
                f,
                "(compilation-result (version 1) (result {}) {})",
                compilation.artifact, compilation.inputs
            ),
            Self::Artifact(compilation, false) => compilation.artifact.fmt(f),
        }
    }
}

/// Keeps only frontend artifacts. Replies are single-line S-expressions whose
/// payload is an escaped string; semantic diagnostics may also go to stderr.
/// Request failure does not terminate the stream. Transport failure does.
pub fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    capacity: usize,
) -> Result<(), String> {
    let mut cache = CompilationCache::new(capacity);
    let mut sequence = 0u64;
    while let Some(frame) = frame(input).map_err(|error| error.to_string())? {
        sequence = sequence
            .checked_add(1)
            .ok_or("package stream sequence exhausted")?;
        let result = frame
            .and_then(|frame| decode(&frame))
            .and_then(package_cli::parse);
        let (code, reused, payload) = match result {
            Err(error) => (2, false, Payload::Error(error)),
            Ok(request) if request.action != Action::Compile || request.options.output.is_some() =>
                (2, false, Payload::Error("package stream supports HIR/NIR/attribute inspection to replies only; execution actions and --output are not accepted".into())),
            Ok(request) => match cache.compile(&request) {
                Err(error) => (1, false, Payload::Error(error)),
                Ok(outcome) => {
                    (0, outcome.reused, Payload::Artifact(outcome.compilation, request.with_inputs))
                }
            },
        };
        writeln!(output, "(package-response (version 1) (sequence {sequence}) (exit-code {code}) (reused {reused}) (payload (string {})))", nir::QuotedDisplay(&payload))
            .map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    #[test]
    fn frame_limits_drain_and_preserve_following_records() {
        let mut bytes = vec![b'a'; MAX_FRAME_BYTES + 17];
        bytes.extend_from_slice(b"\nzz\n-\n00");
        let mut output = Vec::new();
        serve(
            &mut BufReader::with_capacity(7, Cursor::new(bytes)),
            &mut output,
            1,
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        for (index, line) in lines.iter().enumerate() {
            assert!(line.contains(&format!("(sequence {})", index + 1)));
            assert!(line.contains("(exit-code 2)"));
        }
        assert!(lines[0].contains("exceeds 1 MiB"));
        assert!(lines[1].contains("invalid hexadecimal"));
        assert!(lines[3].contains("unterminated"));
    }
    #[test]
    fn argument_encoding_is_strict_and_bounded() {
        assert_eq!(
            decode(b"61622063 -").unwrap(),
            vec![OsString::from("ab c"), OsString::new()]
        );
        for invalid in [b"".as_slice(), b"6", b"gg", b"61  62", b"61\r"] {
            assert!(decode(invalid).is_err());
        }
        assert!(decode("61 ".repeat(MAX_ARGUMENTS).trim_end().as_bytes()).is_ok());
        assert!(decode("61 ".repeat(MAX_ARGUMENTS + 1).trim_end().as_bytes()).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(decode(b"61ff62").unwrap()[0].as_bytes(), b"a\xffb");
        }
    }
    #[test]
    fn response_write_failure_stops_inside_payload_before_reading_next_frame() {
        struct Limited {
            bytes: Vec<u8>,
            limit: usize,
        }
        impl Write for Limited {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.bytes.len() == self.limit {
                    return Err(std::io::Error::other("closed payload transport"));
                }
                let count = bytes.len().min(self.limit - self.bytes.len());
                self.bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let prefix = "(package-response (version 1) (sequence 1) (exit-code 2) (reused false) (payload (string \"";
        let mut output = Limited {
            bytes: vec![],
            limit: prefix.len() + 7,
        };
        let mut input = Cursor::new(b"zz\nzz\n");
        let error = serve(&mut input, &mut output, 1).unwrap_err();
        assert!(error.contains("closed payload transport"), "{error}");
        assert_eq!(output.bytes, format!("{prefix}invalid").as_bytes());
        assert_eq!(input.position(), 3);
    }

    #[test]
    fn response_flush_failure_terminates_transport() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("closed transport"))
            }
        }
        let mut input = Cursor::new(b"zz\nzz\n");
        assert!(
            serve(&mut input, &mut Broken, 1)
                .unwrap_err()
                .contains("closed transport")
        );
        assert_eq!(input.position(), 3);
    }
}
