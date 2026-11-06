/// 从 `buf[*pos..]` 读取一个 LEB128 无符号整数。
/// 成功时推进 `*pos` 并返回数值；越界或超过 10 字节返回 None。
pub fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut i = *pos;

    loop {
        let byte = *buf.get(i)?;
        i += 1;

        // 第 10 个字节只允许低 1 位有效，超出即视为非法编码
        if shift == 63 && byte > 0x01 {
            return None;
        }
        result |= ((byte & 0x7f) as u64) << shift;

        if byte & 0x80 == 0 {
            *pos = i;
            return Some(result);
        }

        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_typical_values() {
        // 0x01 0xC8 0x01 即 1 与 200 的串联
        let buf = [0x01, 0xc8, 0x01, 0x00];
        let mut pos = 0;
        assert_eq!(read_varint(&buf, &mut pos), Some(1));
        assert_eq!(read_varint(&buf, &mut pos), Some(200));
        assert_eq!(read_varint(&buf, &mut pos), Some(0));
        assert_eq!(pos, 4);
    }
}