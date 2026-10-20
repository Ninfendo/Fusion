//! Reading JSON lines.

use std::io::{BufRead, BufReader, Read};

use fusion_hub::decode;
use serde::de::DeserializeOwned;

/// Reads lines from `source` until it ends or fails, and passes each line,
/// decoded, to `each`. Stops early when `each` returns false.
pub(crate) fn read_lines<T: DeserializeOwned>(
    source: impl Read,
    mut each: impl FnMut(Result<T, serde_json::Error>) -> bool,
) {
    let mut reader = BufReader::new(source);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if !each(decode(&line)) => return,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(input: &str, stop_after: usize) -> Vec<Result<u32, String>> {
        let mut seen = Vec::new();
        read_lines(input.as_bytes(), |line: Result<u32, _>| {
            seen.push(line.map_err(|e| e.to_string()));
            seen.len() < stop_after
        });
        seen
    }

    #[test]
    fn every_line_is_decoded_until_the_end() {
        let seen = read("1\nnope\n3", 10);
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[0], Ok(1));
        assert!(seen[1].is_err());
        assert_eq!(seen[2], Ok(3), "the last line needs no newline");
    }

    #[test]
    fn reading_stops_when_asked() {
        assert_eq!(read("1\n2\n3\n", 2), [Ok(1), Ok(2)]);
    }

    #[test]
    fn a_failed_read_ends_reading() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("gone"))
            }
        }
        let mut calls = 0;
        read_lines(Broken, |_: Result<u32, _>| {
            calls += 1;
            true
        });
        assert_eq!(calls, 0);
    }
}
