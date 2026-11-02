# 存储管理

已实现：后端在 `src/backend/storage.rs`（扫描、报告、清理任务、三个接口），界面在 `frontend/src/components/StoragePanel.vue`。下文是它当前的行为，不是待办。

入口在**设置 →「存储空间」**（桌面侧栏的独立存储标签已移除；Dashboard 的存储卡片跳到这个设置分区）。

这一页只回答一个问题：**这台机器上，我为这个图库占了多少磁盘，其中哪些可以删掉再生成。**

## 管理员看成员

管理员在同一个面板里多一张成员表（`GET /api/storage/members`）：每个账号一行，显示总占用、原片、缩略图、AI 索引、统计时间和状态。成员表**只读缓存**，打开页面不会扫描任何人的数据根；"统计"按成员排队一个扫描任务（`storage-scan:{owner}` 去重，任务属于发起人，所以管理员能看到进度），"统计全部"一次最多排 4 个。

- `GET /api/storage/report?owner=<name>`：管理员可读任意成员，其他人只能读自己，越权返回 403；
- 管理员可以在成员视图里直接清理该成员的缓存：`POST /api/storage/cleanup` 带 `owner` 只有管理员能过（`CleanupRequest.owner`），删除范围按目标账号的缓存目录限定，任务去重键是 `storage-cleanup:{目标账号}`；确认框会写明目标账号，避免删错人；非管理员永远看不到别人的视图，请求别人的 `owner` 直接 403；
- darktable 预览按请求者分目录（`cache/darktable/{owner}/`），所以看共享相册产生的预览算请求者的，成员报告能逐张归属；升级前写在扁平 `cache/darktable/*.jpg` 的旧预览无法反推账号，保留到保留期 GC 删除，期间在成员表下方列为「共享缓存（legacy）」；
- `cache/exports` 仍是扁平临时目录（导出中断留下的残渣），同样列为共享缓存；
- 共享相册只算在拥有者名下（成员扫描以该成员的数据根为起点），所以各成员占用相加不会重复计算共享相册。

原片和 XMP 不在可删范围。缩略图、darktable 预览、视频转码可以删，下次查看时按需重建。AI 索引重建要跑模型，只读展示，不在这一版的可删范围。

和 Dashboard 的 `total_bytes` 不是一个口径：Dashboard 统计**可访问**相册里的原片（含别人分享给我的），存储页统计**我这块数据根加上为我生成的缓存**。两个数字不相等是正常的，页面上要写清楚差在哪，不能让用户以为掉数据了。

## 1. 场景

一个人在电脑端打开「存储」，看到自己占了 15.2 GB：原片 14.3 GB（jpg 10.1 GB、zip 1.3 GB、RAW 0.9 GB），其中相册内 8.1 GB、`albums/` 外的浏览目录 6.0 GB；拍摄时间集中在 2026 年 4 月。缩略图 454 MB，转码 381 MB。他展开「转码视频」，删掉一个 126 MB 的 H.264 缓存，磁盘立刻回收，下次播放同一个视频时按需重新转码。他清掉 11 个未归属缩略图。全程原片、XMP、AI 索引都没被碰；无权访问的相册、另一个账号的缓存、带 `../` 的名字在第一步就被拒绝。

## 2. 归属

| 类别 | 位置 | 算谁的 | 可删 |
| --- | --- | --- | --- |
| 原片与文件 | `dist/data/{me}/**` | 数据根的主人 | 否 |
| 相册内的照片、视频 | `dist/data/{me}/albums/{album}/**` | 同上 | 否 |
| 资源库（LUT、预设） | `dist/data/{me}/albums/__resource__/**` | 同上 | 否 |
| 缩略图 | `cache/thumbnails/{me}/*.{webp,avif,jpg}` | 缓存里记的所有者 | 是 |
| darktable 预览 | `cache/darktable/{owner}/*.jpg` | 目录 + key 里的请求者 | 是 |
| 视频转码 | `cache/video-encode/{me}/{codec}/*.mp4` | 视频所属的人 | 是 |
| 转码探测 | `cache/video-encode/{me}/probe/*.json` | 同上 | 是 |
| AI 索引与人脸裁切 | `cache/ai/{me}/albums/{cache_id}/**` | 相册所有者 | 本版否 |
| 索引、EXIF、清单、导出 | `cache/{index,exif,thumbnail-manifests,exports}/**` | 服务级，不按人切 | 本版否 |

