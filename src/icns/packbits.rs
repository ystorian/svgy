//! PackBits-style RLE used by the icns ARGB (`ic04`/`ic05`) chunks.
//!
//! Ported verbatim from `createicns.c` (`RleEncodeChannel`): control bytes in `0x00..=0x7F` mean
//! "the next n+1 bytes are literals" (1..=128); control bytes in `0x80..=0xFF` mean "the next byte
//! repeats n-125 times" (3..=130).

/// RLE-encode a single 8-bit channel.
pub fn rle_encode_channel(input: &[u8]) -> Vec<u8> {
	let n = input.len();
	let mut out = Vec::with_capacity(n + n / 128 + 16);
	let mut i = 0;
	while i < n {
		// Length of the run of identical bytes starting at `i` (capped at 130).
		let mut run = 1;
		while run < 130 && i + run < n && input[i + run] == input[i] {
			run += 1;
		}

		if run >= 3 {
			// The loop above caps `run` at 130, so the control byte fits.
			out.push(u8::try_from(run + 125).expect("run length is at most 130"));
			out.push(input[i]);
			i += run;
		} else {
			// Emit a literal run, breaking before any 3-in-a-row (a future RLE run).
			let mut lit_end = i + 1;
			while lit_end - i < 128 && lit_end < n {
				if lit_end + 2 < n
					&& input[lit_end] == input[lit_end + 1]
					&& input[lit_end + 1] == input[lit_end + 2]
				{
					break;
				}
				lit_end += 1;
			}
			let lit = lit_end - i;
			// Likewise `lit` is capped at 128 by the loop condition.
			out.push(u8::try_from(lit - 1).expect("literal run is at most 128"));
			out.extend_from_slice(&input[i..lit_end]);
			i = lit_end;
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn literal_run() {
		// No 3-in-a-row: single literal block of 4, control byte = len-1 = 3.
		assert_eq!(rle_encode_channel(&[1, 2, 3, 4]), vec![3, 1, 2, 3, 4]);
	}

	#[test]
	fn repeat_run() {
		// Five identical bytes: control = run + 125 = 130, then the byte.
		assert_eq!(rle_encode_channel(&[7, 7, 7, 7, 7]), vec![130, 7]);
	}

	#[test]
	fn empty() {
		assert_eq!(rle_encode_channel(&[]), Vec::<u8>::new());
	}
}
