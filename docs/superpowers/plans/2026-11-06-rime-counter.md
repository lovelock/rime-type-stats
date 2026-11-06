# rime-counter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 统计每天在 macOS Squirrel 上用 Rime 打出的字数，通过菜单栏常驻图标与本地网页仪表盘查看日/周/月趋势。

**Architecture:** 单 crate 单二进制。纯 Rust 按文件名只读解析 `~/Library/Rime/stats.userdb` 下的 LevelDB SSTable 与 WAL（绕开被 Squirrel 排他锁持有的 `LOCK`，不需要快照复制，不依赖 snappy），把每日累计值以 JSONL 追加日志落盘；后台线程每 60 秒同步一次；`tiny_http` 在 `127.0.0.1` 提供内嵌单页仪表盘；`tray-icon` + `tao` 提供菜单栏入口。

**Tech Stack:** Rust 1.98+, `serde`/`serde_json`, `chrono`, `tiny_http 0.12`, `tray-icon 0.19`, `tao 0.30`

**Spec:** `docs/superpowers/specs/2026-11-06-rime-typing-counter-design.md`

## Global Constraints

- **只读、不复制**：不得通过 LevelDB/RocksDB 库打开 `stats.userdb` 目录，也不得复制快照；只以 `File::open` 按文件名读取 `*.ldb` / `*.log`。
- **零非必要依赖**：解析层不得引入压缩库（已确认所有 block 压缩类型为 `none`）；不得引入网络/云相关依赖。
- **仅本地监听**：HTTP 必须绑定 `127.0.0.1`，禁止 `0.0.0.0`。
- **单设备**：`device_id`（key 尾部 8 位十六进制）解析后丢弃，不做跨设备聚合。
- **前端零外部资源**：无 CDN、无外部字体、无框架；HTML 通过 `include_str!` 内嵌进二进制。
- **中文注释与提交信息**：提交信息使用 Conventional Commits 前缀 + 中文正文。
- **幂等写入**：同一天同字段的重复同步必须收敛到相同结果（取 `max`）。

## File Structure

| 文件 | 职责 |
|---|---|
| `Cargo.toml` | crate 元信息与依赖 |
| `src/lib.rs` | 库入口，导出各模块（便于集成测试） |
| `src/main.rs` | 组装：初始化状态、启动同步线程、启动 HTTP、运行菜单栏事件循环 |
| `src/util.rs` | varint 解码等纯工具函数 |
| `src/rime/mod.rs` | Rime 数据源入口：目录扫描、`RimeRecord` 类型、聚合 |
| `src/rime/sstable.rs` | LevelDB SSTable 解析（footer → index → data block） |
| `src/rime/wal.rs` | LevelDB WAL 解析（record framing + write batch） |
| `src/rime/parser.rs` | 内部 key 解析与按天聚合 |
| `src/store.rs` | JSONL 追加日志与折叠 |
| `src/sync.rs` | 定时同步编排与共享状态 |
| `src/server.rs` | tiny_http 服务与 API |
| `src/tray.rs` | 菜单栏入口与事件循环 |
| `src/web/index.html` | 内嵌仪表盘 |
| `scripts/install.sh` / `scripts/uninstall.sh` | LaunchAgent 安装与卸载 |
| `tests/fixtures/` | 解析层测试夹具（自造的字节序列，不含个人数据） |

**分层原则**：`rime` 层是纯函数（输入字节/路径，输出结构体），不持有句柄、不写文件，因此可完全用自造字节单测；`store` 只碰 JSONL；`server` 与 `tray` 只消费 `SyncState`。依赖方向单向：`tray`/`server` → `sync` → `store` → `rime` → `util`。

---

### Task 1: 项目骨架与 varint 解码

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/util.rs`
- Create: `tests/varint.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  - `rime_counter::util::read_varint(buf: &[u8], pos: &mut usize) -> Option<u64>` — 从 `buf[*pos..]` 读一个 LEB128 无符号整数，成功时推进 `*pos`，越界或过长返回 `None`。

- [ ] **Step 1: 创建 crate 骨架**

创建 `Cargo.toml`：

```toml
[package]
name = "rime-counter"
version = "0.1.0"
edition = "2021"

[lib]
name = "rime_counter"
path = "src/lib.rs"

[[bin]]
name = "rime-counter"
path = "src/main.rs"

[dependencies]
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tiny_http = "0.12"
tray-icon = "0.19"
tao = "0.30"
```

创建 `src/lib.rs`：

```rust
pub mod util;
```

创建 `src/main.rs`：

```rust
fn main() {
    println!("rime-counter 启动中");
}
```

创建 `src/util.rs`，先只放签名：

```rust
/// 从 `buf[*pos..]` 读取一个 LEB128 无符号整数。
/// 成功时推进 `*pos` 并返回数值；越界或超过 10 字节返回 None。
pub fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    todo!()
}
```

- [ ] **Step 2: 写失败的测试**

创建 `tests/varint.rs`：

```rust
use rime_counter::util::read_varint;

#[test]
fn decodes_single_byte() {
    let mut pos = 0;
    assert_eq!(read_varint(&[0x00], &mut pos), Some(0));
    assert_eq!(pos, 1);

    let mut pos = 0;
    assert_eq!(read_varint(&[0x7f], &mut pos), Some(127));
    assert_eq!(pos, 1);
}

#[test]
fn decodes_multi_byte() {
    // 200 = 0xC8 0x01
    let mut pos = 0;
    assert_eq!(read_varint(&[0xc8, 0x01], &mut pos), Some(200));
    assert_eq!(pos, 2);
}

#[test]
fn reads_from_offset() {
    let buf = [0xff, 0x05];
    let mut pos = 1;
    assert_eq!(read_varint(&buf, &mut pos), Some(5));
    assert_eq!(pos, 2);
}

#[test]
fn returns_none_when_truncated() {
    // 只有续位没有结束字节
    let mut pos = 0;
    assert_eq!(read_varint(&[0x80], &mut pos), None);
    assert_eq!(read_varint(&[], &mut pos), None);
}

#[test]
fn rejects_overlong_encoding() {
    let buf = [0x80; 11];
    let mut pos = 0;
    assert_eq!(read_varint(&buf, &mut pos), None);
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --test varint`
Expected: FAIL，`read_varint` panic 于 `not yet implemented`（5 个测试全部失败）

- [ ] **Step 4: 实现 varint 解码**

把 `src/util.rs` 替换为：

```rust
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
```

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test`
Expected: PASS，varint 相关的 6 个测试全部通过

- [ ] **Step 6: 提交**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/main.rs src/util.rs tests/varint.rs
git commit -m "feat: 搭建 crate 骨架并实现 LEB128 varint 解码"
```

---

### Task 2: SSTable 解析

