//! LevelDB SSTable（.ldb）只读解析。
//!
//! 只解析 index 与 data 两层 block：统计 key 的查询是全量扫描，
//! 不需要 filter（bloom）与 metaindex。

use crate::util::read_varint;
use std::fmt;

const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const FOOTER_LEN: usize = 48;
const BLOCK_TRAILER_LEN: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct TableEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableError {
    TooShort,
    BadMagic,
    UnsupportedCompression(u8),
    Malformed(&'static str),
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableError::TooShort => write!(f, "文件长度不足，不是合法的 SSTable"),
            TableError::BadMagic => write!(f, "footer magic 不匹配，可能不是 LevelDB 文件"),
            TableError::UnsupportedCompression(t) => {
                write!(f, "不支持的 block 压缩类型 {t}（仅支持未压缩）")
            }
            TableError::Malformed(msg) => write!(f, "SSTable 结构异常：{msg}"),
        }
    }
}

impl std::error::Error for TableError {}

/// 取出 `[off, off+size)` 的 block payload，并校验压缩类型为 none。
fn block_payload(bytes: &[u8], off: u64, size: u64) -> Result<&[u8], TableError> {
    let start = usize::try_from(off).map_err(|_| TableError::Malformed("block 偏移溢出"))?;
    let len = usize::try_from(size).map_err(|_| TableError::Malformed("block 长度溢出"))?;
    let end = start
        .checked_add(len)
        .ok_or(TableError::Malformed("block 越界"))?;
    let trailer_end = end
        .checked_add(BLOCK_TRAILER_LEN)
        .ok_or(TableError::Malformed("block trailer 越界"))?;
    if trailer_end > bytes.len() {
        return Err(TableError::Malformed("block 超出文件范围"));
    }

    let compression = bytes[end];
    if compression != 0 {
        return Err(TableError::UnsupportedCompression(compression));
    }
    Ok(&bytes[start..end])
}

/// 顺序重放一个 block 的全部条目（前缀压缩在重放中自然还原）。
fn block_entries(payload: &[u8]) -> Result<Vec<TableEntry>, TableError> {
    if payload.len() < 4 {
        return Err(TableError::Malformed("block 过短"));
    }
    let restart_count_pos = payload.len() - 4;
    let restart_count = u32::from_le_bytes(
        payload[restart_count_pos..]
            .try_into()
            .map_err(|_| TableError::Malformed("restart count 读取失败"))?,
    ) as usize;
    // 防御损坏数据导致的下溢：restart 数组异常时按无 restart 数组处理
    let restarts_bytes = match restart_count.checked_mul(4) {
        Some(n) if n <= payload.len() - 4 => n,
        _ => 0,
    };
    let entries_end = payload.len() - (restarts_bytes + 4);

    let mut entries = Vec::new();
    let mut pos = 0usize;
    let mut last_key: Vec<u8> = Vec::new();

    while pos < entries_end {
        let shared = read_varint(payload, &mut pos)
            .ok_or(TableError::Malformed("shared 解码失败"))? as usize;
        let non_shared = read_varint(payload, &mut pos)
            .ok_or(TableError::Malformed("non_shared 解码失败"))? as usize;
        let value_len = read_varint(payload, &mut pos)
            .ok_or(TableError::Malformed("value_len 解码失败"))? as usize;

        if shared > last_key.len() {
            return Err(TableError::Malformed("shared 超出上一条 key 长度"));
        }
        let key_end = pos
            .checked_add(non_shared)
            .ok_or(TableError::Malformed("key 越界"))?;
        if key_end > entries_end {
            return Err(TableError::Malformed("key 越过 block 边界"));
        }
        let mut key = last_key[..shared].to_vec();
        key.extend_from_slice(&payload[pos..key_end]);
        pos = key_end;

        let value_end = pos
            .checked_add(value_len)
            .ok_or(TableError::Malformed("value 越界"))?;
        if value_end > entries_end {
            return Err(TableError::Malformed("value 越过 block 边界"));
        }
        let value = payload[pos..value_end].to_vec();
        pos = value_end;

        last_key = key.clone();
        entries.push(TableEntry { key, value });
    }

    Ok(entries)
}