三处不对称，必须写进文档也写进界面提示，否则统计看着像算错了：

- 看别人的共享相册时，缩略图写在**相册所有者**的目录下（`api_thumbnail` 里 `cache_owner = query.owner.unwrap_or(username)`），所以不占我的账，我也删不到。
- darktable 预览既按**请求者**记 key，也落在请求者的目录（`cache/darktable/{owner}/`），所以看共享相册产生的预览算我的，能删；目录归属让成员扫描不需要重算别人的 key。
- 转码写在**视频所属的人**的目录下（`resolve_video` 里的 `file_owner`），同样不是我的账。

共享给我的相册只显示数量，不显示体积，也不进按相册表：不把别人的目录形状搬到我的页面上，也不让我的清理动作有机会碰到别人的缓存。

## 3. 三个账本

### 3.1 原片与文件：走数据根，不只走索引

不能用相册索引当唯一来源。`dist/data/{me}/` 下面除了 `albums/` 还有文件浏览器能看到的目录（实测 `trans/` 1.98 GB、`测试/` 1.70 GB、`lirui.zip` 1.26 GB），索引里没有它们，而它们占了 42%。

一次遍历数据根，产出 `(相对路径, 字节数, mtime, 扩展名)`：

- 不跟随符号链接（`symlink_metadata`，`is_symlink` 就跳过并计数）。相册遍历里现有的 `collect_album_paths` 会跟随目录软链，统计不复用它。
- 跳过 `albums/__uploading/`：半成品上传还没落到相册。
- 扩展名小写；`jpeg` 归一成 `jpg`；没有扩展名归 `(none)`。
- `albums/{album}/…` 的路径首段决定相册；其余归到「相册外」，按首段目录分组（`trans`、`lirui.zip`、`阿那亚1`……）。`__resource__` 单列「资源库」，不当相册。

日期只从相册索引来，不猜；空、`1970-01-01`、格式不合法（比如 `2026-4`）都归「未纳入索引」。每本相册用 `load_existing_album_index(data_path, root_path, owner, dir_name)` 取，**不要自己拼 `{dir_name}.json`**：相册改名后 `album_meta.json` 的 `cache` 指针才是索引文件名（实测 `albums/tuned` 的 `cache` 是 `trans`）。没索引的相册照常计入格式和相册，但归到「未纳入索引」，不进年月；界面给一个走现有 `POST /api/albums/rebuild_index` 的提示。

相册外文件没有拍摄时间，用文件 mtime 会把它算到 `zip` 的复制日期上，所以不进年月。

### 3.2 缩略图：重算 key，求交集

缓存文件名就是 key（`{key}.{webp|avif|jpg}`），key 里带文件长度和 mtime：

```text
sha512("{cache_owner}|{cache_relative_path}|{size}|{src_len}|{src_mtime}|v4|{lumeno|darktable}")
```

所以源文件一改、一删，旧缩略图就立刻对不上任何源。归属的办法是反过来算：对数据根里每个图片/视频，按已知尺寸和生成器各算一遍 key，落在缓存文件名集合里就记账，并从集合里移除。集合里剩下的就是未归属。

已知尺寸从常量来，放在 `src/backend/constants.rs`：

```rust
/// 前端取图调用点用过的缩略图边长，存储统计按它归属缓存。
/// `0` 是全尺寸。宁可多列几档，也不能漏：漏一档就会把在用的缩略图算成未归属。
pub const THUMBNAIL_CACHE_SIZES: &[u32] =
    &[0, 96, 160, 240, 256, 320, 360, 400, 512, 640, 800, 1024, 1600];
```

`0` 是 `size=0` 的全尺寸；`96`/`256` 是文件浏览器；`240`/`400` 是 Dashboard 和相册封面；`360` 是网格；`800`/`1600` 是手机查看器。生成器两种都算（`lumeno`、`darktable`），因为 `generator` 是请求参数。**前端新增一个取图尺寸时，必须同时加进这张表**；漏一位不会丢数据，只会让正常缩略图出现在「未归属」里，等用户删掉再重建。实测这就是我第一次漏掉 `240` 和 `0` 时的结果：634 个文件被误判成孤儿。

