# rime-counter 设计文档

日期：2026-11-06
状态：已确认，待实现

## 1. 目标

统计每天在 macOS（Squirrel）上用 Rime 打出的字数，提供菜单栏常驻入口与本地网页仪表盘，查看日 / 周 / 月趋势曲线。

**成功标准**：安装后无需任何手工操作，菜单栏实时显示今日字数；打开网页能在 1 秒内看到从有历史数据那天起的完整每日曲线；Rime 正在输入时统计不丢数、不阻塞、不影响输入体验。

## 2. 已确认的关键事实

实现前已实测，以下均为事实而非假设，后续设计依赖它们：

1. **数据已经存在。** 用户当前万象配置启用了 `wanxiang.input_statistics`（`wanxiang_pro.schema.yaml:95`），每日字数写入 `~/Library/Rime/stats.userdb`，历史可回溯至 2026-09-10。
2. **key 形态**：`statistics/day/YYYYMMDD/<field> \t<device_id>`，其中分隔符是空格 + TAB 两个字节，`device_id` 为 8 位小写十六进制（当前机器 `9b3d1f36`）。内部 LevelDB key 在其后追加 8 字节（`seq(7B LE) + type(1B)`）。
3. **value 形态**：`c=<int> d=0 t=0`，如 `c=483 d=0 t=0`。
4. **可用字段**：`text/characters`（汉字数）、`text/code_keystrokes`（编码击键）、`text/code_characters`、`commit_length/{1,2,3,4,5_plus}`、`speed_average/{characters,milliseconds,sessions}`、`speed_peak_window_10s/NNNN`。
5. **文件格式为标准 LevelDB**：`.ldb` footer magic `0xdb4775248b80fb57`；WAL `.log` 为标准 record framing（7 字节头：CRC32C + len16 + type8）。**所有 block 的压缩类型为 `none`**，无需 snappy。
6. **`LOCK` 是真锁**：Squirrel 进程持有 `fcntl` 排他锁（实测 `EAGAIN`）。这**只**影响通过 LevelDB 库 open 目录的方式。
7. **多设备**：key 尾部带 `device_id`，同一 field 多设备需按天求和。
8. **工具链**：cargo 1.98.1；crates 走 ustc 镜像，`tray-icon 0.19.3` / `tao 0.30.8` / `tiny_http 0.12.0` 均可拉取。

## 3. 存储位置与数据格式

数据目录：`~/Library/Application Support/rime-counter/`

```
rime-counter/
├── events.jsonl       # 追加日志（唯一数据源）
└── meta.json          # { "last_sync_unix": 1234567890, "last_error": null }
```

`events.jsonl` 每行一个 JSON 对象，追加写入，**永不修改历史行**：

```json
{"day":"2026-11-06","characters":1234,"keystrokes":5678,"commits":210,"speed_cpm":78}
```

设计要点：

- **单调递增 → 幂等。** 同步得到的是"某天截至目前的累计值"，折叠时对同一天取各字段的 `max()`。因此重复同步、乱序同步、进程重启后重放都不会导致数值虚高。
- **只在变化时追加。** 与内存中上次值逐字段比较，全相同则跳过，避免文件无意义膨胀（每天实际写入约 1~5 次/分钟粒度下也仅数百行）。
- **按天为最小粒度。** 不存分钟级时间序列——需求只到日/周/月，存更细粒度是 YAGNI。
- 折叠结果：`BTreeMap<NaiveDate, DayStats>`，十年数据约 3650 条，内存占用可忽略，全程内存聚合。

## 4. 模块划分

单 crate、单二进制，`src/` 下按职责分文件，每个文件只做一件事：

| 文件 | 职责 | 依赖 |
|---|---|---|
| `main.rs` | 组装：启动同步线程 + HTTP 服务 + 菜单栏，事件循环 | 全部 |
| `rime/mod.rs` | 归档扫描入口：列出 `.ldb` / `.log` 文件列表 | `std` |
| `rime/sstable.rs` | 解析 SSTable：footer → index block → data block → 条目 | `std` |
| `rime/wal.rs` | 解析 WAL：record framing，尾部残缺记录丢弃 | `std` |
| `rime/parser.rs` | 从 key/value 提取 `(day, field, device, value)`，按 `seq` 取每 key 最新 | 上述两者 |
| `store.rs` | JSONL 追加、读取、折叠成 `BTreeMap<Date, DayStats>` | `serde_json` |
| `sync.rs` | 定时任务：解析快照 → diff → 追加 → 更新内存态 | `rime`, `store` |
| `server.rs` | tiny_http：路由 `/`、`/api/days`、`/api/meta` | `tiny_http` |
| `web/index.html` | 内嵌单页仪表盘（`include_str!`） | 无 |
| `tray.rs` | 菜单栏图标、标题、左右键行为 | `tray-icon`, `tao` |

