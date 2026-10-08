//! Fragments: how a message too big for one frame travels.
//!
//! The relay (and every WebSocket in the path) has a message size limit, and a large reply sealed as one frame is
//! both over that limit in the worst case (JSON-escaping a file full of control characters multiplies its size) and a
//! wall that a Kill or an approval would have to wait behind. So a message is cut into pieces of at most
//! [`FRAGMENT`] bytes, each sealed as its own frame, and pieces of *different* messages may be interleaved: the
//! scheduler (`core`) picks the next piece from the highest-priority channel every time.
//!
//! Every sealed plaintext starts with one flag byte:
//!
//! | first byte | meaning | then |
//! | ---------- | ------- | ---- |
//! | `0` | the whole message | the message |
//! | `1` | a piece, more follow | message id (4 bytes, big endian), the piece |
//! | `2` | the last piece | message id, the piece |
//!
//! Pieces of one message arrive in order (the session numbers every frame), so reassembly is concatenation. The
//! receiver bounds what it will hold: a message above [`MAX_MESSAGE`], or more than [`MAX_PARTIAL`] unfinished
//! messages, is a protocol error and ends the session.

use std::collections::HashMap;

/// The most message bytes in one frame.
pub const FRAGMENT: usize = 64 * 1024;
/// The largest message a receiver will assemble.
pub const MAX_MESSAGE: usize = 8 * 1024 * 1024;
/// How many unfinished messages a receiver will hold at once.
pub const MAX_PARTIAL: usize = 8;

const WHOLE: u8 = 0;
const MORE: u8 = 1;
const LAST: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Empty,
    UnknownKind(u8),
    TooBig,
    TooManyPartial,
    Truncated,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "an empty frame"),
            Self::UnknownKind(kind) => write!(f, "unknown frame kind {kind}"),
            Self::TooBig => write!(f, "a message above the size limit"),
            Self::TooManyPartial => write!(f, "too many unfinished messages"),
            Self::Truncated => write!(f, "a piece shorter than its header"),
        }
    }
}

impl std::error::Error for FrameError {}

/// A message on its way out, handed over one piece at a time.
#[derive(Debug)]
pub struct Outgoing {
    bytes: Vec<u8>,
    offset: usize,
    id: u32,
}

impl Outgoing {
    pub fn new(bytes: Vec<u8>, id: u32) -> Self {
        Self { bytes, offset: 0, id }
    }

    /// The next plaintext to seal, and whether it was the last. Never called on a finished message.
    pub fn next_piece(&mut self) -> (Vec<u8>, bool) {
        if self.offset == 0 && self.bytes.len() <= FRAGMENT {
            self.offset = self.bytes.len();

            return ([&[WHOLE][..], &self.bytes].concat(), true);
        }

        let end = (self.offset + FRAGMENT).min(self.bytes.len());
        let last = end == self.bytes.len();
        let mut piece = Vec::with_capacity(5 + end - self.offset);

        piece.push(if last { LAST } else { MORE });
        piece.extend_from_slice(&self.id.to_be_bytes());
        piece.extend_from_slice(&self.bytes[self.offset..end]);
        self.offset = end;

        (piece, last)
    }

}

/// The receiving side: puts pieces back together.
#[derive(Debug, Default)]
pub struct Reassembler {
    partial: HashMap<u32, Vec<u8>>,
}

impl Reassembler {
    /// Takes one opened frame. `Some(message)` when it completed one.
    pub fn push(&mut self, plain: &[u8]) -> Result<Option<Vec<u8>>, FrameError> {
        let (&kind, rest) = plain.split_first().ok_or(FrameError::Empty)?;

        match kind {
            WHOLE => Ok(Some((rest.len() <= MAX_MESSAGE).then(|| rest.to_vec()).ok_or(FrameError::TooBig)?)),
            MORE | LAST => {
                if rest.len() < 4 {
                    return Err(FrameError::Truncated);
                }

                let id = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
                let piece = &rest[4..];

                if !self.partial.contains_key(&id) && self.partial.len() >= MAX_PARTIAL {
                    return Err(FrameError::TooManyPartial);
                }

                let buffer = self.partial.entry(id).or_default();

                if buffer.len() + piece.len() > MAX_MESSAGE {
                    self.partial.remove(&id);

                    return Err(FrameError::TooBig);
                }

                buffer.extend_from_slice(piece);

                if kind == LAST {
                    return Ok(self.partial.remove(&id));
                }

                Ok(None)
            }
            other => Err(FrameError::UnknownKind(other)),
        }
    }