### 3.3 darktable 预览：同一套，多一个 XMP 时间

```text
sha512("{requester}|dt|{cache_relative_path}|v2|{size}|{src_len}|{src_mtime}"
       + ("|xmp|{xmp_mtime}" | "|noxmp"))
```

`xmp_mtime` 要先按 `find_xmp_sidecar` 的两种形态找边车：`{文件名}.{扩展名}.xmp` 优先，其次是 `{文件名}.xmp`。每张照片多一到两次 `stat`，这是扫描里唯一无法从索引拿到的信息。尺寸用 `0`（母版）、缩略图边长表里的值，以及 `/api/albums/darktable_preview` 的默认 `1600`。

### 3.4 转码：同一条规则

```text
sha512("video-encode-v1|{cache_relative_path}|{src_len}|{src_mtime}")
```

`probe/{key}.json` 用同一个 key。归属到源视频后，按视频所属相册和拍摄月份记账。

### 3.5 一次扫描的五步

1. 列出 `cache/thumbnails/{me}/`、`cache/darktable/{me}/`、`cache/video-encode/{me}/{codec}/`、`cache/video-encode/{me}/probe/` 的文件名到集合。
2. 遍历数据根，得到每个文件的相对路径、字节数、mtime、扩展名；媒体文件另外取 XMP 边车的 mtime。
3. 对每个媒体文件按尺寸表算缩略图 key，按 XMP 有无算 darktable key，视频再算转码 key；命中就记账并从集合移除。
4. 集合剩下的按类别记成「未归属」，只存个数和字节数。
5. 聚合到相册、年月、格式。

复杂度是 `文件数 × (尺寸数 × 2 生成器 + 1 视频)` 次 sha512，输入一两百字节。十万张约 270 万次，`rayon` 并行后是秒级；它跑在任务里，请求线程不碰。

哈希公式不复制。现在 `thumbnail_cache_key` 已经是 `pub`，另外两处还内联在路径函数里：把 `darktable_preview_cache_path` 里的哈希抽成 `darktable_preview_cache_key(...)`，把 `video_encode::source_cache_key` 提成 `pub(crate)`，统计和真正生成缓存调同一个函数。公式只有一份，否则统计和缓存迟早对不上。

## 4. 报告

### 4.1 落盘

```text
cache/storage-reports/{owner}.json
```

只存聚合，不存文件清单：清单会随图库变大，而且用旧清单删文件本身就是个错误。删除时重新算一遍要删的集合。

报告带 `version` 和 `generated_at`。`version` 对不上、超过 24 小时、或者上一次清理完成过，都算过期。过期时接口照样返回旧报告，前端在顶上写「统计于 X 前」并给重新统计。

### 4.2 `GET /api/storage/report`

会话用现有的 `Token`。报告里只有当前用户自己的数据根和为自己生成的缓存，管理员没有例外。

```json
{
  "generated_at": 1761984000,
  "stale": false,
  "scanning": false,
  "totals": {
    "bytes": 15209428242,
    "files": 6324,
    "albums": { "files": 1397, "bytes": 8070261924 },
    "resources": { "files": 40, "bytes": 19384335 },
    "outside": { "files": 584, "bytes": 5995466551 },
    "temp": { "files": 32, "bytes": 247682099 },
    "thumbnails": { "files": 1451, "bytes": 454079236, "unattributed_files": 11, "unattributed_bytes": 3146786 },
    "darktable_previews": { "files": 14, "bytes": 13608969, "unattributed_files": 0, "unattributed_bytes": 0 },
    "transcodes": { "files": 2, "bytes": 380895911, "unattributed_files": 0, "unattributed_bytes": 0 },
    "transcode_probes": { "files": 3, "bytes": 753 },
    "ai": { "files": 2801, "bytes": 28048411 }
  },
  "formats": [
    { "ext": "jpg", "files": 1205, "bytes": 10086667242, "cache_bytes": 430000000 }
  ],
  "albums": [
    { "album": "测试", "name": "测试", "files": 498, "bytes": 2932807966,
      "thumbnails_bytes": 210000000, "transcodes_bytes": 126174010,
      "thumbnails_files": 405, "transcodes_files": 5,
      "indexed": true }
  ],
  "outside": [
    { "top": "trans", "files": 243, "bytes": 1982433323 }
  ],
  "months": [
    { "month": "2026-04", "files": 181, "bytes": 2345030328,
      "thumbnails_bytes": 62000000, "transcodes_bytes": 0 }
  ],
  "unindexed_albums": 0,
  "shared": { "album_count": 2 },
  "videos": [
    { "codec": "h264", "key": "…86 位…", "name": "PANA8405.MOV", "album": "测试",
      "bytes": 126173752, "duration_ms": 23520, "unattributed": false }
  ],
  "thumbnail_sizes": [
    { "size": 360, "files": 792, "bytes": 240000000 },
    { "size": 240, "files": 617, "bytes": 200000000 }
  ]
}
```