**`rime` 模块是纯函数式的**：输入文件路径，输出 `Vec<Record>`，不持有文件句柄、不写文件，可完全用构造的字节数组单测。

## 5. Rime 读取层

### 5.1 只读打开，不碰 LOCK

不使用 `rusty-leveldb` / `rocksdb` 等库——它们 open 目录时会尝试获取 `LOCK`，与 Squirrel 冲突必然失败。改为：

1. 扫描目录，按文件名匹配 `*.ldb` 与 `*.log`。
2. 每个文件以 `File::open` 只读打开，`read_to_end` 读入内存（当前最大 1.5 MB，可接受）。
3. 自行解析。

这既不触发 `LOCK`，也不需要复制快照，用户可以同时正常打字。

### 5.2 SSTable 解析

```
footer = 文件末尾 48 字节
  metaindex_handle(2 varint) + index_handle(2 varint) + padding(40) + magic(8)
index block → 每项 value 是一个 block handle(off, size)
data block → 条目序列 + restart array（末尾 4 字节 restart 个数）
条目 = shared(varint) + non_shared(varint) + value_len(varint) + key_delta + value
       key = 上一 key 的前缀(shared) + key_delta
block 末尾 5 字节为 type(1) + crc32(4)，type 恒为 0（未压缩）
```

只需读 index → data 两层；filter block（bloom）与 metaindex 无关统计，跳过。

### 5.3 WAL 解析

```
循环：crc32c(4) + length(2) + type(1) + payload(length)
type: 1=FULL, 2=FIRST, 3=MIDDLE, 4=LAST
```

- 单条 FULL 记录即完整；FIRST/MIDDLE/LAST 需拼接。
- **读到文件尾不足一条完整记录时，直接停止**——这是 Squirrel 正在写入的半条记录，属正常现象。
- WAL 中的 write batch 为 `seq(8B) + count(4B) + [type(1) + key(varint len) + value(varint len)]*`，`type=1` 为 put、`type=0` 为 delete。统计 key 从不删除，但解析器遇到 delete 仍按语义处理。

### 5.4 合并与取值

所有文件解析出的记录带 `(user_key, seq, value)`。合并策略：

- 以 `user_key` 分组，取 `seq` 最大者（`.log` 的 seq 天然大于所有 `.ldb`，因此无需额外版本判断）。
- key 解析正则：`^statistics/day/(\d{8})/(.+?) \t([0-9a-f]{8})$`。
- 同一天同一 field 跨设备求和。
- 只关心 `text/characters`、`text/code_keystrokes`、`commit_length/*`、`speed_average/*`，其余字段忽略。

## 6. 同步与一致性

- **周期**：每 60 秒一次，另在启动后立即执行一次。
- **独立线程**：解析 + JSONL 追加全在后台线程，菜单栏与 HTTP 主循环不被阻塞。
- **失败容忍**：任一步失败（文件正在轮转、格式异常）只记录 `last_error` 与 `last_sync_unix`，**保留上一次成功的内存态**，UI 继续可用。
- **锁竞争**：全程不持有任何文件锁；读到的文件若在读取过程中被 LevelDB compact 掉，`File::open` 会直接 `ENOENT`，本轮跳过即可。

## 7. HTTP 与 API

- 监听 `127.0.0.1:8787`，**仅绑定回环地址**，不暴露到局域网（数据含个人输入习惯）。
- `GET /` → `text/html`，内嵌仪表盘。
- `GET /api/days` → 全部日数据，按日期升序：

```json
{
  "sync": {"last_sync_unix": 1791796577, "error": null},
  "days": [
    {"day":"2026-09-10","characters":7714,"keystrokes":20000,"commits":990,"speed_cpm":75}
  ]
}
```

只返回有数据的日期；前端负责补齐空缺日（补 0）。不做分页、不做筛选参数——数据量小，一次全量最省事。

- 响应带 `Cache-Control: no-store`。
- 端口被占用时：自动尝试 `8787..8797`，选定后写回 `meta.json`，菜单栏点开时使用该端口。

## 8. 菜单栏