**Files:**
- Create: `src/rime/mod.rs`
- Create: `src/rime/sstable.rs`
- Modify: `src/lib.rs`
- Test: `src/rime/sstable.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `rime_counter::util::read_varint`
- Produces:
  - `pub struct TableEntry { pub key: Vec<u8>, pub value: Vec<u8> }`
  - `pub fn parse_table(bytes: &[u8]) -> Result<Vec<TableEntry>, TableError>` — 解析整个 SSTable，返回全部条目（按 key 升序）。
  - `pub enum TableError { TooShort, BadMagic, UnsupportedCompression(u8), Malformed(&'static str) }`，实现 `Display` 与 `std::error::Error`。

**格式要点**（实现者必读）：
- 文件末尾 48 字节是 footer：`metaindex_handle(2 varint)` + `index_handle(2 varint)` + `padding(40)` + `magic(u64 LE)`，magic 必须等于 `0xdb4775248b80fb57`。
- 每个 block 存储为 `payload` + `trailer(5 字节)`，trailer 为 `compression_type(u8)` + `crc32(u32)`。本数据源恒为 `0`（未压缩）；遇到非 0 返回 `UnsupportedCompression`。
- 条目编码：`shared(varint)` + `non_shared(varint)` + `value_len(varint)` + `key_delta(non_shared)` + `value(value_len)`；`key = 上一条 key 的前 shared 字节 + key_delta`。
- block 末尾 4 字节是 restart 点个数，紧邻其前的 `4 * count` 字节是 restart 偏移数组；重放条目时应停在 `payload.len() - 4 - 4*count`。restart 点处 `shared` 恒为 0，因此**顺序重放整块即可正确还原**，无需随机访问。

- [ ] **Step 1: 写失败的测试**

在 `src/rime/sstable.rs` 末尾写入：

```rust
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

        let block_payload_len = block.len() - 4; // 不含 restart count
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
        let index_payload_len = (index_block.len() - 4) as u64;
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
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test rime::sstable`
Expected: 编译失败，`parse_table` / `TableError` / 模块未定义

- [ ] **Step 3: 实现 SSTable 解析**

写 `src/rime/sstable.rs`：

```rust
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
    let mut restart_count = u32::from_le_bytes(
        payload[restart_count_pos..]
            .try_into()
            .map_err(|_| TableError::Malformed("restart count 读取失败"))?,
    ) as usize;
    // 防御损坏数据导致的下溢
    let restarts_bytes = restart_count
        .checked_mul(4)
        .ok_or(TableError::Malformed("restart 数组过大"))?;
    if restarts_bytes + 4 > payload.len() {
        restart_count = 0;
    }
    let entries_end = payload
        .len()
        .saturating_sub(restarts_bytes + 4);

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
    let _metaindex_off = read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let _metaindex_size = read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let index_off = read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;
    let index_size = read_varint(footer, &mut pos).ok_or(TableError::Malformed("footer 解析失败"))?;

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
```

把 `src/lib.rs` 改为：

```rust
pub mod rime;
pub mod util;
```

创建 `src/rime/mod.rs`：

```rust
//! Rime 数据源（LevelDB 只读解析）。

pub mod sstable;
pub mod wal;
```

并临时创建空的 `src/rime/wal.rs`（下一任务填充）：

```rust
//! LevelDB WAL 解析（待实现）。
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test rime::sstable`
Expected: PASS，4 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/lib.rs src/rime/
git commit -m "feat(rime): 实现 SSTable 只读解析"
```

---

### Task 3: WAL 解析

**Files:**
- Modify: `src/rime/wal.rs`
- Test: `src/rime/wal.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `rime_counter::util::read_varint`
- Produces:
  - `pub struct BatchRecord { pub seq: u64, pub ops: Vec<BatchOp> }`
  - `pub enum BatchOp { Put { key: Vec<u8>, value: Vec<u8> }, Delete { key: Vec<u8> } }`
  - `pub fn parse_log(bytes: &[u8]) -> Vec<BatchRecord>` — 解析 WAL；尾部不完整记录**静默丢弃**；无法识别的记录（CRC 不符等）跳过其长度并继续。

**格式要点**：
- record 头 7 字节：`crc32c(u32 LE)` + `length(u16 LE)` + `type(u8)`。
- `type`：`1=FULL`（单条完整）、`2=FIRST`、`3=MIDDLE`、`4=LAST`（分片）。
- 拼接后得到 logical record，即 write batch：`seq(u64 LE)` + `count(u32 LE)` + `count` 个 `(tag(u8) + key(varint len) + key + [value(varint len) + value])`，`tag=1` 为 put，`tag=0` 为 delete。
- 本任务**不做 CRC 校验**（Squirrel 用 crc32c masking 变体，校验不是必需，且 WAL 只读不修）；只按长度推进。

- [ ] **Step 1: 写失败的测试**

在 `src/rime/wal.rs` 末尾写入：

```rust
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
        assert_eq!(records[0].ops, vec![BatchOp::Delete { key: b"kk".to_vec() }]);
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
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test rime::wal`
Expected: 编译失败，`parse_log` / `BatchRecord` / `BatchOp` 未定义

- [ ] **Step 3: 实现 WAL 解析**

写 `src/rime/wal.rs`：

```rust
//! LevelDB WAL（.log）只读解析。
//!
//! 只关心 write batch 的形态：`seq + count + ops`。
//! 不做 CRC 校验（写入方使用 masked crc32c，只读场景无需校验），
//! 尾部不完整记录按 LevelDB 语义直接丢弃。

use crate::util::read_varint;

const HEADER_LEN: usize = 7;

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
    let mut pos = 0usize;
    if body.len() < 12 {
        return None;
    }
    let seq = u64::from_le_bytes(body[0..8].try_into().ok()?);
    let count = u32::from_le_bytes(body[8..12].try_into().ok()?);
    pos = 12;

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

    while pos + HEADER_LEN <= bytes.len() {
        let length = u16::from_le_bytes([bytes[pos + 4], bytes[pos + 5]]) as usize;
        let record_type = bytes[pos + 6];
        let body_start = pos + HEADER_LEN;
        let body_end = body_start + length;

        // 尾部残缺：写入方尚未写完，直接停止
        if body_end > bytes.len() {
            break;
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
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test rime::wal`
Expected: PASS，6 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/rime/wal.rs
git commit -m "feat(rime): 实现 WAL 只读解析与分片拼接"
```

---

### Task 4: 记录提取与按天聚合

**Files:**
- Modify: `src/rime/mod.rs`
- Create: `src/rime/parser.rs`
- Test: `src/rime/parser.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `sstable::{parse_table, TableEntry}`, `wal::{parse_log, BatchOp}`
- Produces:
  - `pub struct DayStats { pub characters: u64, pub keystrokes: u64, pub commits: u64, pub speed_cpm: u32 }`（`Copy`、`Default`、`PartialEq`、`Debug`）
  - `pub fn parse_internal_key(user_key: &[u8]) -> Option<(String, RawField)>` — 解析内部 LevelDB key（含 8 字节 `seq+type` 尾部）。
  - `pub enum RawField { Characters, Keystrokes, CommitLength, SpeedAverageChar, SpeedAverageMs }`
  - `pub fn aggregate(entries: impl IntoIterator<Item = (Vec<u8>, u64, Vec<u8>)>) -> BTreeMap<String, DayStats>` — 输入 `(user_key, seq, value)`，按 `seq` 最大者取最新值，再按天聚合。
  - `pub fn read_dir(dir: &Path) -> Result<BTreeMap<String, DayStats>, ReadError>` — 扫描目录下 `*.ldb` 与 `*.log`，解析并聚合。

**key / value 形态**（实测）：
- user key：`statistics/day/YYYYMMDD/<field> \t<device_id>`，`\t` 是空格 + TAB 两个字节，`device_id` 为 8 位小写十六进制，**解析后丢弃**。
- 内部 key = user key + `seq(7 字节 LE) + type(1 字节)`。
- value：`c=<int> d=0 t=0`，只取 `c`。

**聚合规则**：
- `characters` ← `text/characters`
- `keystrokes` ← `text/code_keystrokes`
- `commits` ← 所有 `commit_length/*` 之和
- `speed_cpm` ← `speed_average/characters` 与 `speed_average/milliseconds` 累计值换算：`c*60000/ms`（`ms>0` 时，否则 0）

- [ ] **Step 1: 写失败的测试**

在 `src/rime/parser.rs` 末尾写入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn user_key(day: &str, field: &str) -> Vec<u8> {
        let mut k = format!("statistics/day/{day}/{field}").into_bytes();
        k.extend_from_slice(b" \t");
        k.extend_from_slice(b"9b3d1f36");
        k
    }

    fn internal_key(user: &[u8], seq: u64) -> Vec<u8> {
        let mut k = user.to_vec();
        k.extend_from_slice(&seq.to_le_bytes()[..7]);
        k.push(1);
        k
    }

    fn value(c: u64) -> Vec<u8> {
        format!("c={c} d=0 t=0").into_bytes()
    }

    #[test]
    fn parses_internal_key_fields() {
        let k = internal_key(&user_key("20261106", "text/characters"), 42);
        let (day, field) = parse_internal_key(&k).expect("应能解析");
        assert_eq!(day, "20261106");
        assert_eq!(field, RawField::Characters);

        let k = internal_key(&user_key("20261106", "text/code_keystrokes"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::Keystrokes);

        let k = internal_key(&user_key("20261106", "commit_length/3"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::CommitLength);

        let k = internal_key(&user_key("20261106", "speed_average/characters"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::SpeedAverageChar);

        let k = internal_key(&user_key("20261106", "speed_average/milliseconds"), 42);
        assert_eq!(parse_internal_key(&k).unwrap().1, RawField::SpeedAverageMs);

        // 未知字段与非统计 key 一律忽略
        let k = internal_key(&user_key("20261106", "text/code_characters"), 42);
        assert!(parse_internal_key(&k).is_none());
        assert!(parse_internal_key(b"random/key").is_none());
    }

    #[test]
    fn takes_latest_seq_per_key() {
        let uk = user_key("20261106", "text/characters");
        let entries = vec![
            (internal_key(&uk, 10), 10, value(100)),
            (internal_key(&uk, 30), 30, value(300)),
            (internal_key(&uk, 20), 20, value(200)),
        ];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].characters, 300, "应取 seq 最大的值");
    }

    #[test]
    fn sums_commit_lengths_and_computes_speed() {
        let entries = vec![
            (internal_key(&user_key("20261106", "commit_length/1"), 1), 1, value(10)),
            (internal_key(&user_key("20261106", "commit_length/3"), 1), 1, value(5)),
            (internal_key(&user_key("20261106", "commit_length/5_plus"), 1), 1, value(2)),
            (internal_key(&user_key("20261106", "text/characters"), 1), 1, value(1000)),
            (
                internal_key(&user_key("20261106", "speed_average/characters"), 1),
                1,
                value(600),
            ),
            (
                internal_key(&user_key("20261106", "speed_average/milliseconds"), 1),
                1,
                value(600_000),
            ),
        ];
        let map = aggregate(entries);
        let d = &map["20261106"];
        assert_eq!(d.characters, 1000);
        assert_eq!(d.commits, 17);
        // 600 字 / 600000ms = 60 字/分
        assert_eq!(d.speed_cpm, 60);
    }

    #[test]
    fn speed_is_zero_when_milliseconds_missing() {
        let entries = vec![(
            internal_key(&user_key("20261106", "speed_average/characters"), 1),
            1,
            value(600),
        )];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].speed_cpm, 0);
    }

    #[test]
    fn day_without_characters_is_still_recorded() {
        let entries = vec![(
            internal_key(&user_key("20261106", "commit_length/1"), 1),
            1,
            value(3),
        )];
        let map = aggregate(entries);
        assert_eq!(map["20261106"].commits, 3);
        assert_eq!(map["20261106"].characters, 0);
    }

    #[test]
    fn empty_input_gives_empty_map() {
        assert!(aggregate(Vec::new()).is_empty());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test rime::parser`
Expected: 编译失败，`parser` 模块与相关类型未定义

- [ ] **Step 3: 实现解析与聚合**

写 `src/rime/parser.rs`：

```rust
//! 内部 key 解析与按天聚合。

use std::collections::BTreeMap;
use std::path::Path;

use super::sstable::parse_table;
use super::wal::{parse_log, BatchOp};

/// 每日统计。`characters` 为字数的核心指标。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DayStats {
    pub characters: u64,
    pub keystrokes: u64,
    pub commits: u64,
    pub speed_cpm: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawField {
    Characters,
    Keystrokes,
    CommitLength,
    SpeedAverageChar,
    SpeedAverageMs,
}

#[derive(Debug)]
pub enum ReadError {
    Io(std::io::Error),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Io(e) => write!(f, "读取目录失败：{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

const PREFIX: &[u8] = b"statistics/day/";
/// 尾部 8 字节：seq(7) + type(1)
const KEY_TRAILER: usize = 8;
/// user key 尾部：空格 + TAB + 8 位 device_id
const DEVICE_SUFFIX: usize = 2 + 8;

/// 解析内部 key，返回 `(YYYYMMDD, 字段类别)`；非统计 key 或未知字段返回 None。
pub fn parse_internal_key(user_key: &[u8]) -> Option<(String, RawField)> {
    if user_key.len() < PREFIX.len() + 8 + DEVICE_SUFFIX + KEY_TRAILER {
        return None;
    }
    if !user_key.starts_with(PREFIX) {
        return None;
    }

    let body_end = user_key.len() - KEY_TRAILER;
    let body = &user_key[..body_end];
    if body.len() < DEVICE_SUFFIX {
        return None;
    }
    let device = &body[body.len() - DEVICE_SUFFIX..];
    // 校验分隔符与 device_id 形态
    if device[0] != b' ' || device[1] != b'\t' {
        return None;
    }
    if !device[2..].iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }

    let head = &body[PREFIX.len()..body.len() - DEVICE_SUFFIX];
    // 形态：YYYYMMDD/field
    if head.len() < 9 || head[8] != b'/' {
        return None;
    }
    let day_bytes = &head[..8];
    if !day_bytes.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let day = std::str::from_utf8(day_bytes).ok()?.to_string();
    let field = &head[9..];

    let kind = match field {
        b"text/characters" => RawField::Characters,
        b"text/code_keystrokes" => RawField::Keystrokes,
        b"speed_average/characters" => RawField::SpeedAverageChar,
        b"speed_average/milliseconds" => RawField::SpeedAverageMs,
        f if f.starts_with(b"commit_length/") => RawField::CommitLength,
        _ => return None,
    };

    Some((day, kind))
}

/// 从 `c=<int> d=0 t=0` 中取出 `c`。
fn parse_count(value: &[u8]) -> u64 {
    let text = match std::str::from_utf8(value) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    for part in text.split_whitespace() {
        if let Some(rest) = part.strip_prefix("c=") {
            return rest.parse::<u64>().unwrap_or(0);
        }
    }
    0
}

/// 内部使用的字段键，可排序以便稳定遍历。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RawFieldKey {
    Characters,
    Keystrokes,
    CommitLength,
    SpeedAverageChar,
    SpeedAverageMs,
}

impl From<RawField> for RawFieldKey {
    fn from(f: RawField) -> Self {
        match f {
            RawField::Characters => RawFieldKey::Characters,
            RawField::Keystrokes => RawFieldKey::Keystrokes,
            RawField::CommitLength => RawFieldKey::CommitLength,
            RawField::SpeedAverageChar => RawFieldKey::SpeedAverageChar,
            RawField::SpeedAverageMs => RawFieldKey::SpeedAverageMs,
        }
    }
}

/// 速度需要两个字段同时存在才能换算，因此按天暂存两个累计值再统一计算。
pub fn aggregate(
    entries: impl IntoIterator<Item = (Vec<u8>, u64, Vec<u8>)>,
) -> BTreeMap<String, DayStats> {
    use std::collections::HashMap;

    let mut latest: HashMap<(String, RawFieldKey), (u64, u64)> = HashMap::new();
    for (user_key, seq, value) in entries {
        let Some((day, field)) = parse_internal_key(&user_key) else {
            continue;
        };
        let key = (day, RawFieldKey::from(field));
        let amount = parse_count(&value);
        match latest.get(&key) {
            Some((existing_seq, _)) if *existing_seq >= seq => {}
            _ => {
                latest.insert(key, (seq, amount));
            }
        }
    }

    // 先收集每天的原始字段值
    struct Acc {
        characters: u64,
        keystrokes: u64,
        commits: u64,
        speed_chars: u64,
        speed_ms: u64,
    }
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();
    for ((day, field), (_, amount)) in latest {
        let acc = accs.entry(day).or_insert(Acc {
            characters: 0,
            keystrokes: 0,
            commits: 0,
            speed_chars: 0,
            speed_ms: 0,
        });
        match field {
            RawFieldKey::Characters => acc.characters = amount,
            RawFieldKey::Keystrokes => acc.keystrokes = amount,
            RawFieldKey::CommitLength => acc.commits = acc.commits.saturating_add(amount),
            RawFieldKey::SpeedAverageChar => acc.speed_chars = amount,
            RawFieldKey::SpeedAverageMs => acc.speed_ms = amount,
        }
    }

    accs.into_iter()
        .map(|(day, acc)| {
            let speed_cpm = if acc.speed_ms > 0 {
                ((acc.speed_chars as u128 * 60_000) / acc.speed_ms as u128) as u32
            } else {
                0
            };
            (
                day,
                DayStats {
                    characters: acc.characters,
                    keystrokes: acc.keystrokes,
                    commits: acc.commits,
                    speed_cpm,
                },
            )
        })
        .collect()
}

/// 扫描目录下的 `*.ldb` 与 `*.log` 并聚合。单个文件损坏不影响其余文件。
pub fn read_dir(dir: &Path) -> Result<BTreeMap<String, DayStats>, ReadError> {
    let mut collected: Vec<(Vec<u8>, u64, Vec<u8>)> = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(ReadError::Io)?;
    let mut ldb_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut log_paths: Vec<std::path::PathBuf> = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(ReadError::Io)?;
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("ldb") => ldb_paths.push(path),
            Some("log") => log_paths.push(path),
            _ => {}
        }
    }
    // 文件名即文件编号，按编号升序保证 seq 递增语义
    ldb_paths.sort();
    log_paths.sort();

    for path in &ldb_paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                // 文件可能在读取瞬间被 LevelDB 轮转掉：跳过即可
                eprintln!("跳过 {}：{e}", path.display());
                continue;
            }
        };
        match parse_table(&bytes) {
            Ok(entries) => {
                for entry in entries {
                    let (user_key, seq) = split_internal_key(&entry.key);
                    collected.push((user_key, seq, entry.value));
                }
            }
            Err(e) => {
                // 单个文件损坏不影响整体统计
                eprintln!("跳过 {}：{e}", path.display());
            }
        }
    }

    for path in &log_paths {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("跳过 {}：{e}", path.display());
                continue;
            }
        };
        for batch in parse_log(&bytes) {
            for (i, op) in batch.ops.into_iter().enumerate() {
                match op {
                    BatchOp::Put { key, value } => {
                        let seq = batch.seq + i as u64;
                        let (user_key, _) = split_internal_key(&key);
                        collected.push((user_key, seq, value));
                    }
                    BatchOp::Delete { .. } => {
                        // 统计 key 从不删除；即便出现也直接忽略
                    }
                }
            }
        }
    }

    Ok(aggregate(collected))
}