`videos` 是全量，转码本来就少；每行只给 basename，不给主机路径。缩略图逐条列没有意义，只按尺寸和相册聚合。

`formats` 里 `cache_bytes` 是「这个格式的原片对应的缩略图和转码」，用来解释「jpg 只有 10.1 GB，为什么缓存多出来这么多」。

相册行的 `transcodes_bytes` 和 `transcodes_files` 含转码探测文件（每个几百字节），所以按相册求和会比 `totals.transcodes` 多出探测那一份；`thumbnails_bytes` 同样含 darktable 预览。

### 4.3 扫描任务

`StorageScanTask` 照 `BackgroundThumbnailTask` 的样子接：`TaskBase`、`deduplication_key = "storage-scan:{owner}"`、可取消、可暂停。进度用文件数。第一步 `GET /api/storage/report` 找不到报告时自动排一次（和 `load_or_queue_album_index` 一个思路），有报告就等用户按「重新统计」。

## 5. 删除

### 5.1 作用域

| kind | target | 附加参数 | 删掉什么 |
| --- | --- | --- | --- |
| `thumbnails` | `unattributed` | — | 对不上任何当前源文件的缩略图 |
| `thumbnails` | `album` | `album` | 该相册照片对应的缩略图 |
| `thumbnails` | `size` | `size` | 某一档边长的缩略图 |
| `thumbnails` | `month` | `month` | 拍摄月份对应的缩略图 |
| `thumbnails` | `key` | `key` | 单个（详情里删某张的缓存） |
| `thumbnails` | `all` | — | 我的全部缩略图 |
| `transcodes` | `unattributed` | — | 对不上当前源视频的转码和探测 |
| `transcodes` | `album` / `month` / `key` / `all` | 同上 | 同上 |
| `transcodes` | `codec` | `codec` | 某一个编码（`av1`/`hevc`/`h264`） |

`darktable` 预览跟缩略图同一条流水线，`kind=thumbnails` 时一起算、一起删，报告里分开统计。它没有 owner 子目录，key 里又只有请求者的哈希，所以只统计和删除**能对上我这个数据根里源文件**的那些；对不上的既不计入总数，也不删——那可能是别人生成的预览，删了只是让别人重新生成一次，属于越界。

### 5.2 两步

界面上的按钮先给出报告里的估算（「将删除约 11 个文件，约 3 MB」），确认后起任务。任务自己重新算目标集合，实际删了多少用 `processed_count`、`processed_cost` 报给任务列表；数字和估算不一样不报错，界面在任务结束后刷新报告。

`POST /api/storage/cleanup` 带 `dry_run: true` 时跑同样的枚举但不 `remove_file`，结果写进任务的 `processed_count`、`processed_cost`。这是给「先看要删什么」的入口，不是必走路径。

### 5.3 路径闭合

删除任务只接收 `kind`、`target` 和筛选参数，**不接收路径**。要删的文件都由同一个归属函数算出来，每个都满足：