/// 解析整个 SSTable，按 key 升序返回全部条目。
pub fn parse_table(bytes: &[u8]) -> Result<Vec<TableEntry>, TableError> {
    if bytes.len() < FOOTER_LEN {
        return Err(TableError::TooShort);
    }

    let footer = &bytes[bytes.len() - FOOTER_LEN..];
    let magic = u64::from_le_bytes(
        footer[40..48]
            .try_into()
            .map_err(|_| TableError::Malformed("magic 读取失败"))?,
    );
    if magic != TABLE_MAGIC {
        return Err(TableError::BadMagic);
    }

    let mut pos = 0usize;
    let _metaindex_off =
        read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let _metaindex_size =
        read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let index_off =
        read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let index_size =
        read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;

    let index_payload = block_payload(bytes, index_off, index_size)?;
    let index_entries = block_entries(index_payload)?;

    let mut all = Vec::new();
    for entry in index_entries {
        let mut hpos = 0usize;
        let off = read_varint(&entry.value, &mut hpos)
            .ok_or(TableError::Malformed("block handle 偏移解码失败"))?;
        let size = read_varint(&entry.value, &mut hpos)
            .ok_or(TableError::Malformed("block handle 长度解码失败"))?;
        let payload = block_payload(bytes, off, size)?;
        all.extend(block_entries(payload)?);
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个只含单个 data block 的最小合法 SSTable。
    /// 条目：("a", "1") ("ab", "2")
    fn build_minimal_table() -> Vec<u8> {
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

        // entries: ("a","1") shared=0, ("ab","2") shared=1
        let mut block = Vec::new();
        push_varint(&mut block, 0);
        push_varint(&mut block, 1);
        push_varint(&mut block, 1);
        block.extend_from_slice(b"a");
        block.extend_from_slice(b"1");

        push_varint(&mut block, 1);
        push_varint(&mut block, 1);
        push_varint(&mut block, 1);
        block.extend_from_slice(b"b");
        block.extend_from_slice(b"2");

        // restart array：1 个 restart 点，偏移 0
        block.extend_from_slice(&0u32.to_le_bytes());
        block.extend_from_slice(&1u32.to_le_bytes());

        // LevelDB 的 block payload 尺寸包含末尾 4 字节 restart count
        let block_payload_len = block.len();
        let mut file = Vec::new();
        file.extend_from_slice(&block); // data block 位于偏移 0
        file.push(0); // compression = none
        file.extend_from_slice(&0u32.to_le_bytes()); // crc 占位，解析时不校验

        // index block：一个条目，key="a"，value = data block 的 handle(off=0, size=payload_len)
        let mut handle = Vec::new();
        push_varint(&mut handle, 0); // data block offset
        push_varint(&mut handle, block_payload_len as u64);

        let mut index_block = Vec::new();
        push_varint(&mut index_block, 0); // shared
        push_varint(&mut index_block, 1); // non_shared：key 只有 "a"
        push_varint(&mut index_block, handle.len() as u64);
        index_block.extend_from_slice(b"a");
        index_block.extend_from_slice(&handle);
        index_block.extend_from_slice(&0u32.to_le_bytes()); // restart 偏移
        index_block.extend_from_slice(&1u32.to_le_bytes()); // restart count

        let index_off = file.len() as u64;
        let index_payload_len = index_block.len() as u64;
        file.extend_from_slice(&index_block);
        file.push(0);
        file.extend_from_slice(&0u32.to_le_bytes());

        // footer
        let mut footer = Vec::new();
        push_varint(&mut footer, 0); // metaindex offset
        push_varint(&mut footer, 0); // metaindex size
        push_varint(&mut footer, index_off);
        push_varint(&mut footer, index_payload_len);
        footer.resize(40, 0);
        footer.extend_from_slice(&0xdb4775248b80fb57u64.to_le_bytes());
        file.extend_from_slice(&footer);
        file
    }

    #[test]
    fn parses_entries_with_prefix_compression() {
        let table = build_minimal_table();
        let entries = parse_table(&table).expect("解析应成功");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, b"a");
        assert_eq!(entries[0].value, b"1");
        assert_eq!(entries[1].key, b"ab");
        assert_eq!(entries[1].value, b"2");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut table = build_minimal_table();
        let len = table.len();
        table[len - 8..].copy_from_slice(&0u64.to_le_bytes());
        assert!(matches!(parse_table(&table), Err(TableError::BadMagic)));
    }

    #[test]
    fn rejects_truncated_file() {
        assert!(matches!(parse_table(&[0u8; 10]), Err(TableError::TooShort)));
    }

    #[test]
    fn rejects_unsupported_compression() {
        let mut table = build_minimal_table();
        let len = table.len();
        // index block trailer 位于 footer(48 字节) 之前 5 字节：type + crc32
        table[len - 48 - 5] = 1;
        assert!(matches!(
            parse_table(&table),
            Err(TableError::UnsupportedCompression(1))
        ));
    }
}