/// 把内部 key 拆成 `(user_key, seq)`；不足 8 字节时视为 seq=0。
fn split_internal_key(internal: &[u8]) -> (Vec<u8>, u64) {
    if internal.len() < KEY_TRAILER {
        return (internal.to_vec(), 0);
    }
    let split = internal.len() - KEY_TRAILER;
    let user = internal[..split].to_vec();
    let tail = &internal[split..];
    let mut seq_bytes = [0u8; 8];
    seq_bytes[..7].copy_from_slice(&tail[..7]);
    (user, u64::from_le_bytes(seq_bytes))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test rime::parser`
Expected: PASS，5 个测试全部通过

- [ ] **Step 5: 用真实数据做冒烟检查**

Run:
```bash
cargo run --quiet --example dump_days 2>/dev/null || true
```
若尚未有 example，改用下面的临时验证（内联测试无法访问真实文件，故直接写一个 example）：

创建 `examples/dump_days.rs`：

```rust
use rime_counter::rime::parser;

fn main() {
    let home = std::env::var("HOME").expect("HOME 未设置");
    let dir = std::path::Path::new(&home).join("Library/Rime/stats.userdb");
    match parser::read_dir(&dir) {
        Ok(days) => {
            println!("解析到 {} 天", days.len());
            for (day, stats) in days.iter().take(5) {
                println!("{day}: {stats:?}");
            }
        }
        Err(e) => eprintln!("解析失败：{e}"),
    }
}
```

Run: `cargo run --example dump_days`
Expected: 打印出天数和前 5 天的统计，**字数非零**。记录实际输出的最早日期，用于验收。

- [ ] **Step 6: 提交**

```bash
git add src/rime/ examples/dump_days.rs
git commit -m "feat(rime): 内部 key 解析与按天聚合，附真实数据冒烟示例"
```

---

### Task 5: JSONL 存储层

**Files:**
- Create: `src/store.rs`
- Modify: `src/lib.rs`
- Test: `src/store.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `rime::parser::DayStats`
- Produces:
  - `pub struct DayRecord { pub day: String, pub characters: u64, pub keystrokes: u64, pub commits: u64, pub speed_cpm: u32 }`（`Serialize`/`Deserialize`，字段名与 JSON 一致）
  - `pub struct Store { /* 私有 */ }`
  - `impl Store { pub fn open(dir: &Path) -> std::io::Result<Self>; pub fn load(&self) -> std::io::Result<BTreeMap<String, DayStats>>; pub fn append(&self, day: &str, stats: &DayStats) -> std::io::Result<bool>; }`
    - `append` 返回 `true` 表示确实写入了新行，`false` 表示与已有值相同而跳过。

**语义**：
- 文件：`<dir>/events.jsonl`，一行一条，**只追加**。
- `load()` 读取全部行，对同一天取各字段的 `max`（单调递增语义）；坏行跳过。
- `append()` 先 `load()`，若新值各字段**都不大于**现有值则返回 `false` 不写（已收敛）；否则追加一行并返回 `true`。
- 日期统一使用 `YYYY-MM-DD` 字符串（`DayStats` 侧是 `YYYYMMDD`，由调用方转换）。

- [ ] **Step 1: 写失败的测试**

在 `src/store.rs` 末尾写入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn stats(chars: u64) -> DayStats {
        DayStats {
            characters: chars,
            keystrokes: chars * 3,
            commits: 1,
            speed_cpm: 50,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rime-counter-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_store_loads_empty_map() {
        let dir = temp_dir("empty");
        let store = Store::open(&dir).unwrap();
        assert!(store.load().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_then_load_roundtrips() {
        let dir = temp_dir("roundtrip");
        let store = Store::open(&dir).unwrap();
        assert!(store.append("2026-11-06", &stats(100)).unwrap());

        let loaded = store.load().unwrap();
        assert_eq!(loaded["2026-11-06"], stats(100));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_same_value_is_skipped() {
        let dir = temp_dir("skip");
        let store = Store::open(&dir).unwrap();
        let first = store.append("2026-11-06", &stats(100)).unwrap();
        let second = store.append("2026-11-06", &stats(100)).unwrap();
        assert!(first, "首次写入应返回 true");
        assert!(!second, "值未变化应返回 false，不追加");

        let content = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert_eq!(content.lines().count(), 1, "文件中只应有一行");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_takes_max_across_duplicate_days() {
        let dir = temp_dir("max");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-06","characters":100,"keystrokes":300,"commits":1,"speed_cpm":50}
{"day":"2026-11-06","characters":250,"keystrokes":700,"commits":3,"speed_cpm":60}
{"day":"2026-11-06","characters":180,"keystrokes":500,"commits":2,"speed_cpm":55}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load().unwrap();
        let d = &loaded["2026-11-06"];
        assert_eq!(d.characters, 250);
        assert_eq!(d.keystrokes, 700);
        assert_eq!(d.commits, 3);
        assert_eq!(d.speed_cpm, 60);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_skips_malformed_lines() {
        let dir = temp_dir("malformed");
        std::fs::write(
            dir.join("events.jsonl"),
            r#"{"day":"2026-11-06","characters":100,"keystrokes":300,"commits":1,"speed_cpm":50}
这不是 JSON
{"day":"2026-11-07","characters":20,"keystrokes":60,"commits":1,"speed_cpm":40}
"#,
        )
        .unwrap();

        let store = Store::open(&dir).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 2, "坏行应被跳过，其余两行正常解析");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_creates_parent_dirs() {
        let dir = temp_dir("nested");
        let nested = dir.join("a").join("b");
        let store = Store::open(&nested).unwrap();
        assert!(store.append("2026-11-06", &stats(1)).unwrap());
        assert!(nested.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test store::`
Expected: 编译失败，`Store` / `DayRecord` 未定义

- [ ] **Step 3: 实现存储层**

写 `src/store.rs`：

```rust
//! 追加式 JSONL 存储。
//!
//! 每次同步写入的是“当天截至目前的累计值”，因此折叠时对同一天取各字段最大值，
//! 使重复/乱序/重放同步都收敛到相同结果（幂等）。

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rime::parser::DayStats;

const EVENTS_FILE: &str = "events.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayRecord {
    pub day: String,
    pub characters: u64,
    pub keystrokes: u64,
    pub commits: u64,
    pub speed_cpm: u32,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// 打开（必要时创建）数据目录。
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Store {
            dir: dir.to_path_buf(),
        })
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join(EVENTS_FILE)
    }

    /// 读取全部记录，按天折叠（各字段取最大）。
    pub fn load(&self) -> std::io::Result<BTreeMap<String, DayStats>> {
        let mut out: BTreeMap<String, DayStats> = BTreeMap::new();
        let content = match std::fs::read_to_string(self.events_path()) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: DayRecord = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => continue, // 坏行跳过，不影响其余数据
            };
            let entry = out.entry(record.day).or_default();
            entry.characters = entry.characters.max(record.characters);
            entry.keystrokes = entry.keystrokes.max(record.keystrokes);
            entry.commits = entry.commits.max(record.commits);
            entry.speed_cpm = entry.speed_cpm.max(record.speed_cpm);
        }

        Ok(out)
    }

    /// 追加一天的最新累计值；与现有值完全相同时跳过并返回 false。
    pub fn append(&self, day: &str, stats: &DayStats) -> std::io::Result<bool> {
        let current = self.load()?;
        if let Some(existing) = current.get(day) {
            let unchanged = existing.characters >= stats.characters
                && existing.keystrokes >= stats.keystrokes
                && existing.commits >= stats.commits
                && existing.speed_cpm >= stats.speed_cpm;
            if unchanged {
                return Ok(false);
            }
        }

        let record = DayRecord {
            day: day.to_string(),
            characters: stats.characters,
            keystrokes: stats.keystrokes,
            commits: stats.commits,
            speed_cpm: stats.speed_cpm,
        };
        let mut line = serde_json::to_string(&record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');

        std::fs::create_dir_all(&self.dir)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())?;
        file.write_all(line.as_bytes())?;
        file.flush()?;

        Ok(true)
    }
}
```

把 `src/lib.rs` 改为：

```rust
pub mod rime;
pub mod store;
pub mod util;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test store::`
Expected: PASS，6 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/store.rs src/lib.rs
git commit -m "feat(store): 实现追加式 JSONL 存储与幂等折叠"
```

---

### Task 6: 同步编排与共享状态

**Files:**
- Create: `src/sync.rs`
- Modify: `src/lib.rs`
- Test: `src/sync.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `rime::parser::{read_dir, DayStats}`, `store::Store`
- Produces:
  - `pub struct SyncOutcome { pub days: BTreeMap<String, DayStats>, pub changed: usize, pub error: Option<String>, pub last_sync_unix: u64 }`
  - `pub struct SyncState { /* 私有 */ }`，`SyncState: Clone`（内部 `Arc<RwLock<_>>`）
  - `impl SyncState { pub fn new(initial: BTreeMap<String, DayStats>) -> Self; pub fn snapshot(&self) -> (BTreeMap<String, DayStats>, Option<String>, u64); pub fn replace(&self, outcome: SyncOutcome); }`
  - `pub fn sync_once(rime_dir: &Path, store: &Store) -> SyncOutcome` — 执行一次完整同步，**不返回 Err**，失败信息放进 `error`。
  - `pub fn to_iso_day(compact: &str) -> Option<String>` — `"20261106"` → `"2026-11-06"`，格式不对返回 `None`。

**行为**：
- `sync_once` 读目录 → 逐天 `store.append` → 统计 `changed` 数量 → 返回最新全量日表（以 store 折叠结果为准）。
- `replace` 仅当新结果至少与旧结果一样新时才覆盖（防止一次失败的扫描清空 UI）。
- 时间戳用 `std::time::SystemTime` 转 Unix 秒，不引入额外依赖。

- [ ] **Step 1: 写失败的测试**

在 `src/sync.rs` 末尾写入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("rime-counter-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn converts_compact_day_to_iso() {
        assert_eq!(to_iso_day("20261106").as_deref(), Some("2026-11-06"));
        assert_eq!(to_iso_day("2026-11-06"), None);
        assert_eq!(to_iso_day("2026110"), None);
        assert_eq!(to_iso_day("abcdefgh"), None);
    }

    #[test]
    fn sync_once_reports_error_for_missing_dir() {
        let dir = temp_dir("missing");
        let store = Store::open(&dir).unwrap();
        let outcome = sync_once(&dir.join("不存在的目录"), &store);
        assert!(outcome.error.is_some(), "目录不存在时应记录错误");
        assert!(outcome.days.is_empty());
        assert_eq!(outcome.changed, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_state_keeps_previous_data_on_error() {
        let mut initial = BTreeMap::new();
        initial.insert(
            "2026-11-06".to_string(),
            DayStats {
                characters: 100,
                ..Default::default()
            },
        );
        let state = SyncState::new(initial);

        state.replace(SyncOutcome {
            days: BTreeMap::new(),
            changed: 0,
            error: Some("读取失败".to_string()),
            last_sync_unix: 1,
        });

        let (days, error, _) = state.snapshot();
        assert_eq!(days["2026-11-06"].characters, 100, "失败不应清空既有数据");
        assert_eq!(error.as_deref(), Some("读取失败"));
    }

    #[test]
    fn sync_state_replaces_on_success() {
        let state = SyncState::new(BTreeMap::new());
        let mut days = BTreeMap::new();
        days.insert(
            "2026-11-07".to_string(),
            DayStats {
                characters: 42,
                ..Default::default()
            },
        );
        state.replace(SyncOutcome {
            days,
            changed: 1,
            error: None,
            last_sync_unix: 99,
        });

        let (days, error, ts) = state.snapshot();
        assert_eq!(days["2026-11-07"].characters, 42);
        assert_eq!(error, None);
        assert_eq!(ts, 99);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test sync::`
Expected: 编译失败，`SyncState` / `sync_once` / `to_iso_day` 未定义

- [ ] **Step 3: 实现同步编排**

写 `src/sync.rs`：

```rust
//! 定时同步编排与线程间共享状态。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::rime::parser::{read_dir, DayStats};
use crate::store::Store;

/// 一次同步的完整结果。
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    pub days: BTreeMap<String, DayStats>,
    pub changed: usize,
    pub error: Option<String>,
    pub last_sync_unix: u64,
}

/// 线程间共享的只读快照，供 HTTP 与菜单栏读取。
#[derive(Clone)]
pub struct SyncState {
    inner: Arc<RwLock<Snapshot>>,
}

#[derive(Clone, Default)]
struct Snapshot {
    days: BTreeMap<String, DayStats>,
    error: Option<String>,
    last_sync_unix: u64,
}

impl SyncState {
    pub fn new(initial: BTreeMap<String, DayStats>) -> Self {
        SyncState {
            inner: Arc::new(RwLock::new(Snapshot {
                days: initial,
                error: None,
                last_sync_unix: 0,
            })),
        }
    }

    /// 返回 `(日表, 错误信息, 最后同步时间)`。
    pub fn snapshot(&self) -> (BTreeMap<String, DayStats>, Option<String>, u64) {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        (guard.days.clone(), guard.error.clone(), guard.last_sync_unix)
    }

    /// 用新结果替换快照。失败（`error` 非空）时保留原有日表，只更新错误信息。
    pub fn replace(&self, outcome: SyncOutcome) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if outcome.error.is_none() {
            guard.days = outcome.days;
        }
        guard.error = outcome.error;
        guard.last_sync_unix = outcome.last_sync_unix;
    }
}

/// 把紧凑日期 `YYYYMMDD` 转成 `YYYY-MM-DD`。
pub fn to_iso_day(compact: &str) -> Option<String> {
    if compact.len() != 8 || !compact.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{}",
        &compact[..4],
        &compact[4..6],
        &compact[6..]
    ))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 执行一次完整同步。任何失败都收敛为 `error` 字段，不 panic、不返回 Err。
pub fn sync_once(rime_dir: &Path, store: &Store) -> SyncOutcome {
    let scanned = match read_dir(rime_dir) {
        Ok(days) => days,
        Err(e) => {
            return SyncOutcome {
                days: BTreeMap::new(),
                changed: 0,
                error: Some(format!("{e}")),
                last_sync_unix: now_unix(),
            };
        }
    };

    let mut changed = 0usize;
    let mut write_error: Option<String> = None;
    for (compact_day, stats) in &scanned {
        let Some(iso_day) = to_iso_day(compact_day) else {
            continue;
        };
        match store.append(&iso_day, stats) {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(e) => write_error = Some(format!("写入存储失败：{e}")),
        }
    }

    // 以存储折叠结果为准，保证 UI 与磁盘一致
    let days = match store.load() {
        Ok(days) => days,
        Err(e) => {
            return SyncOutcome {
                days: BTreeMap::new(),
                changed,
                error: Some(format!("读取存储失败：{e}")),
                last_sync_unix: now_unix(),
            };
        }
    };

    SyncOutcome {
        days,
        changed,
        error: write_error,
        last_sync_unix: now_unix(),
    }
}
```

把 `src/lib.rs` 改为：

```rust
pub mod rime;
pub mod store;
pub mod sync;
pub mod util;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test sync::`
Expected: PASS，4 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/sync.rs src/lib.rs
git commit -m "feat(sync): 实现同步编排与共享快照状态"
```

---

### Task 7: HTTP 服务与 API

**Files:**
- Create: `src/server.rs`
- Create: `src/web/index.html`（本任务只放占位骨架，Task 9 填充完整仪表盘）
- Modify: `src/lib.rs`
- Test: `src/server.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `sync::SyncState`
- Produces:
  - `pub fn render_days(state: &SyncState) -> String` — 生成 `/api/days` 的 JSON 响应体
  - `pub fn bind(port: u16) -> Option<tiny_http::Server>` — 尝试绑定 `127.0.0.1:port`，失败返回 `None`
  - `pub fn serve(server: tiny_http::Server, state: SyncState)` — 阻塞式事件循环，处理请求直到服务被关闭
  - `pub fn static_index() -> &'static str` — 返回内嵌 HTML

**JSON 形态**（与 spec 一致）：

```json
{
  "sync": {"last_sync_unix": 1791796577, "error": null},
  "days": [
    {"day":"2026-09-10","characters":7714,"keystrokes":20000,"commits":990,"speed_cpm":75}
  ]
}
```

**约定**：
- 只绑定回环地址；`bind` 内部用 `"127.0.0.1:{port}"`。
- 响应头 `Content-Type` 与 `Cache-Control: no-store`。
- 未知路径返回 404。

- [ ] **Step 1: 写失败的测试**

在 `src/server.rs` 末尾写入：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rime::parser::DayStats;

    fn state_with_days() -> SyncState {
        let mut days = std::collections::BTreeMap::new();
        days.insert(
            "2026-11-06".to_string(),
            DayStats {
                characters: 1234,
                keystrokes: 4000,
                commits: 210,
                speed_cpm: 78,
            },
        );
        let state = SyncState::new(days);
        state.replace(crate::sync::SyncOutcome {
            days: state.snapshot().0,
            changed: 0,
            error: None,
            last_sync_unix: 1791796577,
        });
        state
    }

    #[test]
    fn renders_days_json() {
        let body = render_days(&state_with_days());
        let v: serde_json::Value = serde_json::from_str(&body).expect("应为合法 JSON");

        assert_eq!(v["sync"]["last_sync_unix"], 1791796577);
        assert!(v["sync"]["error"].is_null());

        let days = v["days"].as_array().expect("days 应为数组");
        assert_eq!(days.len(), 1);
        assert_eq!(days[0]["day"], "2026-11-06");
        assert_eq!(days[0]["characters"], 1234);
        assert_eq!(days[0]["keystrokes"], 4000);
        assert_eq!(days[0]["commits"], 210);
        assert_eq!(days[0]["speed_cpm"], 78);
    }

    #[test]
    fn renders_days_sorted_by_date() {
        let mut days = std::collections::BTreeMap::new();
        for d in ["2026-11-08", "2026-11-06", "2026-11-07"] {
            days.insert(d.to_string(), DayStats::default());
        }
        let state = SyncState::new(days);
        let body = render_days(&state);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let dates: Vec<&str> = v["days"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["day"].as_str().unwrap())
            .collect();
        assert_eq!(dates, vec!["2026-11-06", "2026-11-07", "2026-11-08"]);
    }

    #[test]
    fn renders_empty_days_array() {
        let state = SyncState::new(std::collections::BTreeMap::new());
        let body = render_days(&state);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["days"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn bind_uses_loopback_only() {
        let server = bind(0).expect("绑定随机端口应成功");
        let addr = server.server_addr();
        assert!(
            addr.to_ip().is_loopback(),
            "必须只绑定回环地址，实际为 {addr}"
        );
    }

    #[test]
    fn bind_first_available_reports_chosen_port() {
        let (_server, port) = bind_first_available(8787, 11).expect("应能绑定到一个端口");
        assert!((8787..=8797).contains(&port));
    }

    #[test]
    fn static_index_is_non_empty_html() {
        let html = static_index();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.trim().is_empty());
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test server::`
Expected: 编译失败，`render_days` / `bind` / `static_index` 未定义

- [ ] **Step 3: 实现 HTTP 服务**

创建 `src/web/index.html`（占位，Task 9 会替换为完整仪表盘）：

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>打字统计</title>
</head>
<body>
<p>仪表盘加载中…</p>
</body>
</html>
```

写 `src/server.rs`：

```rust
//! 本地 HTTP 服务：提供仪表盘页面与日数据接口。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tiny_http::{Header, Response, Server};

use crate::sync::SyncState;

/// 内嵌仪表盘（单文件，无外部资源）。
const INDEX_HTML: &str = include_str!("web/index.html");

pub fn static_index() -> &'static str {
    INDEX_HTML
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("响应头字面量必须合法")
}

/// 生成 `/api/days` 的响应体。
pub fn render_days(state: &SyncState) -> String {
    let (days, error, last_sync_unix) = state.snapshot();
    let day_values: Vec<serde_json::Value> = days
        .iter()
        .map(|(day, stats)| {
            serde_json::json!({
                "day": day,
                "characters": stats.characters,
                "keystrokes": stats.keystrokes,
                "commits": stats.commits,
                "speed_cpm": stats.speed_cpm,
            })
        })
        .collect();

    serde_json::json!({
        "sync": {
            "last_sync_unix": last_sync_unix,
            "error": error,
        },
        "days": day_values,
    })
    .to_string()
}

/// 绑定回环地址上的指定端口；失败返回 None。
pub fn bind(port: u16) -> Option<Server> {
    Server::http(("127.0.0.1", port)).ok()
}

/// 处理请求直到 `shutdown` 被置位。
pub fn serve(server: Server, state: SyncState, shutdown: Arc<AtomicBool>) {
    for request in server.incoming_requests() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("");

        let response = match path {
            "/" | "/index.html" => Response::from_string(static_index())
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store")),
            "/api/days" => Response::from_string(render_days(&state))
                .with_header(header("Content-Type", "application/json; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store")),
            _ => Response::from_string("404 Not Found")
                .with_status_code(404)
                .with_header(header("Content-Type", "text/plain; charset=utf-8")),
        };

        let _ = request.respond(response);
    }
}

/// 从起始端口开始尝试绑定，返回 `(server, port)` 与 `Ok` 表示成功。
pub fn bind_first_available(start: u16, attempts: u16) -> Option<(Server, u16)> {
    for offset in 0..attempts {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        if let Some(server) = bind(port) {
            return Some((server, port));
        }
    }
    None
}

把 `src/lib.rs` 改为：

```rust
pub mod rime;
pub mod server;
pub mod store;
pub mod sync;
pub mod util;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test server::`
Expected: PASS，5 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/server.rs src/web/index.html src/lib.rs
git commit -m "feat(server): 实现本地 HTTP 服务与日数据接口"
```

---

### Task 8: 菜单栏

**Files:**
- Create: `src/icons.rs`
- Create: `src/tray.rs`
- Modify: `src/lib.rs`
- Test: `src/icons.rs`（内联 `#[cfg(test)]`）

**Interfaces:**
- Consumes: `sync::SyncState`
- Produces:
  - `pub const ICON_RGBA: (u32, u32, &[u8])` — 32×32 位 RGBA 原始像素，`const` 便于编译期内嵌
  - `pub fn menu_bar_title(today_chars: u64) -> String` — 生成 `⌨ 1,234` 形式的标题
  - `pub fn format_thousands(n: u64) -> String` — `1234` → `"1,234"`
  - `pub fn run_app(rime_dir: PathBuf, data_dir: PathBuf, state: SyncState, port: u16) -> !` — 运行菜单栏事件循环（`main` 调用）

**图标策略**：不引入图像依赖。用代码在 `src/icons.rs` 里以紧凑方式生成"圆角方块 + 三条横线"的指针形状（模拟键盘），32×32 RGBA。菜单栏用 `icon_as_template(true)`，系统会自动按浅/深色着色，因此像素只用黑白 + alpha。

**事件循环要点**（已核对 `tray-icon 0.19.3` / `tao 0.30.8` / `muda 0.15.3` 的真实签名）：
- macOS 上必须**在事件循环已开始运行后**创建 `TrayIcon`，因此第一次 `StartCause::Init` 事件里创建。
- `tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS}` 的 `set_activation_policy(ActivationPolicy::Accessory)` 隐藏 Dock 图标。
- `EventLoopBuilder::<UserEvent>::with_user_event()` 建循环，用一个 `EventLoopProxy<UserEvent>` 从同步线程唤醒 UI。
- `ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(60))` 驱动定时同步。
- `TrayIconEvent::receiver().try_recv()`、`MenuEvent::receiver().try_recv()` 在 `MainEventsCleared` 里轮询。
- 左键点击：`MouseButton::Left` 且 `MouseButtonState::Up` 时 `open` 网页；菜单只挂右键（`with_menu_on_left_click(false)`）。

- [ ] **Step 1: 写失败的测试**

在 `src/icons.rs` 末尾写入：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_thousands() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(7), "7");
        assert_eq!(format_thousands(999), "999");
        assert_eq!(format_thousands(1000), "1,000");
        assert_eq!(format_thousands(1234), "1,234");
        assert_eq!(format_thousands(1234567), "1,234,567");
    }

    #[test]
    fn builds_menu_bar_title() {
        assert_eq!(menu_bar_title(0), "⌨ 0");
        assert_eq!(menu_bar_title(1234), "⌨ 1,234");
    }

    #[test]
    fn icon_buffer_matches_dimensions() {
        let (w, h, rgba) = ICON_RGBA;
        assert_eq!(w, 32);
        assert_eq!(h, 32);
        assert_eq!(rgba.len(), (w * h * 4) as usize, "RGBA 缓冲区大小必须为 w*h*4");
    }

    #[test]
    fn icon_has_visible_and_transparent_pixels() {
        let (_, _, rgba) = ICON_RGBA;
        let alphas: Vec<u8> = rgba.chunks_exact(4).map(|px| px[3]).collect();
        assert!(alphas.iter().any(|&a| a > 200), "应存在不透明像素");
        assert!(alphas.iter().any(|&a| a < 20), "应存在透明像素（模板图标需要镂空）");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test icons::`
Expected: 编译失败，`format_thousands` / `menu_bar_title` / `ICON_RGBA` 未定义

- [ ] **Step 3: 实现图标与标题**

写 `src/icons.rs`：

```rust
//! 菜单栏图标（代码生成，无外部图像依赖）与标题格式化。

/// 图标边长。
const SIZE: u32 = 32;

/// 32×32 RGBA 像素，编译期生成：圆角方块轮廓 + 三条横线，形似键盘。
pub const ICON_RGBA: (u32, u32, &[u8]) = (SIZE, SIZE, &build_icon());

/// 生成模板图标像素：白色不透明为可见区域，alpha=0 为透明。
const fn build_icon() -> [u8; (SIZE * SIZE * 4) as usize] {
    let mut buf = [0u8; (SIZE * SIZE * 4) as usize];
    let mut y = 0u32;
    while y < SIZE {
        let mut x = 0u32;
        while x < SIZE {
            let visible = is_keyboard_pixel(x, y);
            let idx = ((y * SIZE + x) * 4) as usize;
            // 模板图标：可见部分为纯白，透明部分 alpha=0
            buf[idx] = 255;
            buf[idx + 1] = 255;
            buf[idx + 2] = 255;
            buf[idx + 3] = if visible { 255 } else { 0 };
            x += 1;
        }
        y += 1;
    }
    buf
}

/// 判断像素是否属于图形：外框圆角矩形 + 内部两条横线挖空。
const fn is_keyboard_pixel(x: u32, y: u32) -> bool {
    // 外框范围（留 2px 边距）
    let in_outer = x >= 2 && x < SIZE - 2 && y >= 6 && y < SIZE - 6;
    if !in_outer {
        return false;
    }
    // 圆角：四角 2×2 剔除
    let corner = (x < 4 || x >= SIZE - 4) && (y < 8 || y >= SIZE - 8);
    if corner {
        return false;
    }
    // 内部三条横向镂空线，形成键盘键位感
    let inner_y = y >= 9 && y < SIZE - 9;
    let on_line = inner_y
        && ((y == 12 || y == 13) || (y == 19 || y == 20));
    !on_line
}

/// 千分位格式化。
pub fn format_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 菜单栏标题：键盘符号 + 今日汉字数。
pub fn menu_bar_title(today_chars: u64) -> String {
    format!("⌨ {}", format_thousands(today_chars))
}
```

把 `src/lib.rs` 改为：

```rust
pub mod icons;
pub mod rime;
pub mod server;
pub mod store;
pub mod sync;
pub mod util;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test icons::`
Expected: PASS，4 个测试全部通过

- [ ] **Step 5: 提交**

```bash
git add src/icons.rs src/lib.rs
git commit -m "feat(icons): 代码生成菜单栏模板图标与千分位标题"
```

- [ ] **Step 6: 写菜单栏事件循环**

写 `src/tray.rs`：

```rust
//! 菜单栏入口与事件循环。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::icons::{menu_bar_title, ICON_RGBA};
use crate::sync::{sync_once, SyncState};
use crate::store::Store;

/// 同步线程唤醒 UI 的事件。
#[derive(Debug, Clone, Copy)]
pub enum UserEvent {
    /// 已同步完毕，请刷新菜单栏数字
    Synced,
}

const MENU_OPEN: &str = "open";
const MENU_SYNC: &str = "sync";
const MENU_DATA_DIR: &str = "data-dir";
const MENU_QUIT: &str = "quit";

const SYNC_INTERVAL: Duration = Duration::from_secs(60);

fn open_in_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    let _ = std::process::Command::new("open").arg(url).spawn();
}

fn open_data_dir(dir: &PathBuf) {
    let _ = std::process::Command::new("open").arg(dir).spawn();
}

fn build_icon() -> Icon {
    let (w, h, rgba) = ICON_RGBA;
    Icon::from_rgba(rgba.to_vec(), w, h).expect("内嵌图标必须是合法 RGBA")
}

/// 菜单栏标题所用的今日字数。
fn today_chars(state: &SyncState) -> u64 {
    let (days, _, _) = state.snapshot();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    days.get(&today).map(|d| d.characters).unwrap_or(0)
}

/// 运行菜单栏应用，永不返回。
pub fn run_app(
    rime_dir: PathBuf,
    data_dir: PathBuf,
    state: SyncState,
    port: u16,
) -> ! {
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();

    // 后台同步线程：立即同步一次，之后按间隔循环
    {
        let state = state.clone();
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let Ok(store) = Store::open(&data_dir) else {
                return;
            };
            loop {
                let outcome = sync_once(&rime_dir, &store);
                state.replace(outcome);
                let _ = proxy.send_event(UserEvent::Synced);
                std::thread::sleep(SYNC_INTERVAL);
            }
        });
    }

    let mut tray: Option<tray_icon::TrayIcon> = None;
    let mut menu_open: Option<MenuItem> = None;
    let mut menu_sync: Option<MenuItem> = None;
    let mut menu_data_dir: Option<MenuItem> = None;
    let mut menu_quit: Option<MenuItem> = None;
    let mut next_sync = Instant::now() + SYNC_INTERVAL;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_sync);

        match event {
            Event::NewEvents(cause) => {
                let timer_fired = matches!(
                    cause,
                    StartCause::Init | StartCause::ResumeTimeReached { .. }
                );
                if timer_fired && Instant::now() >= next_sync {
                    next_sync = Instant::now() + SYNC_INTERVAL;
                    // 同步线程独立运行，这里只负责重排定时器
                }

                if tray.is_none() {
                    let menu = Menu::new();
                    let open = MenuItem::new("打开统计", true, None);
                    let sync_now = MenuItem::new("立即同步", true, None);
                    let data = MenuItem::new("打开数据目录", true, None);
                    let quit = MenuItem::new("退出", true, None);
                    let _ = menu.append(&open);
                    let _ = menu.append(&sync_now);
                    let _ = menu.append(&data);
                    let _ = menu.append(&quit);

                    let icon = TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_menu_on_left_click(false)
                        .with_icon_as_template(true)
                        .with_icon(build_icon())
                        .with_tooltip("Rime 打字统计")
                        .with_title(menu_bar_title(today_chars(&state)))
                        .build();

                    if let Ok(icon) = icon {
                        tray = Some(icon);
                        menu_open = Some(open);
                        menu_sync = Some(sync_now);
                        menu_data_dir = Some(data);
                        menu_quit = Some(quit);
                    }
                }
            }
            Event::MainEventsCleared => {
                // 刷新标题
                if let Some(icon) = tray.as_ref() {
                    let title = menu_bar_title(today_chars(&state));
                    icon.set_title(Some(title));
                }

                // 托盘左键：打开网页
                while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        open_in_browser(port);
                    }
                }

                // 菜单项
                while let Ok(MenuEvent { id }) = MenuEvent::receiver().try_recv() {
                    if Some(&id) == menu_quit.as_ref().map(|m| m.id()) {
                        *control_flow = ControlFlow::Exit;
                    } else if Some(&id) == menu_open.as_ref().map(|m| m.id()) {
                        open_in_browser(port);
                    } else if Some(&id) == menu_sync.as_ref().map(|m| m.id()) {
                        let (rime_dir, data_dir) = (rime_dir.clone(), data_dir.clone());
                        let state = state.clone();
                        std::thread::spawn(move || {
                            if let Ok(store) = Store::open(&data_dir) {
                                state.replace(sync_once(&rime_dir, &store));
                            }
                        });
                    } else if Some(&id) == menu_data_dir.as_ref().map(|m| m.id()) {
                        open_data_dir(&data_dir);
                    }
                }
            }
            Event::UserEvent(UserEvent::Synced) => {
                // 标题在 MainEventsCleared 中统一刷新
            }
            Event::LoopDestroyed => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    })
}
```

把 `src/lib.rs` 追加：

```rust
pub mod tray;
```

- [ ] **Step 7: 编译验证**

Run: `cargo build`
Expected: 编译通过，无 error。

- [ ] **Step 8: 提交**

```bash
git add src/tray.rs src/lib.rs
git commit -m "feat(tray): 实现菜单栏图标、标题与右键菜单"
```

---

### Task 9: 仪表盘前端

**Files:**
- Modify: `src/web/index.html`（整体替换占位内容）
- Test: 手动验证（见 Step 4）

**Interfaces:**
- Consumes: `GET /api/days` 的 JSON（`sync.last_sync_unix`、`sync.error`、`days[].day/characters/keystrokes/commits/speed_cpm`）
- Produces: 无（终端产物）

**设计要求**（来自 spec 第 9 节与 dataviz 规范）：
- 单文件，无框架、无 CDN、无外部字体。
- 深色优先，同时响应 `prefers-color-scheme`，并提供手动主题切换。
- 已校验调色板（`validate_palette.js --mode dark` 全 PASS）：
  `--series-1: #3987e5`、`--series-2: #d95926`、`--series-3: #199e70`；深色表面 `#1a1a19`，主文字 `#f0efec`，次文字 `#c3c2b7`。