- 缩略图：`cache/thumbnails/{session_user}/` 的直接子文件，扩展名在 `webp|avif|jpg` 里。
- darktable：`cache/darktable/` 的直接子文件，且 key 里有当前用户名。
- 转码：`cache/video-encode/{session_user}/{codec}/` 的直接子文件，`codec` 必须在 `ENCODE_CODECS` 里。
- `key` 参数必须是 86 位 `[A-Za-z0-9_-]`；`codec` 走现成的 `is_encode_codec`；`size` 夹在 `THUMBNAIL_SIZE_MIN..=THUMBNAIL_SIZE_MAX` 或就是 `0`；`month` 匹配 `^\d{4}-\d{2}$`；`album` 只用来在这个用户的 `albums/` 里按名字比，不拼进任何路径。
- 每个路径在 `remove_file` 前再 `starts_with` 复核一次，和 `owner_cache_dir` 现在做的一样。

第一次进目录就用 `is_safe_cache_owner` 校验 `{session_user}`；它现在的规则（非空、≤16、不以 `.` 开头、只允许字母数字和下划线）正好挡住 `../`、`a/b`、`.hidden`。

### 5.4 并发与拒绝

以下情况返回 `BadRequest`，不起任务：

- `background-thumbnails:{owner}` 还在跑（`TaskManager::has_active_key`）：刚生成完又被删掉，纯浪费。
- 目标转码的 `video-encode:{输出路径}` 正在编码。
- 同一个 owner 已有 `storage-cleanup:{owner}` 在跑。

正在进行中的读取不会因为 unlink 失败：Linux 上已经打开的 fd 能读完；下一次请求发现文件没了，走正常生成路径。删除是幂等的，任务被取消时已删的不回滚，报告作废，下次扫描重新算。

### 5.5 删完之后

- 报告立刻标过期（写一次 `stale` 或者直接删掉），界面刷新时显示旧数据加一条「统计于 X 前」。
- 下次浏览时缩略图按需重建，播放视频时按需转码，原片和 XMP 不动。
- 现有的 `BackgroundThumbnailTask` 本来就会按 `cache/thumbnail-manifests/{owner}.json` 删过期 key（实测它把那 11 个孤儿删了）。存储页不是唯一清理路径：它管的是**马上**按相册、年月、尺寸删，以及把「能不能删、删了少多少」摆出来，不用等下一次后台扫描。开着后台缩略图模式时，删掉的缩略图会在一段时间后又被预热任务生成回来，这一点写在确认框里。
- 现有 `spawn_cache_gc` 只覆盖 `thumbnails`、`exif`、`darktable`、`exports`，不覆盖 `video-encode`，而且它只看文件时间，不知道缓存属于哪个相册、哪一档尺寸。这一次把 `video-encode` 按 `thumbnails` 那种「先按 owner 子目录、再按文件」的方式加进去，让「缓存保留天数」真的管到转码；手工清理负责它做不到的「按相册、按年月、按尺寸」。

## 6. 接口与路由

| 方法 | 路径 | 谁 | 作用 |
| --- | --- | --- | --- |
| GET | `/api/storage/report` | 有效会话 | 读自己的报告；没有就排队扫描 |
| POST | `/api/storage/scan` | 有效会话 | 重新统计，返回任务 uuid |
| POST | `/api/storage/cleanup` | 有效会话 | 按作用域清理自己的缓存 |

三个都走 `create_response`，错误走 `AppError`：缺会话 `Unauthorized`，参数不合法 `BadRequest`，相册不在自己根下 `NotFound`，任务冲突 `BadRequest`。响应和日志里只有 basename 和相册名，没有主机路径。

## 7. 前端

### 7.1 入口

电脑端侧栏加一项，放在 Dashboard 后面：`{ key: "storage", label: t("nav.storage"), icon: IconDatabase }`。Dashboard 的「存储」卡片加上点击后跳到这个 tab（`DashboardPanel` 的 `@navigate` 已经通了，卡片目前只是展示）。

手机这一版不出现这个入口。删除需要明细列表和二次确认，手机上放不下；和调色工作区、资源管理器一样，先在文档里写明只有电脑端。

### 7.2 页面

