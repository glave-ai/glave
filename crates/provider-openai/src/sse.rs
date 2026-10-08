//! A Server-Sent Events decoder: network bytes in, complete `data` payloads out.

use glave_ports::ProviderError;

/// The longest line accepted, so a broken or hostile stream cannot grow memory without bound.
const MAX_LINE: usize = 8 * 1024 * 1024;

/// Decodes a Server-Sent Events stream incrementally. Bytes may arrive split anywhere,
/// including inside a line or a multi-byte character.
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    data: Vec<String>,
}

impl SseDecoder {
    /// Feeds bytes and returns the data payloads of every event they complete.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, ProviderError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            let line = String::from_utf8(line)
                .map_err(|_| ProviderError::Protocol("the event stream is not UTF-8".into()))?;
            self.line(&line, &mut events);
        }
        if self.buffer.len() > MAX_LINE {
            return Err(ProviderError::Protocol(format!(
                "an event stream line exceeded {MAX_LINE} bytes"
            )));
        }
        Ok(events)
    }

    /// Ends the stream, returning a last event that was not followed by a blank line.
    pub(crate) fn finish(&mut self) -> Option<String> {
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let mut events = Vec::new();
            self.line(
                String::from_utf8_lossy(&rest).trim_end_matches('\r'),
                &mut events,
            );
        }
        (!self.data.is_empty()).then(|| self.take_event())
    }

    fn line(&mut self, line: &str, events: &mut Vec<String>) {
        if line.is_empty() {
            if !self.data.is_empty() {
                events.push(self.take_event());
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            self.data
                .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
        }
        // Comments (lines starting with ':') and the other fields (event, id, retry) carry
        // nothing the chat completion protocol uses.
    }

    fn take_event(&mut self) -> String {
        let event = self.data.join("\n");
        self.data.clear();
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]]) -> Vec<String> {
        let mut decoder = SseDecoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(decoder.push(chunk).unwrap());
        }
        events.extend(decoder.finish());
        events
    }

    #[test]
    fn events_split_anywhere_reassemble() {
        let stream: &[u8] = b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n\n";
        let whole = decode(&[stream]);
        assert_eq!(whole, ["{\"a\":1}", "{\"b\":2}", "[DONE]"]);
        for split in 1..stream.len() {
            let (head, tail) = stream.split_at(split);
            assert_eq!(decode(&[head, tail]), whole, "split at {split}");
        }
    }

    #[test]
    fn crlf_comments_and_multi_line_data() {
        let events =
            decode(&[b": keep-alive\r\ndata: one\r\ndata: two\r\n\r\nevent: x\ndata:three\n\n"]);
        assert_eq!(events, ["one\ntwo", "three"]);
    }

    #[test]
    fn a_split_multibyte_character_survives() {
        let stream = "data: caf\u{e9}\n\n".as_bytes();
        let cut = stream.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let (head, tail) = stream.split_at(cut);
        assert_eq!(decode(&[head, tail]), ["caf\u{e9}"]);
    }

    #[test]
    fn a_last_event_without_a_blank_line_is_kept() {
        assert_eq!(decode(&[b"data: tail"]), ["tail"]);
    }

    #[test]
    fn an_endless_line_is_refused() {
        let mut decoder = SseDecoder::default();
        let chunk = vec![b'x'; MAX_LINE + 1];
        assert!(decoder.push(&chunk).is_err());
    }

    proptest::proptest! {
        /// However the network splits a stream into chunks, it decodes to the same events,
        /// including multibyte characters cut in half.
        #[test]
        fn any_chunking_decodes_the_same_events(
            payloads in proptest::collection::vec("[a-zA-Z0-9é漢🙂{}:,\\[\\]][a-zA-Z0-9é漢🙂 {}:,]{0,30}", 0..8),
            cuts in proptest::collection::vec(proptest::prelude::any::<usize>(), 0..12),
        ) {
            let stream: String = payloads.iter().map(|payload| ["data: ", payload, "\n\n"].concat()).collect();
            let bytes = stream.as_bytes();
            let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
            points.sort_unstable();
            let mut chunks = Vec::new();
            let mut start = 0;
            for point in points {
                chunks.push(&bytes[start..point]);
                start = point;
            }
            chunks.push(&bytes[start..]);
            proptest::prop_assert_eq!(decode(&chunks), payloads);
        }

        /// Whatever bytes arrive, the decoder answers or refuses; it never panics.
        #[test]
        fn arbitrary_bytes_never_panic(
            chunks in proptest::collection::vec(proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64), 0..8),
        ) {
            let mut decoder = SseDecoder::default();
            for chunk in &chunks {
                let _ = decoder.push(chunk);
            }
            let _ = decoder.finish();
        }
    }
}