- 线宽 2px、圆点半径 ≥4 且带 2px 表面色描边、柱宽 ≤24px 顶端 4px 圆角；网格线 1px 实线且退到背景一档。
- **禁止双 Y 轴**；击键数只进浮层与表格，不进主图。
- ≥2 序列时必须有图例；单序列不设图例。
- hover 十字准线 + 浮层；提供表格视图开关（无障碍兜底）。
- 聚合在前端完成：日 → 周（ISO 周，周一起）→ 月。

- [ ] **Step 1: 写入完整仪表盘**

把 `src/web/index.html` 整体替换为：

```html
<!DOCTYPE html>
<html lang="zh-CN" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Rime 打字统计</title>
<style>
  :root {
    color-scheme: dark;
    --surface-1: #1a1a19;
    --surface-2: #232322;
    --grid: #2f2f2d;
    --text-primary: #f0efec;
    --text-secondary: #c3c2b7;
    --text-muted: #8a8a82;
    --series-1: #3987e5;
    --series-2: #d95926;
    --series-3: #199e70;
  }
  :root[data-theme="light"] {
    color-scheme: light;
    --surface-1: #fcfcfb;
    --surface-2: #f2f2f0;
    --grid: #ddddda;
    --text-primary: #0b0b0b;
    --text-secondary: #52514e;
    --text-muted: #7a7a74;
    --series-1: #2a78d6;
    --series-2: #eb6834;
    --series-3: #1baf7a;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    background: var(--surface-1);
    color: var(--text-primary);
    font: 14px/1.5 -apple-system, "Helvetica Neue", "PingFang SC", sans-serif;
  }
  main { max-width: 1080px; margin: 0 auto; padding: 28px 24px 64px; }
  header {
    display: flex; align-items: baseline; justify-content: space-between;
    gap: 16px; flex-wrap: wrap; margin-bottom: 24px;
  }
  h1 { font-size: 19px; font-weight: 600; margin: 0; letter-spacing: .01em; }
  .meta { color: var(--text-muted); font-size: 12px; }
  .meta .err { color: var(--series-2); }
  .toolbar { display: flex; gap: 8px; align-items: center; }
  button {
    background: var(--surface-2); color: var(--text-secondary);
    border: 1px solid var(--grid); border-radius: 6px;
    padding: 5px 11px; font: inherit; font-size: 12px; cursor: pointer;
  }
  button:hover { color: var(--text-primary); }
  button[aria-pressed="true"] {
    background: var(--series-1); border-color: var(--series-1); color: #fff;
  }
  .kpis {
    display: grid; gap: 1px; background: var(--grid);
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    border: 1px solid var(--grid); border-radius: 10px; overflow: hidden;
    margin-bottom: 32px;
  }
  .kpi { background: var(--surface-1); padding: 16px 18px; }
  .kpi .label { color: var(--text-muted); font-size: 12px; margin-bottom: 6px; }
  .kpi .value {
    font-size: 26px; font-weight: 600; letter-spacing: -.01em;
    font-variant-numeric: tabular-nums;
  }
  .kpi .delta { font-size: 12px; color: var(--text-secondary); margin-top: 2px; }
  .kpi .delta.up { color: var(--series-3); }
  .kpi .delta.down { color: var(--series-2); }
  .card {
    background: var(--surface-1); border: 1px solid var(--grid);
    border-radius: 10px; padding: 18px 18px 12px; margin-bottom: 24px;
  }
  .card-head {
    display: flex; justify-content: space-between; align-items: center;
    gap: 12px; flex-wrap: wrap; margin-bottom: 14px;
  }
  .card-title { font-size: 13px; font-weight: 600; color: var(--text-secondary); }
  .segments { display: flex; gap: 6px; flex-wrap: wrap; }
  svg { display: block; width: 100%; height: auto; overflow: visible; }
  .axis-label { fill: var(--text-muted); font-size: 11px; }
  .gridline { stroke: var(--grid); stroke-width: 1; }
  .crosshair { stroke: var(--text-muted); stroke-width: 1; stroke-dasharray: 3 3; }
  .tooltip {
    position: fixed; pointer-events: none; z-index: 10; opacity: 0;
    background: var(--surface-2); border: 1px solid var(--grid);
    border-radius: 8px; padding: 9px 12px; font-size: 12px;
    box-shadow: 0 6px 20px rgba(0,0,0,.35); transition: opacity .1s;
  }
  .tooltip .t-day { font-weight: 600; margin-bottom: 4px; }
  .tooltip .t-row { color: var(--text-secondary); display: flex; justify-content: space-between; gap: 18px; }
  .tooltip .t-row b { color: var(--text-primary); font-variant-numeric: tabular-nums; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: right; padding: 7px 10px; border-bottom: 1px solid var(--grid); }
  th:first-child, td:first-child { text-align: left; }
  th { color: var(--text-muted); font-weight: 500; font-size: 12px; }
  td { font-variant-numeric: tabular-nums; }
  .empty { color: var(--text-muted); padding: 28px 0; text-align: center; }
  .hint { color: var(--text-muted); font-size: 12px; margin-top: 10px; }
</style>
</head>
<body>
<main>
  <header>
    <div>
      <h1>Rime 打字统计</h1>
      <div class="meta" id="meta">加载中…</div>
    </div>
    <div class="toolbar">
      <button id="theme" type="button">切换到浅色</button>
      <button id="table-toggle" type="button" aria-pressed="false">表格视图</button>
    </div>
  </header>

  <section class="kpis" id="kpis" aria-label="关键指标"></section>

  <section class="card">
    <div class="card-head">
      <div class="card-title" id="chart-title">字数趋势</div>
      <div class="segments" id="granularity" role="group" aria-label="聚合粒度">
        <button type="button" data-g="day" aria-pressed="true">日</button>
        <button type="button" data-g="week" aria-pressed="false">周</button>
        <button type="button" data-g="month" aria-pressed="false">月</button>
      </div>
      <div class="segments" id="range" role="group" aria-label="时间范围">
        <button type="button" data-r="30" aria-pressed="true">近 30 天</button>
        <button type="button" data-r="90" aria-pressed="false">近 90 天</button>
        <button type="button" data-r="365" aria-pressed="false">近一年</button>
        <button type="button" data-r="all" aria-pressed="false">全部</button>
      </div>
    </div>
    <svg id="chart" viewBox="0 0 900 340" role="img" aria-label="字数随时间变化趋势"></svg>
    <div class="hint" id="chart-hint"></div>
  </section>

  <section class="card" id="table-card" hidden>
    <div class="card-head"><div class="card-title">数据表</div></div>
    <table id="table"><thead></thead><tbody></tbody></table>
  </section>
</main>
<div class="tooltip" id="tooltip" role="status" aria-live="polite"></div>

<script>
(function () {
  "use strict";

  const state = { days: [], granularity: "day", range: "30", error: null, lastSync: 0 };

  const el = (id) => document.getElementById(id);
  const fmt = (n) => Number(n || 0).toLocaleString("zh-CN");
  const pad = (n) => String(n).padStart(2, "0");
  const toISO = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const parseISO = (s) => {
    const [y, m, d] = s.split("-").map(Number);
    return new Date(y, m - 1, d);
  };

  // ---------- 数据访问 ----------
  async function load() {
    try {
      const res = await fetch("/api/days", { cache: "no-store" });
      if (!res.ok) throw new Error("HTTP " + res.status);
      const data = await res.json();
      state.days = data.days || [];
      state.error = data.sync ? data.sync.error : null;
      state.lastSync = data.sync ? data.sync.last_sync_unix : 0;
    } catch (e) {
      state.error = "无法连接本地服务：" + e.message;
    }
    renderMeta();
    renderKpis();
    renderChart();
    renderTable();
  }

  function renderMeta() {
    const parts = [];
    if (state.lastSync) {
      parts.push("最后同步 " + new Date(state.lastSync * 1000).toLocaleTimeString("zh-CN"));
    }
    if (state.error) {
      parts.push('<span class="err">' + escapeHtml(state.error) + "</span>");
    }
    el("meta").innerHTML = parts.join(" · ") || "尚无数据";
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({
      "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
    })[c]);
  }

  // ---------- 聚合 ----------
  function sumRange(startISO, endISO) {
    let chars = 0, keys = 0;
    for (const d of state.days) {
      if (d.day >= startISO && d.day <= endISO) {
        chars += d.characters || 0;
        keys += d.keystrokes || 0;
      }
    }
    return { chars, keys };
  }

  function isoWeekStart(date) {
    const d = new Date(date);
    const dow = (d.getDay() + 6) % 7; // 周一为 0
    d.setDate(d.getDate() - dow);
    return d;
  }

  function aggregate(granularity) {
    const buckets = new Map();
    for (const d of state.days) {
      const date = parseISO(d.day);
      let key;
      if (granularity === "week") {
        key = toISO(isoWeekStart(date));
      } else if (granularity === "month") {
        key = d.day.slice(0, 7);
      } else {
        key = d.day;
      }
      const cur = buckets.get(key) || { key, characters: 0, keystrokes: 0, commits: 0, speedSum: 0, speedN: 0 };
      cur.characters += d.characters || 0;
      cur.keystrokes += d.keystrokes || 0;
      cur.commits += d.commits || 0;
      if (d.speed_cpm) { cur.speedSum += d.speed_cpm; cur.speedN += 1; }
      buckets.set(key, cur);
    }
    let list = Array.from(buckets.values()).sort((a, b) => (a.key < b.key ? -1 : 1));
    list.forEach((b) => { b.speed = b.speedN ? Math.round(b.speedSum / b.speedN) : 0; });

    if (state.range !== "all") {
      const n = Number(state.range);
      const cutoff = new Date();
      cutoff.setDate(cutoff.getDate() - n);
      const cutoffISO = toISO(cutoff);
      list = list.filter((b) => (b.key.length === 10 ? b.key : b.key + "-01") >= cutoffISO);
    }
    return list;
  }

  // ---------- KPI ----------
  function renderKpis() {
    const today = new Date();
    const todayISO = toISO(today);
    const yesterday = new Date(today); yesterday.setDate(today.getDate() - 1);
    const yISO = toISO(yesterday);

    const weekStart = isoWeekStart(today);
    const weekS = sumRange(toISO(weekStart), todayISO).chars;
    const monthS = sumRange(todayISO.slice(0, 7) + "-01", todayISO).chars;
    const total = state.days.reduce((a, d) => a + (d.characters || 0), 0);
    const todayRec = state.days.find((d) => d.day === todayISO);
    const yRec = state.days.find((d) => d.day === yISO);
    const todayChars = todayRec ? todayRec.characters : 0;
    const yChars = yRec ? yRec.characters : 0;

    let deltaHTML = "";
    if (yChars > 0) {
      const pct = Math.round(((todayChars - yChars) / yChars) * 100);
      const cls = pct >= 0 ? "up" : "down";
      deltaHTML = `<div class="delta ${cls}">较昨日 ${pct >= 0 ? "+" : ""}${pct}%</div>`;
    } else {
      deltaHTML = `<div class="delta">昨日 ${fmt(yChars)}</div>`;
    }

    const speeds = state.days.filter((d) => d.speed_cpm > 0).map((d) => d.speed_cpm);
    const avgSpeed = speeds.length
      ? Math.round(speeds.reduce((a, b) => a + b, 0) / speeds.length)
      : 0;

    const items = [
      { label: "今日", value: fmt(todayChars), extra: deltaHTML },
      { label: "本周", value: fmt(weekS), extra: "" },
      { label: "本月", value: fmt(monthS), extra: "" },
      { label: "累计", value: fmt(total), extra: `<div class="delta">${state.days.length} 天有记录</div>` },
      { label: "平均速度", value: fmt(avgSpeed), extra: '<div class="delta">字 / 分钟</div>' },
    ];

    el("kpis").innerHTML = items.map((i) => `
      <div class="kpi">
        <div class="label">${i.label}</div>
        <div class="value">${i.value}</div>
        ${i.extra}
      </div>`).join("");
  }

  // ---------- 图表 ----------
  const SVG_NS = "http://www.w3.org/2000/svg";
  const W = 900, H = 340, M = { top: 16, right: 16, bottom: 34, left: 62 };
  const plotW = W - M.left - M.right;
  const plotH = H - M.top - M.bottom;

  function svgEl(tag, attrs) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const k in attrs) node.setAttribute(k, attrs[k]);
    return node;
  }

  function niceMax(v) {
    if (v <= 0) return 10;
    const mag = Math.pow(10, Math.floor(Math.log10(v)));
    const norm = v / mag;
    const step = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10;
    return step * mag;
  }

  function renderChart() {
    const svg = el("chart");
    svg.innerHTML = "";
    const data = aggregate(state.granularity);
    const gLabel = { day: "每日", week: "每周", month: "每月" }[state.granularity];
    el("chart-title").textContent = `${gLabel}字数`;

    if (!data.length) {
      svg.appendChild(svgEl("text", {
        x: W / 2, y: H / 2, "text-anchor": "middle", class: "axis-label",
      })).textContent = "暂无数据";
      el("chart-hint").textContent = "";
      return;
    }

    const max = niceMax(Math.max(...data.map((d) => d.characters)));
    const xOf = (i) => M.left + (data.length === 1 ? plotW / 2 : (i / (data.length - 1)) * plotW);
    const yOf = (v) => M.top + plotH - (v / max) * plotH;

    // 网格线与 Y 轴刻度
    const ticks = 4;
    for (let t = 0; t <= ticks; t++) {
      const v = (max / ticks) * t;
      const y = yOf(v);
      svg.appendChild(svgEl("line", {
        x1: M.left, x2: W - M.right, y1: y, y2: y, class: "gridline",
      }));
      const label = svgEl("text", {
        x: M.left - 10, y: y + 4, "text-anchor": "end", class: "axis-label",
      });
      label.textContent = fmt(Math.round(v));
      svg.appendChild(label);
    }

    // X 轴标签（首、中、尾，避免拥挤）
    const xIndexes = data.length <= 3
      ? data.map((_, i) => i)
      : [0, Math.floor((data.length - 1) / 2), data.length - 1];
    for (const i of new Set(xIndexes)) {
      const label = svgEl("text", {
        x: xOf(i), y: H - 10, "text-anchor": "middle", class: "axis-label",
      });
      label.textContent = data[i].key;
      svg.appendChild(label);
    }

    const isBar = state.granularity !== "day";

    if (isBar) {
      const slot = plotW / data.length;
      const barW = Math.min(24, slot * 0.6);
      data.forEach((d, i) => {
        const x = M.left + slot * i + (slot - barW) / 2;
        const y = yOf(d.characters);
        const h = M.top + plotH - y;
        if (h <= 0) return;
        svg.appendChild(svgEl("path", {
          d: roundedTopRect(x, y, barW, h, 4),
          fill: "var(--series-1)",
        }));
      });
    } else {
      // 面积渐变
      const grad = svgEl("linearGradient", { id: "area", x1: "0", y1: "0", x2: "0", y2: "1" });
      grad.appendChild(svgEl("stop", { offset: "0", "stop-color": "var(--series-1)", "stop-opacity": "0.18" }));
      grad.appendChild(svgEl("stop", { offset: "1", "stop-color": "var(--series-1)", "stop-opacity": "0" }));
      const defs = svgEl("defs", {});
      defs.appendChild(grad);
      svg.appendChild(defs);

      let linePath = "";
      let areaPath = "";
      data.forEach((d, i) => {
        const x = xOf(i), y = yOf(d.characters);
        linePath += (i === 0 ? "M" : "L") + x + "," + y;
        if (i === 0) areaPath = `M${x},${M.top + plotH} L${x},${y}`;
        else areaPath += `L${x},${y}`;
      });
      areaPath += `L${xOf(data.length - 1)},${M.top + plotH} Z`;

      svg.appendChild(svgEl("path", { d: areaPath, fill: "url(#area)" }));
      svg.appendChild(svgEl("path", {
        d: linePath, fill: "none", stroke: "var(--series-1)",
        "stroke-width": "2", "stroke-linejoin": "round", "stroke-linecap": "round",
      }));

      // 末端圆点：半径 4 + 2px 表面色描边
      const last = data.length - 1;
      svg.appendChild(svgEl("circle", {
        cx: xOf(last), cy: yOf(data[last].characters), r: 4,
        fill: "var(--series-1)", stroke: "var(--surface-1)", "stroke-width": "2",
      }));
    }

    // 交互层
    const crosshair = svgEl("line", {
      y1: M.top, y2: M.top + plotH, class: "crosshair", opacity: "0",
    });
    svg.appendChild(crosshair);

    const hit = svgEl("rect", {
      x: M.left, y: M.top, width: plotW, height: plotH, fill: "transparent",
    });
    svg.appendChild(hit);

    const tip = el("tooltip");
    hit.addEventListener("mousemove", (ev) => {
      const rect = svg.getBoundingClientRect();
      const relX = ((ev.clientX - rect.left) / rect.width) * W;
      let idx;
      if (isBar) {
        idx = Math.floor(((relX - M.left) / plotW) * data.length);
      } else {
        idx = Math.round(((relX - M.left) / plotW) * (data.length - 1));
      }
      idx = Math.max(0, Math.min(data.length - 1, idx));
      const d = data[idx];
      const cx = isBar
        ? M.left + (plotW / data.length) * (idx + 0.5)
        : xOf(idx);

      crosshair.setAttribute("x1", cx);
      crosshair.setAttribute("x2", cx);
      crosshair.setAttribute("opacity", "1");

      tip.innerHTML = `
        <div class="t-day">${escapeHtml(d.key)}</div>
        <div class="t-row"><span>字数</span><b>${fmt(d.characters)}</b></div>
        <div class="t-row"><span>击键</span><b>${fmt(d.keystrokes)}</b></div>
        <div class="t-row"><span>上屏次数</span><b>${fmt(d.commits)}</b></div>
        <div class="t-row"><span>速度</span><b>${fmt(d.speed)} 字/分</b></div>`;
      tip.style.left = Math.min(ev.clientX + 14, window.innerWidth - 190) + "px";
      tip.style.top = Math.max(ev.clientY - 70, 8) + "px";
      tip.style.opacity = "1";
    });
    hit.addEventListener("mouseleave", () => {
      crosshair.setAttribute("opacity", "0");
      tip.style.opacity = "0";
    });

    el("chart-hint").textContent =
      `共 ${data.length} 个${gLabel.slice(1)}区间；击键与速度见浮层与表格。`;
  }

  function roundedTopRect(x, y, w, h, r) {
    const rr = Math.min(r, w / 2, h);
    return `M${x},${y + h} L${x},${y + rr} Q${x},${y} ${x + rr},${y}` +
      ` L${x + w - rr},${y} Q${x + w},${y} ${x + w},${y + rr}` +
      ` L${x + w},${y + h} Z`;
  }

  // ---------- 表格 ----------
  function renderTable() {
    const data = aggregate(state.granularity);
    const table = el("table");
    table.querySelector("thead").innerHTML =
      `<tr><th>日期</th><th>字数</th><th>击键</th><th>上屏次数</th><th>速度（字/分）</th></tr>`;
    table.querySelector("tbody").innerHTML = data.length
      ? data.slice().reverse().map((d) => `
          <tr><td>${escapeHtml(d.key)}</td><td>${fmt(d.characters)}</td>
          <td>${fmt(d.keystrokes)}</td><td>${fmt(d.commits)}</td><td>${fmt(d.speed)}</td></tr>`).join("")
      : '<tr><td colspan="5" class="empty">暂无数据</td></tr>';
  }

  // ---------- 控件 ----------
  function bindSegments(id, attr, onPick) {
    const box = el(id);
    box.addEventListener("click", (ev) => {
      const btn = ev.target.closest("button");
      if (!btn) return;
      box.querySelectorAll("button").forEach((b) =>
        b.setAttribute("aria-pressed", String(b === btn)));
      onPick(btn.dataset[attr]);
    });
  }

  bindSegments("granularity", "g", (v) => {
    state.granularity = v;
    renderChart();
    renderTable();
  });
  bindSegments("range", "r", (v) => {
    state.range = v;
    renderChart();
    renderTable();
  });

  el("table-toggle").addEventListener("click", (ev) => {
    const btn = ev.currentTarget;
    const showing = btn.getAttribute("aria-pressed") === "true";
    btn.setAttribute("aria-pressed", String(!showing));
    el("table-card").hidden = showing;
  });

  el("theme").addEventListener("click", (ev) => {
    const root = document.documentElement;
    const next = root.getAttribute("data-theme") === "light" ? "dark" : "light";
    root.setAttribute("data-theme", next);
    ev.currentTarget.textContent = next === "light" ? "切换到深色" : "切换到浅色";
    renderChart();
  });

  // 每分钟自动刷新一次（与后端同步周期一致）
  load();
  setInterval(load, 60000);
})();
</script>
</body>
</html>
```