```text
+----------------------------------------------------------------------+
| 我的占用 15.2 GB                                                     |
| 原片 14.3 GB · 缩略图 468 MB · 转码 381 MB · AI 28 MB                |
| 可回收 849 MB · 统计于 2 小时前                    [重新统计]        |
+----------------------------------------------------------------------+
| [按格式] [按相册] [按年月]                                           |
|                                                                      |
|   ◯ 环形图            jpg   10.1 GB  ████████████████░░░░  1205 个   |
|   格式占比            zip    1.3 GB  ██░░░░░░░░░░░░░░░░░░     1 个   |
|   图例：大小 / 百分比  ...                                            |
+----------------------------------------------------------------------+
| 缩略图  1451 个 / 454 MB                                             |
|   360px 792 个 · 240px 617 个 · 其他 42 个                           |
|   未归属 11 个 / 3.1 MB                  [清理未归属] [清理全部…]     |
+----------------------------------------------------------------------+
| 转码视频  2 个 / 381 MB                                              |
|   PANA8405.MOV  测试  H.264  126 MB  [删除]                          |
|   PANA6850.MP4  测试  H.264  255 MB  [删除]                          |
|   未归属 0 个                                        [清理未归属]     |
+----------------------------------------------------------------------+
```

- 三个视图各有一张环形图（`StorageDonut.vue`，纯 SVG，没有图表库）：总览的「磁盘构成」、格式页的格式占比、相册页的相册占比。悬停一块，中心显示它的名称和大小，其余块变淡；图例给大小和百分比，图例在下方通栏，避免窄卡片里被截断。环形图是辅助，数字表仍然是主要信息。
- 「按相册」是表格：相册、文件数、原片、缩略图、转码、合计，右边跟一张相册占比环形图；缩略图、转码两列各带一个清理按钮。
- 「按年月」按年折叠月份，条形表示原片字节数；缩略图和转码按源文件月份叠加成第二段颜色。没索引的相册在表尾单列，带「建立索引」按钮。
- 「按格式」按扩展名排序，列表给全部；环形图取前五加「其他」。
- 相册外目录单列一块，说明这些文件不在相册里、不参与年月统计。
- 「清理全部」列出来的时候要写清楚会删掉多少、下次查看会重新生成；未归属默认不选中。
- 手机不给入口，但同一个接口以后能直接接。

### 7.3 四态

| 情况 | 界面 |
| --- | --- |
| 加载 | 骨架条，保留上一次的数字 |
| 空 | 图库还是空的，写清楚没有可统计的内容 |
| 失败 | 顶上写原因，保留下次可用的旧数字和「重试」 |
| 无权限 | 会话过期就回登录；共享相册不在这个页面出现 |

### 7.4 状态放哪

和 Dashboard 一样挂在 `useAlbumApp`：`storageReport`、`storageLoading`、`storageError`、`storageScanning`、`refreshStorageReport()`、`startStorageScan()`、`startStorageCleanup(kind, target, params)`。复用现成的 `apiFetch`，不重复拼认证和语言头。任务进度沿用 `TaskSidebar` 的轮询，不新造一套。

## 8. 国际化

后端 `locales/{zh,en}.yml`：

```text
task.storage_scan / task.storage_cleanup / task.storage_scanning / task.storage_removing
response.storage_scan_started / response.storage_cleanup_started
error.storage_conflict / error.storage_invalid_target / error.storage_invalid_key
error.storage_album_not_found / error.storage_size_invalid
```

前端 `frontend/src/i18n/{zh,en}.js` 新增 `nav.storage` 和 `storage.*`：总览、三个视图、相册外、资源库、临时目录、未归属、转码列表、确认框、四态、估算与「下次查看会重新生成」的说明。数字用现有的 `formatSize`，日期用现有本地化方式。

## 9. 测试

Rust 侧（`src/backend/storage.rs` 里 `#[cfg(test)]`）：

- 造一个临时数据根和缓存文件，用真实 key 函数写一份缩略图，扫描能归属；改源文件 mtime 后同一文件变成未归属。
- 扩展名归一：`JPEG`→`jpg`、无扩展名→`(none)`。
- 年月桶：`""`、`"1970-01-01"`、`"2026-4"` 都进「未纳入索引」。
- `__uploading` 不计、符号链接不跟、`__resource__` 单列。
- 清理参数：`owner="../exif"`、`album="../x"`、`key="a/b"`、`codec="mp4"`、`size=99999` 全部 `BadRequest`。
- `dry_run` 一个文件都不删。
- 报告 `version` 不匹配判过期。
- 未认证、另一个账号读不到我的报告（接口层）。