- 图标：内嵌 32×32 PNG（base64 解码进内存），`tray-icon` 设置 `icon_as_template(true)` 以适配浅/深色菜单栏。
- 标题：`⌨ 1,234`（今日汉字数，千分位）。数字每分钟随同步更新。
- 左键（鼠标抬起）：`open http://127.0.0.1:<port>/`。
- 右键：弹出菜单
  - `打开统计`
  - `立即同步`
  - `打开数据目录`
  - `退出`
- 事件循环用 `tao`，超时设为 1 秒，以驱动定时同步与 UI 刷新，避免忙等。

## 9. 前端仪表盘

单文件 HTML，无框架、无 CDN、无外部字体，`include_str!` 打进二进制。

**主题**：深色优先，同时适配 `prefers-color-scheme` 并提供手动切换。色值取自经过校验的深色调色板（`validate_palette.js --mode dark` 全部 PASS：CVD 最差相邻 ΔE 9.4，正常视觉 26.5）：

```
--surface-1: #1a1a19   --text-primary: #f0efec   --text-secondary: #c3c2b7
--series-1:  #3987e5   --series-2: #d95926       --series-3: #199e70
```

**结构**：

1. **KPI 行**：今日 / 本周 / 本月 / 累计 / 平均速度，每格含小字环比与迷你 sparkline。
2. **主图**：分段控件 `日 / 周 / 月` × `近 30 天 / 近 90 天 / 近一年 / 全部`。
   - 日视图：折线（2px）+ 10% 面积渐变，末端圆点 ≥8px 带 2px 背景色描边。
   - 周 / 月视图：柱图，柱宽 ≤24px，顶端 4px 圆角、底边方角。
   - 单序列，因此不设 legend（标题即说明）；网格线为 1px 实线、退到背景色一档。
   - hover 十字准线 + 浮层显示日期、字数、击键、速度。
3. **热力图**：近一年日历格，单色顺序色阶（`#3987e5` 由浅到深），空格为背景色。
4. **表格视图**：右上角开关，把当前聚合结果摊成表格（无障碍兜底，也便于复制）。

**明确不做**：双 Y 轴（字数与速度量纲不同，拆成两个图或只放浮层）；每个点都标数值；饼图。

聚合在前端完成：把 `/api/days` 的日数据按 ISO 周（周一起）/ 自然月折叠，实时跟随分段控件切换，无需重新请求。

## 10. 安装与自启

- `cargo build --release` 产物单文件，无运行时依赖。
- `scripts/install.sh`：
  1. 复制二进制到 `~/Library/Application Support/rime-counter/bin/rime-counter`。
  2. 写 `~/Library/LaunchAgents/com.lovelock.rime-counter.plist`（`RunAtLoad` + `KeepAlive`）。
  3. `launchctl bootstrap gui/$(id -u)` 加载。
- `scripts/uninstall.sh` 逆向清理（保留数据目录）。

## 11. 测试策略

| 层级 | 方式 |
|---|---|
| SSTable 解析 | 用真实 `.ldb` 截取的字节片段作 fixture（不含个人信息，仅 statistics key），断言条目数与若干已知 key/value |
| WAL 解析 | 手工构造 FULL / 跨块 FIRST+LAST / 尾部残缺 三种字节序列 |
| 合并取值 | 构造多文件多 seq 冲突，断言取最大 seq；构造多设备同日，断言求和 |
| JSONL 折叠 | 同一 day 多条递增记录 → 取 max；乱序输入 → 结果相同；空文件 → 空表 |
| 聚合 | 日→周、日→月折叠的边界（跨年周、月末 31 号、闰年 2 月） |
| 端到端 | 用 fixture 目录跑一次完整 sync，断言 `/api/days` JSON 结构与非空 |
| 手动 | 安装后观察菜单栏数字变化；打开网页核对与 Rime 内 `/rtj` 今日统计一致 |

## 12. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 万象 lua 未来改 key 格式 | 解析器对未知 field 静默忽略；仅 `text/characters` 缺失时告警并在 UI 显示"数据源无更新" |
| LevelDB 压缩策略被改为 snappy | 解析时检查 block type；非 0 则记录错误并在 UI 提示（不静默出错） |
| 用户换机器/多设备 | 按 device_id 求和；UI 不区分设备（视作个人总输入量） |
| 端口冲突 | 自动寻址 8787..8797 |
| 二进制长期运行内存 | 数据量级 < 1 MB，无累积增长；每次同步重建记录集 |

## 13. 明确不做（YAGNI）

- 分钟级/小时级时序、按应用或按词频统计
- 云端同步、多用户、远程访问
- 修改或回写 Rime 配置、自建 lua 上报
- 打包 `.app`、签名公证、Dock 图标
- 数据导出/导入 UI（JSONL 本身即导出格式）