- [ ] **Step 2: 编译验证内嵌资源**

Run: `cargo build`
Expected: 编译通过（`include_str!` 能找到新文件）

- [ ] **Step 3: 启动服务做端到端检查**

Run: `cargo run -- --serve-only`（该参数在 Task 10 实现；若尚未实现，先执行 Task 10 再回到此步）
Expected: 控制台打印监听端口

- [ ] **Step 4: 浏览器核对**

Run: `curl -s http://127.0.0.1:8787/api/days | head -c 400`
Expected: 返回含 `"days":[{"day":"2026-09-...` 的 JSON

打开 `http://127.0.0.1:8787/`，逐项核对：
- KPI 五格数字与 `/api/days` 汇总一致
- 切「日 / 周 / 月」曲线形态随之变化，Y 轴刻度不重叠
- 切「近 30 天 / 90 天 / 一年 / 全部」数据范围随之变化
- 鼠标移到图上出现十字准线与浮层，离开后消失
- 点「表格视图」出现表格且与图一致；再点收起
- 点主题切换，深/浅色均无对比度问题

- [ ] **Step 5: 提交**

```bash
git add src/web/index.html
git commit -m "feat(web): 实现深色优先的打字统计仪表盘"
```

---

### Task 10: 程序入口、LaunchAgent 与端到端验证