实现时用真实缓存目录结构（`cache/thumbnails/{owner}/`、`cache/video-encode/{owner}/{codec}/`）建临时目录，不要只在内存里模拟路径。

手动验收：

1. 空账号打开存储：空态、能点重新统计。
2. 有数据的账号：三个视图的数字和 `du -sh` 对得上（误差只来自 `cache/`）。
3. 删一个转码，播放同一视频能重新转出来，原片 mtime 和 XMP 没变。
4. 清理未归属缩略图，回到相册网格，图还在，滚动时按需生成。
5. 改一个文件名（让旧缩略图失配），重新统计，未归属多一个。
6. 只读共享账号：看到的是自己的数字，删不动别人的缓存，后台直接拒绝。

前端 `cd frontend && npm run build`；Rust `cargo +nightly test`。

## 10. 实施顺序

1. `src/backend/storage.rs`：类型、`scan_user_storage()`、归属与聚合、单测。挂进 `src/backend/mod.rs`。这一步不碰 HTTP。
2. 把 `darktable_preview_cache_path` 里的哈希抽成 `darktable_preview_cache_key`，`source_cache_key` 提成 `pub(crate)`，两边共用。
3. `StorageScanTask` + 报告落盘 + `GET /api/storage/report`。
4. `StorageCleanupTask` + `POST /api/storage/scan` / `cleanup`，路由进 `src/main.rs`，越权和冲突测试。
5. 前端 `StoragePanel.vue`、侧栏 tab、`useAlbumApp` 状态、中英文案。
6. 收尾：`spawn_cache_gc` 覆盖 `video-encode`；Dashboard 存储卡片跳转；README「功能」补一行；按实测数字回填本文件。

## 11. 不做 / 待定

- **AI 缓存删除**：重建要跑模型，可能几十分钟。这一版只读展示，界面写明重建代价。
- **管理员看全站**：和 Dashboard 的规则一致，管理员也只统计自己的数据根和缓存。
- **历史趋势**：只存当前快照，不存时间序列。
- **写入时登记**：生成缓存时顺手记一份 `key → 源文件` 清单，比按尺寸表重算精确，但要在缩略图热路径上多一次写。等尺寸表真的开始漏文件（未归属持续不为零）再补，接口形状不用变。
- **自动清理策略**：沿用现有的「缓存保留天数」，这次只把覆盖范围扩到转码，不新增规则。
- **手机**：接口以后能接，界面这一版不做。
- **回收站**：删的是缓存，不涉及原片，不需要。

## 12. 实测（本机 daniel 数据根）

| 项目 | 实测 |
| --- | --- |
| 数据根 | 2053 个文件 / 14.33 GB |
| 相册内 | 1397 个 / 8.07 GB |
| 资源库 | 40 个 / 0.02 GB |
| 相册外 | 584 个 / 6.00 GB（`trans` 1.98 GB、`测试` 1.70 GB、`lirui.zip` 1.26 GB） |
| 临时目录 | 32 个 / 0.25 GB |
| 格式前六 | jpg 10.09 GB、zip 1.26 GB、rw2 0.86 GB、xcf 0.79 GB、mp4 0.51 GB、mov 0.44 GB |
| 缩略图 | 首次扫描 1451 个 / 454 MB，命中 1440（360px 792、240px 617、0px 9、其余 22），未归属 11 个 / 3.1 MB；后台缩略图任务随后自己删掉了那 11 个，再扫是 1440 个 / 0 未归属 |
| darktable 预览 | 14 个 / 14 MB；命中 14 |
| 转码 | 2 个 / 381 MB；命中 2（440.9 MB MOV→126.2 MB，500 MB MP4→254.7 MB）；探测 3 个，全部可归属，其中一个视频只探测过、没转码 |
| AI 缓存 | 2801 个 / 28 MB（只读） |

归属验证的做法和实现一致：按 3.2–3.4 的公式从数据根重算 key，和目录里的文件名求交集。第一版尺寸表漏了 `240` 和 `0` 时，未归属冒到 634 个 / 357 MB，补全后回到 11 个；这就是第 3.2 节那条维护规则的由来。
