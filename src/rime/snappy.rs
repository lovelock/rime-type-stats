//! Snappy block 格式解压（raw block，不含 framed 流的长度头）。
//!
//! LevelDB 在启用压缩时以 type=1 标记 Snappy。这里只实现解压，足够读取
//! Rime 的 SSTable；不做压缩。

use crate::util::read_varint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnappyError {
    Truncated,
    BadOffset,
    LengthMismatch,
}

/// 解压一个 Snappy block。
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, SnappyError> {
    let mut pos = 0usize;
    let expected = read_varint(input, &mut pos).ok_or(SnappyError::Truncated)? as usize;
    let mut out = Vec::new();

    while pos < input.len() {
        let tag = input[pos];
        pos += 1;

        match tag & 0x03 {
            // literal
            0 => {
                let marker = (tag >> 2) as usize;
                let len = if marker < 60 {
                    marker + 1
                } else {
                    let extra = marker - 59;
                    if pos + extra > input.len() {
                        return Err(SnappyError::Truncated);
                    }
                    let mut len = 0usize;
                    for i in 0..extra {
                        len |= (input[pos + i] as usize) << (8 * i);
                    }
                    pos += extra;
                    len + 1
                };
                let end = pos.checked_add(len).ok_or(SnappyError::Truncated)?;
                if end > input.len() {
                    return Err(SnappyError::Truncated);
                }
                out.extend_from_slice(&input[pos..end]);
                pos = end;
            }
            // copy：offset 指向已经解压出的数据，允许重叠复制
            kind => {
                let (len, offset) = match kind {
                    1 => {
                        if pos >= input.len() {
                            return Err(SnappyError::Truncated);
                        }
                        let len = 4 + ((tag >> 2) & 0x07) as usize;
                        let offset = (((tag >> 5) as usize) << 8) | input[pos] as usize;
                        pos += 1;
                        (len, offset)
                    }
                    2 => {
                        if pos + 2 > input.len() {
                            return Err(SnappyError::Truncated);
                        }
                        let len = 1 + (tag >> 2) as usize;
                        let offset = u16::from_le_bytes([input[pos], input[pos + 1]]) as usize;
                        pos += 2;
                        (len, offset)
                    }
                    _ => {
                        if pos + 4 > input.len() {
                            return Err(SnappyError::Truncated);
                        }
                        let len = 1 + (tag >> 2) as usize;
                        let offset = u32::from_le_bytes(
                            input[pos..pos + 4]
                                .try_into()
                                .map_err(|_| SnappyError::Truncated)?,
                        ) as usize;
                        pos += 4;
                        (len, offset)
                    }
                };

                if offset == 0 || offset > out.len() {
                    return Err(SnappyError::BadOffset);
                }
                let start = out.len() - offset;
                for i in 0..len {
                    let byte = out[start + i];
                    out.push(byte);
                }
            }
        }
    }

    if out.len() != expected {
        return Err(SnappyError::LengthMismatch);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompresses_literal_only() {
        let input = [0x05, 0x10, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(decompress(&input).unwrap(), b"hello");
    }

    #[test]
    fn decompresses_overlapping_copy() {
        // 长度 9：literal "abc"，然后 type 2 copy（偏移 3、长度 6）
        let input = [0x09, 0x08, b'a', b'b', b'c', 0x16, 0x03, 0x00];
        assert_eq!(decompress(&input).unwrap(), b"abcabcabc");
    }

    #[test]
    fn decompresses_one_byte_offset_copy() {
        // 长度 8：literal "abcd"，然后 type 1 copy（偏移 2、长度 4）
        let input = [0x08, 0x0c, b'a', b'b', b'c', b'd', 0x01, 0x02];
        assert_eq!(decompress(&input).unwrap(), b"abcdcdcd");
    }

    #[test]
    fn rejects_truncated_input() {
        let input = [0x05, 0x10, b'h'];
        assert_eq!(decompress(&input), Err(SnappyError::Truncated));
    }

    #[test]
    fn rejects_zero_offset() {
        let input = [0x02, 0x00, b'a', 0x16, 0x00, 0x00];
        assert_eq!(decompress(&input), Err(SnappyError::BadOffset));
    }

    #[test]
    fn rejects_length_mismatch() {
        let input = [0x05, 0x00, b'a'];
        assert_eq!(decompress(&input), Err(SnappyError::LengthMismatch));
    }
}