    pub fn pending(&self) -> usize {
        self.partial.len()
    }
}

/// A message as one whole frame's plaintext (for the few places that never fragment: pairing replies).
pub fn whole(bytes: &[u8]) -> Vec<u8> {
    [&[WHOLE][..], bytes].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_pieces(bytes: &[u8], id: u32) -> Vec<Vec<u8>> {
        let mut out = Outgoing::new(bytes.to_vec(), id);
        let mut pieces = Vec::new();

        loop {
            let (piece, done) = out.next_piece();

            pieces.push(piece);

            if done {
                break;
            }
        }

        pieces
    }

    #[test]
    fn a_small_message_is_one_whole_frame() {
        let pieces = all_pieces(b"{\"type\":\"ping\"}", 1);

        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0][0], 0);
        assert_eq!(Reassembler::default().push(&pieces[0]).unwrap().unwrap(), b"{\"type\":\"ping\"}");
    }

    #[test]
    fn a_big_message_comes_back_whole_after_its_pieces() {
        let message: Vec<u8> = (0..FRAGMENT * 3 + 123).map(|n| (n % 251) as u8).collect();
        let pieces = all_pieces(&message, 7);

        assert_eq!(pieces.len(), 4);
        assert!(pieces.iter().all(|piece| piece.len() <= FRAGMENT + 5));
        assert_eq!(pieces[0][0], 1);
        assert_eq!(pieces[3][0], 2);

        let mut rx = Reassembler::default();
        let mut done = None;

        for piece in &pieces {
            done = rx.push(piece).unwrap();
        }

        assert_eq!(done.unwrap(), message);
        assert_eq!(rx.pending(), 0);
    }

    #[test]
    fn an_exactly_full_frame_is_still_whole() {
        let pieces = all_pieces(&vec![b'x'; FRAGMENT], 1);

        assert_eq!(pieces.len(), 1);
        assert_eq!(all_pieces(&vec![b'x'; FRAGMENT + 1], 1).len(), 2);
    }

    #[test]
    fn pieces_of_two_messages_can_be_interleaved() {
        let a: Vec<u8> = vec![b'a'; FRAGMENT * 2 + 1];
        let b: Vec<u8> = vec![b'b'; FRAGMENT + 1];
        let (pa, pb) = (all_pieces(&a, 1), all_pieces(&b, 2));
        let mut rx = Reassembler::default();
        let order = [&pa[0], &pb[0], &pa[1], &pb[1], &pa[2]];
        let mut finished = Vec::new();

        for piece in order {
            if let Some(message) = rx.push(piece).unwrap() {
                finished.push(message);
            }
        }

        assert_eq!(finished.len(), 2);
        assert_eq!(finished[0], b, "b finished first");
        assert_eq!(finished[1], a);
    }

    #[test]
    fn a_receiver_refuses_what_it_could_not_hold() {
        let mut rx = Reassembler::default();

        for id in 0..MAX_PARTIAL as u32 {
            assert!(rx.push(&[1, 0, 0, 0, id as u8, 1, 2, 3]).unwrap().is_none());
        }

        assert_eq!(rx.push(&[1, 0, 0, 1, 0, 1]), Err(FrameError::TooManyPartial));

        let mut big = Reassembler::default();
        let piece = [&[1_u8, 0, 0, 0, 9][..], &vec![0_u8; FRAGMENT]].concat();
        let mut error = None;

        for _ in 0..(MAX_MESSAGE / FRAGMENT + 2) {
            if let Err(e) = big.push(&piece) {
                error = Some(e);

                break;
            }
        }

        assert_eq!(error, Some(FrameError::TooBig));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        let mut rx = Reassembler::default();

        assert_eq!(rx.push(&[]), Err(FrameError::Empty));
        assert_eq!(rx.push(&[9, 1, 2]), Err(FrameError::UnknownKind(9)));
        assert_eq!(rx.push(&[1, 0, 0]), Err(FrameError::Truncated));
        assert_eq!(rx.push(&[2]), Err(FrameError::Truncated));
    }

    #[test]
    fn whole_helper_matches_the_wire_form() {
        assert_eq!(whole(b"x"), vec![0, b'x']);
    }
}