**Files:**
- Modify: `src/main.rs`
- Create: `scripts/install.sh`
- Create: `scripts/uninstall.sh`
- Test: 手动端到端

**Interfaces:**
- Consumes: `store::Store`, `sync::{sync_once, SyncState}`, `server::{bind_first_available, serve}`, `tray::run_app`
- Produces: 可执行二进制 `rime-counter`

**行为**：
- 默认路径：Rime 目录 `~/Library/Rime`，数据目录 `~/Library/Application Support/rime-counter`；均可由环境变量 `RIME_COUNTER_RIME_DIR` / `RIME_COUNTER_DATA_DIR` 覆盖（测试与排障用）。
- 启动顺序：加载已存数据 → 起 HTTP 线程（端口 8787 起自动寻址）→ 菜单栏事件循环占用主线程。
- `--serve-only` 参数：只起 HTTP，不进事件循环，便于调试与自动化验证。
- `--dump` 参数：打印聚合结果后退出。

- [ ] **Step 1: 实现 main**

写 `src/main.rs`：

```rust
//! rime-counter：统计 Rime 每日打字字数，提供菜单栏入口与本地仪表盘。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use rime_counter::server::{bind_first_available, serve};
use rime_counter::store::Store;
use rime_counter::sync::{sync_once, to_iso_day, SyncState};
use rime_counter::tray;

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn rime_dir() -> PathBuf {
    std::env::var_os("RIME_COUNTER_RIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("Library/Rime"))
}

fn data_dir() -> PathBuf {
    std::env::var_os("RIME_COUNTER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home_dir().join("Library/Application Support/rime-counter")
        })
}

fn print_days(days: &std::collections::BTreeMap<String, rime_counter::rime::parser::DayStats>) {
    println!("共 {} 天有记录", days.len());
    let mut total = 0u64;
    for (day, stats) in days {
        total += stats.characters;
        println!(
            "{day}  字数 {:>7}  击键 {:>8}  上屏 {:>5}  速度 {:>3}",
            stats.characters, stats.keystrokes, stats.commits, stats.speed_cpm
        );
    }
    println!("合计 {total} 字");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let serve_only = args.iter().any(|a| a == "--serve-only");
    let dump = args.iter().any(|a| a == "--dump");

    let data_dir = data_dir();
    let rime_dir = rime_dir();

    if dump {
        match rime_counter::rime::parser::read_dir(&rime_dir) {
            Ok(raw) => {
                // 紧凑日期转 ISO 便于阅读
                let converted: std::collections::BTreeMap<_, _> = raw
                    .into_iter()
                    .filter_map(|(k, v)| to_iso_day(&k).map(|iso| (iso, v)))
                    .collect();
                print_days(&converted);
            }
            Err(e) => {
                eprintln!("解析 Rime 数据失败：{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let store = match Store::open(&data_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法打开数据目录 {}：{e}", data_dir.display());
            std::process::exit(1);
        }
    };

    // 启动即同步一次，让 UI 立刻有数据
    let outcome = sync_once(&rime_dir, &store);
    let initial = store.load().unwrap_or_else(|_| outcome.days.clone());

    if let Some(err) = &outcome.error {
        eprintln!("首次同步告警：{err}");
    }

    let state = SyncState::new(initial);
    state.replace(outcome);

    let (server, port) = match bind_first_available(8787, 11) {
        Some(pair) => pair,
        None => {
            eprintln!("无法在 8787..8797 绑定端口，请检查是否已有实例在运行");
            std::process::exit(1);
        }
    };
    println!("rime-counter 已启动：http://127.0.0.1:{port}/");

    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let state = state.clone();
        let shutdown = shutdown.clone();
        std::thread::spawn(move || serve(server, state, shutdown));
    }

    if serve_only {
        // 阻塞等待 Ctrl-C
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    tray::run_app(rime_dir, data_dir, state, port)
}
```

