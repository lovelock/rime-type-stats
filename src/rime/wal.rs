//! LevelDB WAL（.log）只读解析。
//!
//! 只关心 write batch 的形态：`seq + count + ops`。
//! 不做 CRC 校验（写入方使用 masked crc32c，只读场景无需校验），
//! 尾部不完整记录按 LevelDB 语义直接丢弃。
//!
//! LevelDB 的 WAL 按 32 KiB 分块；块尾剩余不足 7 字节（放不下 record 头）
//! 时写入方会补零，解析必须跳过这段 padding，否则会把零字节误读成长度字段，
//! 从而丢掉块边界之后的全部记录。

use crate::util::read_varint;

const HEADER_LEN: usize = 7;
const BLOCK_SIZE: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum BatchOp {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchRecord {
    pub seq: u64,
    pub ops: Vec<BatchOp>,
}

/// 解析 write batch 正文；失败返回 None（调用方跳过该记录）。
fn parse_batch(body: &[u8]) -> Option<BatchRecord> {
    if body.len() < 12 {
        return None;
    }
    let seq = u64::from_le_bytes(body[0..8].try_into().ok()?);
    let count = u32::from_le_bytes(body[8..12].try_into().ok()?);
    let mut pos = 12usize;

    let mut ops = Vec::new();
    for _ in 0..count {
        let tag = *body.get(pos)?;
        pos += 1;
        let key_len = read_varint(body, &mut pos)? as usize;
        let key_end = pos.checked_add(key_len)?;
        if key_end > body.len() {
            return None;
        }
        let key = body[pos..key_end].to_vec();
        pos = key_end;

        match tag {
            1 => {
                let value_len = read_varint(body, &mut pos)? as usize;
                let value_end = pos.checked_add(value_len)?;
                if value_end > body.len() {
                    return None;
                }
                ops.push(BatchOp::Put {
                    key,
                    value: body[pos..value_end].to_vec(),
                });
                pos = value_end;
            }
            0 => ops.push(BatchOp::Delete { key }),
            _ => return None,
        }
    }

    Some(BatchRecord { seq, ops })
}

/// 解析 WAL 文件，返回全部可识别的 write batch。
pub fn parse_log(bytes: &[u8]) -> Vec<BatchRecord> {
    let mut records = Vec::new();
    let mut pos = 0usize;
    // 正在进行的分片拼接缓冲
    let mut fragment: Option<Vec<u8>> = None;

    while pos < bytes.len() {
        // 块尾剩余不足一个 record 头：整段是写入方补的零 padding，跳到下一块
        let leftover = BLOCK_SIZE - (pos % BLOCK_SIZE);
        if leftover < HEADER_LEN {
            pos += leftover;
            continue;
        }
        if pos + HEADER_LEN > bytes.len() {
            break;
        }

        let length = u16::from_le_bytes([bytes[pos + 4], bytes[pos + 5]]) as usize;
        let record_type = bytes[pos + 6];
        let body_start = pos + HEADER_LEN;
        let body_end = body_start + length;

        // 尾部残缺：写入方尚未写完，直接停止
        if body_end > bytes.len() {
            break;
        }

        // 记录不跨越块边界；越界说明此处不是合法头，按损坏块跳过
        if body_end > pos - (pos % BLOCK_SIZE) + BLOCK_SIZE {
            pos = pos - (pos % BLOCK_SIZE) + BLOCK_SIZE;
            fragment = None;
            continue;
        }
        let body = &bytes[body_start..body_end];
        pos = body_end;

        match record_type {
            1 => {
                // FULL：丢弃任何残留分片
                fragment = None;
                if let Some(batch) = parse_batch(body) {
                    records.push(batch);
                }
            }
            2 => {
                // FIRST
                fragment = Some(body.to_vec());
            }
            3 => {
                // MIDDLE
                if let Some(buf) = fragment.as_mut() {
                    buf.extend_from_slice(body);
                }
            }
            4 => {
                // LAST
                if let Some(mut buf) = fragment.take() {
                    buf.extend_from_slice(body);
                    if let Some(batch) = parse_batch(&buf) {
                        records.push(batch);
                    }
                }
            }
            _ => {
                // 未知类型：无法可靠跳过，终止解析
                break;
            }
        }
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                return;
            }
        }
    }

    /// 构造函数，便于逐步搭建测试请求
    fn write_batch_record(records: Vec<&[u8]>, ty: u8) -> Vec<u8> {
        let mut out = Vec::new();
        for record in records {
            out.extend_from_slice(&0u32.to_le_bytes()); // crc 不校验，占位
            out.extend_from_slice(&(record.len() as u16).to_le_bytes());
            out.push(ty);
            out.extend_from_slice(record);
        }
        out
    }

    /// 构造 write batch：seq=7, 一个 put("k","v")
    fn simple_batch() -> Vec<u8> {
        let mut batch = Vec::new();
        batch.extend_from_slice(&7u64.to_le_bytes()); // seq
        batch.extend_from_slice(&1u32.to_le_bytes()); // count=1
        batch.push(1); // put
        push_varint(&mut batch, 1);
        batch.extend_from_slice(b"k");
        push_varint(&mut batch, 1);
        batch.extend_from_slice(b"v");
        batch
    }

    #[test]
    fn parses_full_record() {
        let log = write_batch_record(vec![&simple_batch()], 1);
        let records = parse_log(&log);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].seq, 7);
        assert_eq!(
            records[0].ops,
            vec![BatchOp::Put {
                key: b"k".to_vec(),
                value: b"v".to_vec()
            }]
        );
    }

    #[test]
    fn parses_fragmented_record() {
        let batch = simple_batch();
        let (first, last) = batch.split_at(5);
        let mut log = Vec::new();
        log.extend_from_slice(&write_batch_record(vec![first], 2));
        log.extend_from_slice(&write_batch_record(vec![last], 4));

        let records = parse_log(&log);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].seq, 7);
    }

    #[test]
    fn drops_trailing_partial_record() {
        let mut log = write_batch_record(vec![&simple_batch()], 1);
        // 追加一条声明长度 100 但只有 3 字节正文的残缺记录
        log.extend_from_slice(&0u32.to_le_bytes());
        log.extend_from_slice(&100u16.to_le_bytes());
        log.push(1);
        log.extend_from_slice(b"abc");

        let records = parse_log(&log);
        assert_eq!(records.len(), 1, "残缺尾部应被丢弃，且不 panic");
    }

    #[test]
    fn parses_delete_op() {
        let mut batch = Vec::new();
        batch.extend_from_slice(&1u64.to_le_bytes());
        batch.extend_from_slice(&1u32.to_le_bytes());
        batch.push(0); // delete
        push_varint(&mut batch, 2);
        batch.extend_from_slice(b"kk");

        let log = write_batch_record(vec![&batch], 1);
        let records = parse_log(&log);
        assert_eq!(
            records[0].ops,
            vec![BatchOp::Delete {
                key: b"kk".to_vec()
            }]
        );
    }

    #[test]
    fn parses_multiple_records() {
        let log = write_batch_record(vec![&simple_batch(), &simple_batch()], 1);
        let records = parse_log(&log);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn empty_input_yields_no_records() {
        assert!(parse_log(&[]).is_empty());
    }

    /// LevelDB 在块尾剩余不足 7 字节时补零；解析要跳过 padding 继续读下一块，
    /// 而不是把零字节当成 `length=0, type=0` 的伪记录后终止。
    #[test]
    fn skips_block_tail_zero_padding() {
        // 构造一条恰好结束于块尾前 3 字节的 put 记录：record = 7 + body = BLOCK_SIZE-3
        let body_len = BLOCK_SIZE - 3 - HEADER_LEN;
        let value_len = body_len - 15 - 3; // 8 seq + 4 count + 1 tag + 1 keylen + 1 key + 3 varint
        let mut batch = Vec::new();
        batch.extend_from_slice(&7u64.to_le_bytes());
        batch.extend_from_slice(&1u32.to_le_bytes());
        batch.push(1);
        push_varint(&mut batch, 1);
        batch.extend_from_slice(b"k");
        push_varint(&mut batch, value_len as u64);
        batch.resize(batch.len() + value_len, b'v');
        assert_eq!(7 + batch.len(), BLOCK_SIZE - 3);

        let mut log = write_batch_record(vec![&batch], 1);
        log.extend_from_slice(&[0u8; 3]); // 块尾 padding
        assert_eq!(log.len(), BLOCK_SIZE);
        // 下一块里的完整记录必须仍被解析出来
        log.extend_from_slice(&write_batch_record(vec![&simple_batch()], 1));

        let records = parse_log(&log);
        assert_eq!(records.len(), 2, "块尾 padding 不应吞掉后续记录");
        assert_eq!(records[1].seq, 7);
    }
}