把 `src/lib.rs` 改为：

```rust
pub mod icons;
pub mod rime;
pub mod server;
pub mod store;
pub mod sync;
pub mod tray;
pub mod util;
```

- [ ] **Step 2: 编译并准备用 dump 验证真实数据**

先按上面的提醒清掉占位代码，然后：

Run: `cargo build --release`
Expected: 编译通过

Run: `./target/release/rime-counter --dump | head -12`
Expected: 打印 `共 N 天有记录` 与逐日明细，**首行日期应早于今天且字数非零**；把 N 记下来用于验收。

- [ ] **Step 3: 用 serve-only 验证 API 与页面**

Run（后台）: `RIME_COUNTER_DATA_DIR=/tmp/rime-counter-e2e ./target/release/rime-counter --serve-only &`
Run: `sleep 2 && curl -s http://127.0.0.1:8787/api/days | head -c 300`
Expected: JSON 非空，`days` 数组长度与 Step 2 的 N 一致

Run: `curl -s http://127.0.0.1:8787/ | head -5`
Expected: 返回 HTML（含 `<!DOCTYPE html>`）

然后 `kill %1` 停掉。

- [ ] **Step 4: 手动验证菜单栏**

Run: `./target/release/rime-counter`
Expected:
- 菜单栏出现 ⌨ 图标与今日字数，**Dock 中没有图标**
- 左键点击打开浏览器到统计页
- 右键出现四项菜单
- 「退出」能结束进程

按 `Ctrl-C` 结束。

- [ ] **Step 5: 编写安装脚本**

创建 `scripts/install.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

APP_NAME="rime-counter"
BUNDLE_ID="com.lovelock.rime-counter"
INSTALL_DIR="$HOME/Library/Application Support/rime-counter/bin"
BIN="$INSTALL_DIR/$APP_NAME"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"

echo "==> 编译 release 版本"
cargo build --release

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp "target/release/$APP_NAME" "$BIN"
chmod +x "$BIN"

echo "==> 写入 LaunchAgent：$PLIST"
mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$BUNDLE_ID</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>$HOME/Library/Logs/$APP_NAME.log</string>
  <key>StandardErrorPath</key>
  <string>$HOME/Library/Logs/$APP_NAME.log</string>
</dict>
</plist>
PLIST_EOF

echo "==> 加载 LaunchAgent"
launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"

echo "==> 完成。菜单栏应已出现 ⌨ 图标。"
echo "    卸载：scripts/uninstall.sh"
```

创建 `scripts/uninstall.sh`：

```bash
#!/usr/bin/env bash
set -euo pipefail

APP_NAME="rime-counter"
BUNDLE_ID="com.lovelock.rime-counter"
PLIST="$HOME/Library/LaunchAgents/$BUNDLE_ID.plist"
BIN_DIR="$HOME/Library/Application Support/rime-counter/bin"

if [[ -f "$PLIST" ]]; then
  echo "==> 停止并移除 LaunchAgent"
  launchctl bootout "gui/$(id -u)/$BUNDLE_ID" 2>/dev/null || true
  rm -f "$PLIST"
fi

echo "==> 删除二进制：$BIN_DIR"
rm -rf "$BIN_DIR"

echo "==> 数据（events.jsonl）已保留：$HOME/Library/Application Support/rime-counter"
```

- [ ] **Step 6: 赋予执行权限并验证脚本语法**

Run:
```bash
chmod +x scripts/install.sh scripts/uninstall.sh
bash -n scripts/install.sh && bash -n scripts/uninstall.sh && echo "脚本语法 OK"
```
Expected: 输出 `脚本语法 OK`

- [ ] **Step 7: 提交**

```bash
git add src/main.rs src/lib.rs scripts/
git commit -m "feat: 组装程序入口并添加 LaunchAgent 安装脚本"
```

---

### Task 11: 收尾检查

**Files:**
- Create: `README.md`
- Test: 全量测试

- [ ] **Step 1: 运行全量测试**

Run: `cargo test`
Expected: 全部 PASS，无 `test.skip` / `#[ignore]` 残留

- [ ] **Step 2: 运行 clippy**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: 无警告。若报 `needless_range_loop` 等纯风格问题，按提示修正而非 `#[allow]`。

- [ ] **Step 3: 检查无占位残留**

Run: `grep -rn "TODO\|todo!\|unimplemented!\|TBD" src/ scripts/ || echo "无占位符"`
Expected: 输出 `无占位符`

- [ ] **Step 4: 写 README**

创建 `README.md`：

```markdown
# rime-counter

统计每天在 macOS（Squirrel）上用 Rime 打出的字数，菜单栏常驻显示今日字数，点开查看日 / 周 / 月趋势。

## 原理

直接只读解析 `~/Library/Rime/stats.userdb`（LevelDB）中的每日统计键，
不复制快照、不获取文件锁、不修改任何 Rime 配置，因此在 Rime 正常输入时也能安全读取。
数据来自万象输入法的 `input_statistics` 模块，历史数据从启用那天起自动补齐。

本地只保存一份追加式日志 `events.jsonl`（在
`~/Library/Application Support/rime-counter/`），折叠时按天取最大值，重复同步幂等。

## 安装

```bash
scripts/install.sh
```

安装后菜单栏出现 ⌨ 图标与今日字数；左键打开仪表盘，右键有「立即同步 / 打开数据目录 / 退出」。

## 卸载

```bash
scripts/uninstall.sh
```

数据目录会保留，如需彻底清除手动删除 `~/Library/Application Support/rime-counter`。

## 开发

```bash
cargo test                     # 单元测试
cargo run -- --dump            # 打印解析到的每日数据
cargo run -- --serve-only      # 只起 HTTP 服务，便于调试前端
```

环境变量 `RIME_COUNTER_RIME_DIR` 与 `RIME_COUNTER_DATA_DIR` 可覆盖默认路径。

## 前提

`~/Library/Rime` 下已启用万象的 `input_statistics`（`wanxiang_pro.schema.yaml` 中的
`lua_translator@*wanxiang.input_statistics`）。未启用时没有统计数据可读。
```

- [ ] **Step 5: 提交**

```bash
git add README.md
git commit -m "docs: 添加 README 与使用说明"
```

---

## 自检记录

- **Spec 覆盖**：第 2 节事实 → Task 2/3/4；第 3 节存储 → Task 5；第 4 节模块划分 → 全部任务；第 5 节读取层 → Task 2/3/4；第 6 节同步 → Task 6；第 7 节 HTTP → Task 7；第 8 节菜单栏 → Task 8；第 9 节前端 → Task 9；第 10 节安装 → Task 10；第 11 节测试 → 各任务内联测试 + Task 11；第 13 节 YAGNI → 未引入任何额外功能。
- **类型一致性**：`DayStats` 在 rime 层、store 层、sync 层、server 层、tray 层统一使用 `crate::rime::parser::DayStats`；日期在 rime 层为 `YYYYMMDD`，从 sync 层起统一转为 `YYYY-MM-DD`。
- **占位检查**：本计划中所有代码块均为可直接写入的完整实现，无 `todo!()` / `unimplemented!()` / TBD。