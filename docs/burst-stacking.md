# 连拍折叠设计

状态：已实现 v1（文件级分组；阈值和页面细节仍可继续调整）。

首版采用**文件级分组**：每个相册文件都是独立候选，相似度只看时间、相机、方向和 pHash。角标是 stack 内部图像文件总数。

## 1. 目标

相册默认按拍摄时间浏览时，把相邻、相似度高的几张文件合并成一个网格项：

- 网格只显示一张封面，角标显示这一组有几个文件；
- 点开后能在同一组内左右切换，不丢文件；
- 删除、下载、复制、移动可以按整组操作，但真正执行的仍是逐个文件；
- 原片和 XMP 不被改写，折叠只是展示层；
- 扫描结果放在缓存里，删掉可以重建。

一句话场景：**用户打开相册的时间线，同一连拍的 5 个文件折叠成一张带 `5` 角标的封面；点开后看到 5 个文件，左右切换；删除时确认“删除这 5 个文件”，原片和 XMP 不被改写。**

失败时：

- 缓存还没算好：先显示平铺列表，并提示“正在分析连拍”；
- 分析失败：保持平铺，提供重试，不隐藏任何文件；
- 没有权限：不返回其他账号的相册或路径，整组操作由后端逐个文件拒绝。

## 2. 非目标

首版不做：

- 跨相册合并；
- 视频连拍；
- 手动合并两组、手动把一组拆开；
- 把连拍组写进 XMP 或 `.lumeno.json`；
- 用物体识别或语义模型判断“这是不是连拍”；
- 在 WebDAV 里暴露虚拟文件夹；
- 自动删除连拍里的“差片”。

### 文件计数

`stack.count` 始终是 stack 内部图像文件总数，不换算成其他单位。

连拍折叠和现有的“重复图像”也是两件事：

- 重复图像看的是**内容相同**，可以跨时间、跨目录；
- 连拍看的是**时间相邻 + 画面相似**，只在同一个相册的相邻时间窗口里成立。

## 3. 总体方案

```text
AlbumIndex.entries（每个文件一条）
    + phash cache（没有就后台补算）
        ↓
按拍摄时间排序、相邻候选过滤
        ↓
连拍分组（时间 + 相机 + 方向 + pHash）
        ↓
burst cache（每个相册一份 JSON）
        ↓
相册列表接口返回“封面 + stack 摘要”
        ↓
VirtualGrid 显示折叠卡片
        ↓
点开 → 取组内文件 → 现有照片查看器 / 调色工作区
```

关键决定：

1. **后端分组**。前端只拿当前页，不能靠“当前页的相邻项”判断，否则分页边界会把一组拆开。
2. **复用现有 DCT 查重向量**。连拍不新增一套相似度哈希：直接读 `src/backend/ai/duplicate.rs` 已有 DCT pHash 对应的 `AlbumAiCache` `phash/{key}.json`。缺失时用同一个 `compute_photo_phash_preferring_preview` 算一次并写回同一份缓存，重复图像视图和连拍共享。连拍不依赖模型或 AI provider。
3. **只看四个信号**。分组逻辑只使用时间、相机、方向、pHash；解码器按文件内容选择解码方式，但不参与分组。
4. **折叠开启时，排序和过滤都照常工作**。排序时每个 stack 以封面照片的参数参与；过滤时逐个文件判断，最后 stack 的角标是过滤后剩余文件数。
5. **列表接口向后兼容**。不传 `collapse=bursts` 时行为完全不变；传了才返回 `stack` 字段和新计数。
6. **缓存可删**。连拍索引是派生数据，不进原片目录，不写 XMP。

## 4. 路径约定

本设计区分两种路径，不能混用：

- **相册相对路径**：`2026/2026-04-01/IMG_0042.jpg`，相对这本相册的根目录。  
  连拍缓存、`POST /api/albums/stack` 返回的 `photos` 都用这个。
- **用户数据根相对路径**：`albums/trip/2026/2026-04-01/IMG_0042.jpg`。  
  这是现有 `/api/albums/photos` 的 `photos[].path` 契约，前端调用缩略图、删除、下载、XMP 等现有接口时继续用它。

新接口不要再把 `albums/{album}/` 存进相册内的缓存或 stack 响应；`album` 已经在请求上下文里。前端从 stack API 拿到相册相对路径后，用 `albums/{album}/` 拼回现有接口需要的路径。

## 5. 数据模型

### 5.1 网格项

列表接口在折叠模式下，`photos` 数组里每个元素仍是一条 `AlbumPhotoItem`，但封面多一个可选字段：

```json
{
  "path": "albums/trip/2026/2026-04-01/IMG_0042.jpg",
  "name": "IMG_0042.jpg",
  "datetime": "2026-04-01 18:31:02",
  "width": 6000,
  "height": 4000,
  "rating": 5,
  "owner": "daniel",
  "album": "trip",
  "stack": {
    "id": "c2E6...",
    "count": 5,
    "start_datetime": "2026-04-01 18:31:00",
    "end_datetime": "2026-04-01 18:31:04",
    "max_hamming_distance": 6
  }
}
```

- 单张文件没有 `stack`。
- 这里的 `photos[].path` 仍是现有列表契约的用户数据根相对路径；`stack` 内部不存这个前缀。
- `stack.count` 是**文件数**，包含封面自己。
- `stack.id` 只用于取组内成员，不是权限凭证。
- 组内成员的完整信息不放在列表里，避免每页重复携带 5 份 EXIF。

### 5.2 组内成员接口

```http
POST /api/albums/stack
Content-Type: application/json

{
  "album": "trip",
  "stack_id": "c2E6..."
}
```

响应：

```json
{
  "success": true,
  "message": "ok",
  "code": 0,
  "data": {
    "owner": "daniel",
    "album": "trip",
    "stack": {
      "id": "c2E6...",
      "count": 5,
      "cover": "2026/2026-04-01/IMG_0042.jpg"
    },
    "photos": [
      {
        "path": "2026/2026-04-01/IMG_0041.jpg",
        "name": "IMG_0041.jpg",
        "datetime": "2026-04-01 18:31:00"
      },
      {
        "path": "2026/2026-04-01/IMG_0042.jpg",
        "name": "IMG_0042.jpg",
        "datetime": "2026-04-01 18:31:01"
      },
      {
        "path": "2026/2026-04-01/IMG_0043.jpg",
        "name": "IMG_0043.jpg",
        "datetime": "2026-04-01 18:31:02"
      },
      {
        "path": "2026/2026-04-01/IMG_0044.jpg",
        "name": "IMG_0044.jpg",
        "datetime": "2026-04-01 18:31:03"
      },
      {
        "path": "2026/2026-04-01/IMG_0045.jpg",
        "name": "IMG_0045.jpg",
        "datetime": "2026-04-01 18:31:04"
      }
    ]
  }
}
```

- 这里的 `photos[].path` 是**相册相对路径**，和 `POST /api/albums/download` 的 `photos` 一致。
- `photos` 里每个元素就是一个文件；不做配对，也不做合并。
- `stack.count` 等于 `photos.len()`。
- 成员必须全部落在该相册目录下；服务端用已解析的相册根目录 `join` 后再经 `into_host_path_safely` 收口。
- 只返回调用者能读的文件；`can_delete` 只影响前端按钮，删除仍由现有接口逐文件鉴权。
- 缓存过期或组已不存在时返回 `404 stack_not_found`，前端重新拉列表。

### 5.3 缓存文件

新增缓存区：

```text
dist/data/cache/bursts/{owner}/{album_cache_id}.json
```

`album_cache_id` 使用 `AlbumMeta.cache`，和现有相册索引共用稳定 key，相册改名不丢缓存。

```json
{
  "schema_version": 1,
  "algorithm": "burst-v1",
  "source_index": {
    "entries": 33725,
    "modified_unix_ms": 1780000000000,
    "len": 12345678
  },
  "created_at_unix_ms": 1780000000000,
  "max_hamming": 10,
  "stacks": [
    {
      "id": "c2E6...",
      "cover": "2026/2026-04-01/IMG_0042.jpg",
      "count": 5,
      "members": [
        "2026/2026-04-01/IMG_0041.jpg",
        "2026/2026-04-01/IMG_0042.jpg",
        "2026/2026-04-01/IMG_0043.jpg",
        "2026/2026-04-01/IMG_0044.jpg",
        "2026/2026-04-01/IMG_0045.jpg"
      ],
      "start_datetime": "2026-04-01 18:31:00",
      "end_datetime": "2026-04-01 18:31:04",
      "max_hamming_distance": 6
    }
  ]
}
```

- `source_index` 用来判断缓存是否过期；索引重建后修改时间或条目数变化就重算。
- `members` 是文件的**相册相对路径**，不存绝对路径，也不带 `albums/{album}/`。
- `count` 等于 `members.len()`。
- `id` 是 `sha256("burst-v1|" + 排序后的成员路径)` 的 base64url；成员变化时 id 变化，封面变化时 id 不变。
- 缓存区注册到 `src/backend/cache_areas.rs`：`PerOwner`，首版 `Never`（和 `index` 一样，不参与按时间自动 GC；可以用显式重建清掉）。

## 6. 相似度算法

### 6.1 候选集合

只处理：

- 同一个相册；
- `duration_ms == 0` 的图片（视频不折叠）；
- `datetime` 可解析的文件；
- 整个相册的索引；排序和过滤在折叠之后另行应用。

按 `datetime` 升序、`name` 升序排列；展示时再按请求的日期方向排。

每个索引条目就是一个候选文件，没有额外的分组层。

### 6.2 常量

首版固定参数，放在 `src/backend/constants.rs`，不暴露到普通设置：

| 常量 | 初值 | 说明 |
| --- | --- | --- |
| `BURST_ALGORITHM` | `"burst-v1"` | 算法版本，进缓存 key 和 stack id |
| `BURST_MAX_GAP_MS` | `2000` | 相邻文件时间窗口的默认值；由相册级时间窗口覆盖 |
| `BURST_MAX_SPAN_MS` | `15000` | 一组从第一个文件到最后一个文件的最大跨度 |
| `BURST_MAX_HAMMING` | `10` | DCT 算法的默认汉明距离；由相册级阈值覆盖 |
| `BURST_EMBEDDING_THRESHOLD` | `80` | Embedding 算法的默认余弦相似度百分比 |
| `BURST_MAX_TIME_WINDOW_SECONDS` | `86400` | 时间窗口上限，24 小时 |
| `BURST_MIN_MEMBERS` | `2` | 少于 2 个文件不成组 |
| `BURST_MAX_MEMBERS` | `20` | 一组最多 20 个文件，达到上限后封组 |

这些是初值，不是永久协议；调到 `burst-v2` 时改算法名，旧缓存自然失效。

### 6.3 相邻判断

对有序文件 `i`，从 `i` 开始尝试向后收：

```text
group = [i]
for j in i+1..:
    if j - i >= BURST_MAX_MEMBERS: break
    if gap(group.last, j) > BURST_MAX_GAP_MS: break
    if j.time - group.first.time > BURST_MAX_SPAN_MS: break
    if camera_mismatch(group.first, j): break
    if orientation_mismatch(group.first, j): break
    if any hamming(phash[k], phash[j]) > BURST_MAX_HAMMING for k in group: break
    group.push(j)

if group.len >= 2: emit group
```

关键点：

- **和组内每一个文件都比较**，不是只比上一张。这样不会因为“1 像 2、2 像 3、3 像 4”把一段长链滚成一组。
- 第一个不满足就停止向后，不跳过中间文件继续找；连拍在时间线上是连续的。
- 相机字段两边都有值时要求相同；有一边为空时允许，但时间和 pHash 仍然要过。
- 方向用 `width >= height` 的横/竖分类；两边都有尺寸时要求一致。
- 时间差用解析后的时间；解析失败的文件直接单张，不参与折叠。
- 分组逻辑只使用时间、相机、方向、pHash。

### 6.4 复用现有 DCT 查重向量

连拍不新增 pHash 算法，也不新增 `burst-phash` 缓存。唯一的相似度来源是现有“重复图像”的 DCT pHash：

```text
dist/data/cache/ai/{owner}/albums/{cache_id}/phash/{key}.json
{
  "version": 1,
  "stable_key": "owner|album|albums/album/rel.jpg",
  "source": "rel.jpg",              // 相册相对路径
  "phash_u64": 123456789012345678,
  "content_signature": "size:mtime"
}
```

复用方式：

1. 每个 `AlbumPhotoItem` 用 AI 扫描同一套身份函数计算 `stable_key` 和 `content_signature`；
2. 用同一个 `phash_cache_key(stable_key, content_signature)` 读 `AlbumAiCache::read_phash_async`；
3. 命中且 `content_signature` 一致、`source` 等于该文件的相册相对路径：直接使用 `PhashCacheFile.phash_u64`；
4. 缺失、版本不符或 `source` 不匹配：用 `compute_photo_phash_preferring_preview` 从预览/缩略图算一次，再用 `write_phash_async` 写回**同一个 key**；
5. 不另建 path → hash 的索引；逐文件按 key 读即可。若 AI 索引记录已经在内存里，可以用它的 `duplicate_phash_u64` 作为快路径，但最终仍以同一份 `phash/{key}.json` 为准。

约束：

- `PhashCacheFile.source` 已经是相册相对路径（`AiIndexedPhotoRecord::album_relative_path()`），正好和本设计的 stack 路径约定一致；
- 不复制、不重命名、不另建 `bursts/phash/`；重复图像和连拍必须读写同一份缓存；
- `duplicates_index.json` 只用于“重复图像”的 clique 结果，**不能**直接拿来当连拍分组：它是阈值 8 的团、没有时间顺序，也没有保留逐文件 pHash。连拍只复用它的底层 pHash 值；
- 两个入口必须共用 `build_photo_stable_identity`、`build_photo_content_signature`、`phash_cache_key` 和 `PHASH_CACHE_VERSION`，避免两套公式漂移；
- 现有 `build_photo_stable_identity` / `build_photo_content_signature` 在 `ai/pipelines/scan.rs` 里是私有的，需要抽到共享模块（例如 `ai/phash.rs` 或 `ai/identity.rs`），AI 扫描和 burst 任务都调用它；
- 缓存里的 `source` 是相册相对路径；如果旧缓存缺少 `content_signature` 或 `source`，按 miss 处理并重算；
- 连拍扫描不要求 AI provider 可用，只需要这份本地 pHash 缓存可写；
- 不引入新的模型，也不要求 SigLIP/InsightFace 权重；图像解码仍用现有预览/解码路径，pHash 值和重复图像完全同源。

这样删除 `cache/ai/.../phash/` 后，重复图像和连拍会一起重建，不会出现两套“相似度真相”。

### 6.5 算法选择

相册级 `burst_algorithm` 可以选择：

- `dct`：复用 DCT pHash，汉明距离 ≤ `burst_threshold`（0–32）；
- `embedding`：读取 AI 索引里的图像 embedding，余弦相似度 ≥ `burst_embedding_threshold / 100`（30–100）。

两种算法共用相同的时间窗口、相机、方向和最大组大小约束。时间窗口是相册级设置，范围 1 秒到 24 小时，表示相邻文件的最大时间差；整组跨度至少保留原来的 15 秒，窗口更大时按窗口放宽。

Embedding 算法需要已经存在的 AI 图像索引；没有 embedding 时不会生成组，不会自动回退到 DCT。

缓存文件记录 `algorithm` 和 `threshold`，切换算法或阈值都会使旧缓存失效。

### 6.6 封面选择

组内选一个文件作为封面，优先级：

1. 评分高；
2. 像素面积大；
3. 位于组中间（避免第一个刚好是糊的）；
4. 拍摄时间早。

封面只是展示；组内所有文件都在成员接口里，不做“只保留封面”的删除。封面可能是组内任意一个文件。

### 6.7 组的排序

折叠时，stack 作为一个整体参与排序：

- 用 stack 的封面照片作为排序键；
- 日期、名称、大小、评分、相机、镜头等排序字段都取封面的值；
- 多级排序逐级比较封面的值；
- 默认无排序时按封面的日期降序、名称升序；
- 关闭折叠时，每个文件按自己的字段排序。

过滤在排序之后进行：每个文件单独判断是否匹配，stack 只统计匹配的文件数。封面的排序位置不因过滤而改变。

## 7. API

### 7.1 列表接口

在现有两个入口加可选参数：

```http
GET  /api/albums/photos?album=trip&collapse=bursts&offset=0&limit=100
POST /api/albums/photos
{
  "album": "trip",
  "filters": [],
  "sort": [{"field":"date","dir":"desc"}],
  "collapse": "bursts",
  "offset": 0,
  "limit": 100
}
```

不传 `collapse`：响应和现在完全一致。

传 `collapse=bursts` 且允许折叠时：

- `photos` 里每个元素是一个显示项（单张或连拍封面）；
- `total_count` 是**显示项数量**；
- `photo_count` 是**文件数量**；
- `offset` / `limit` 按显示项计数；
- 分页不会拆开一组；
- `bursts` 描述扫描状态。

```json
{
  "photos": [ "…封面或单张…" ],
  "total_count": 120,
  "photo_count": 437,
  "offset": 0,
  "limit": 100,
  "bursts": {
    "state": "ready",
    "algorithm": "burst-v1",
    "stack_count": 42,
    "task_key": null
  },
  "available_tags": [],
  "available_cameras": [],
  "available_lenses": [],
  "available_formats": []
}
```

`bursts.state`：

| 状态 | 含义 | 前端行为 |
| --- | --- | --- |
| `ready` | 缓存有效 | 正常显示折叠 |
| `pending` | 正在扫描 | 先显示平铺，提示“正在分析连拍” |
| `stale` | 缓存过期，已排队刷新 | 先显示平铺，刷新后重取 |
| `error` | 缓存或索引不可读 | 平铺；任务列表里可以重试 |


### 7.2 现有接口不变

- 删除、下载、复制、移动继续接收路径数组；
- 前端选中一个连拍组时，把组内所有文件路径放进现有请求；
- 后端逐个文件走原有权限和路径检查；
- 不新增“批量删除整组”的绕过入口；
- stack API 给的是相册相对路径；前端调用现有接口前用 `albums/{album}/` 拼回用户数据根相对路径。

### 7.3 错误码

| HTTP | 场景 |
| --- | --- |
| `404` | 组已不存在、缓存已重建或 id 无效 |
| `403` | 没有相册读权限，或现有逐文件操作被拒绝 |
| `400` | 请求体缺少 `album` / `stack_id`，或相册名不合法 |

后台扫描失败会出现在现有任务列表里；列表接口会继续返回平铺，前端会短暂轮询，最多重试约 20 次后停止，不会把失败伪装成空相册。

## 8. 缓存与任务

### 8.1 触发

主路径：

- 重建相册索引、云导入、本地导入的 TaskGroup 最后一步都会调用 `bursts::ensure_for_album`；
- AI 索引流水线的最后一步也会为受影响的相册生成 burst cache；
- 同一个 `ensure_for_album` 在缓存已有效时直接返回，不会重复计算。

兜底路径：相册列表请求带 `collapse=bursts`，但缓存仍未生成时：

1. 解析并鉴权相册；
2. 读取 `cache/bursts/...json`；
3. 用 `source_index` 对比当前 `AlbumIndex`；
4. 有效：直接分组返回；
5. 缺失/过期：返回平铺，`bursts.state=pending/stale`，用 `TaskManager` 排队 `burst-index:{owner}:{cache_id}`；
6. 已有同 key 的活动任务：不重复排队。

### 8.2 任务

`BurstIndexTask` 只用于列表请求的兜底；正常情况下由 TaskGroup / AI pipeline 完成：

- 读取相册索引，按拍摄时间排序；
- 对每个文件读同一份 `AlbumAiCache` pHash 缓存，缺失则用现有 `compute_photo_phash_preferring_preview` 计算并写回；
- 按第 6 节分组；
- 原子写入 `cache/bursts/{owner}/{cache_id}.json`；
- 进度用现有 `TaskBase` 的已处理/总数；
- 并发复用现有 `ConcurrencyLimiter`（优先缩略图 limiter，避免扫描把所有 CPU 吃掉）；
- 失败写任务错误，不删除旧缓存。

任务完成后：

- 前端现有的任务轮询发现 `burst-index` 完成/失败；
- 当前页重新请求一次；
- 用户不需要手动刷新。

### 8.3 失效

缓存 `source_index` 与当前索引不一致时失效：

- 索引条目数变化；
- 索引文件修改时间或长度变化；
- 算法版本变化（文件名/字段里带 `burst-v1`）。

缓存被删、被 GC 或相册重建索引后，下一次请求重新扫描；原片和 XMP 不受影响。

## 9. 前端

### 9.1 开关

- 放在缩略图大小弹层（网格的“视图设置”入口）里：**折叠连拍**开关。
- 折叠算法和阈值都是**相册级配置**，在“更多设置”里修改。
- 时间窗口：1 秒到 24 小时，默认 2 秒。
- DCT：阈值 0–32，默认 10，数值越小越严格。
- Embedding：余弦阈值 30–100%，默认 80%，数值越高越严格。
- 配置写入 `album_meta.json`，同一相册的所有查看者共用。
- 折叠开关本身仍记在本地视图偏好里。
- 默认开启；用户关闭后，请求不带 `collapse=bursts`。

### 9.2 网格卡片

`PhotoCard` 增加 `stack`：

- 封面后面画 2 层偏移边框，形成“叠起来”的视觉；
- 右下角显示 `count`（文件数）；
- `aria-label` 包含“连拍，5 个文件”；
- 单张文件仍走原样。

`VirtualGrid`：

- 显示项的 key 用 `stack.id`（有 stack 时）或 `path`；
- 列表仍是虚拟滚动；
- 选中一个 stack 时，选中它的所有成员路径，复选框显示半选/全选状态；
- 过滤在排序之后逐个文件执行；stack 只显示匹配的文件数。
- 封面如果被过滤掉，显示第一个剩余成员，但排序位置仍按原封面。

### 9.3 点开

- 普通文件：现有 `PhotoDetail` 行为不变。
- 连拍封面：
  1. 调 `POST /api/albums/stack` 取成员；
  2. 用 `albums/{album}/` 把成员转成现有查看器/缩略图接口需要的路径；
  3. 打开现有照片查看器，标题加“连拍 · 5 个文件”；
  4. 底部胶片条只显示这一组，左右键在组内切换；
  5. 关闭后回到网格。
- 桌面调色工作区：打开当前成员，`photos` 列表传组内成员；不新增“整组调色”概念。

### 9.4 操作

- **删除**：确认框显示“删除这组 5 个文件？”，执行时提交全部成员路径；后端逐个校验。
- **下载**：复用相册下载接口，组内成员全部加入。
- **复制/移动**：复用现有路径选择。
- **分享**：仍按相册分享，不按组。
- **WebDAV**：文件仍然逐个出现，不创建虚拟目录。

### 9.5 状态

- `pending`：平铺网格 + 轻量提示“正在分析连拍”，不盖住照片；
- `stale`：平铺 + 提示“正在刷新连拍”；
- `error`：平铺 + “连拍分析失败，重试”；
- `ready`：正常折叠；
- 无权限/组失效：成员接口返回 404/403，前端回列表并提示，不展示空组。

### 9.6 移动端

手机壳复用同一个 `AlbumPanel`（`embedded` 模式）和照片查看器：

- 网格卡片同样显示角标；
- 点开进入现有全屏 viewer；
- 组内左右滑动只切这一组；
- 删除/下载沿用现有 ActionSheet，并显示文件数量。

## 10. 权限与安全

- 列表和成员接口都先 `resolve_album_access`；没有相册读权限就不返回 stack。
- `stack_id` 只是缓存键，不能当能力凭证；必须重新校验相册和成员路径。
- 成员路径是相册相对路径；服务端用已解析的相册根目录 `join` 后再经 `into_host_path_safely`，不能跨出该相册目录。
- 只读分享：能看 stack，不能删/移动；后端现有逐文件权限仍然生效。
- 不跨 owner、不跨相册；同一张照片在另一个相册不会自动合并。
- 缓存只写 `cache/bursts/`，不写原片、不写 XMP、不写 `.lumeno.json`。
- 日志和响应不输出绝对路径、HMAC key、会话；stack id 是哈希，不携带路径。
- WebDAV、缩略图、下载接口不感知 stack，不改变原有授权。

## 11. 测试

### 11.1 算法单元测试

- 3 个时间相邻、pHash 距离小的文件 → 一组；
- 时间差超过 2s → 不合并；
- 中间一个不同 → 断组，不把后面硬塞进来；
- 相机型号不同 → 不合并；
- 横竖方向不同 → 不合并；
- 链式相似（1↔2、2↔3、1↔3 超阈值）→ 不成一组；
- DCT 阈值实际参与分组比较；
- Embedding 余弦阈值能区分相近和相反的向量；
- 时间窗口可以跨过默认的 2 秒；
- 超过 `BURST_MAX_MEMBERS` → 按上限封组，后面的文件重新开始；
- 封面评分/分辨率/中间帧优先级；
- 空 `datetime`、视频 → 单张；
- 5 个文件一组 → `count=5`，成员数和 `members.len()` 一致；
- 相同成员集合的 stack id 稳定；
- 分组函数只使用时间、相机、方向、pHash。
- 已有 `PhashCacheFile` 命中时不调用 `compute_photo_phash_preferring_preview`；
- 缺失时写回同一个 `phash_cache_key`，重复图像索引能看到新增的 pHash；
- `PhashCacheFile.source` 是相册相对路径，和 stack 成员路径一致；
- 不创建 `bursts/phash/` 或其他第二套 pHash 缓存。

### 11.2 API 测试

- 不传 `collapse`：响应、`total_count`、分页和原来一致；
- `collapse=bursts`：`total_count` 是显示项，`photo_count` 是文件数；
- 分页边界不拆组；
- 筛选/非日期排序仍返回折叠结果；
- 过滤后 stack 的 `count` 等于剩余匹配文件数；
- 成员接口：owner、被分享者、只读分享、无权限、过期 stack id；
- 成员路径越界、路径穿越、跨相册 id 被拒绝；
- stack 响应返回相册相对路径；前端拼回 `albums/{album}/` 后现有接口仍能工作；
- 缓存缺失时返回 `pending`，任务完成后 `ready`；
- 索引重建后旧缓存失效。

### 11.3 前端验证

- 网格出现角标和叠层；
- 点开只显示组内成员，左右切换不越组；
- 关闭折叠后恢复平铺；
- 筛选时仍显示 stack，角标是剩余匹配文件数；
- 选中组后删除确认数量正确，取消不删；
- 手机端滑动、删除、下载；
- pending/error/空状态有可见结果；
- 角标始终等于 stack 内文件数。

### 11.4 性能

- 30k 文件相册，缓存命中时列表请求不读图片，不重算 pHash；
- 首次扫描走缩略图，不逐张解码原片；
- 扫描并发有上限，不阻塞缩略图请求；
- 单个相册缓存 JSON 保持在几 MB 量级。

## 12. 发布顺序

1. 后端文件级分组算法 + 单元测试，默认不启用；
2. burst cache + 任务 + 列表 `collapse=bursts`；
3. 桌面网格、查看器、删除/下载、视图开关；
4. 手机壳复用；
5. 用真实相册校准 `BURST_MAX_HAMMING`/时间窗口，再决定是否默认开启。

首版默认开启、可通过视图设置关闭；如果校准期误合并明显，先默认关闭，由用户主动打开。

## 13. 现有路径约定审计

以“相册相对路径”为基准，现有设计/实现大致分三类。

| 位置 | 当前路径 | 判断 |
| --- | --- | --- |
| `/api/albums/photos` 的 `photos[].path` | `albums/{album}/{rel}`（用户数据根相对） | 既有 API 契约，前端缩略图、删除、下载、XMP 都依赖它；本功能不静默改 |
| `POST /api/albums/download` 的 `photos` | `{rel}`（相册相对） | 现有正确先例；stack API 跟随这个约定 |
| `AlbumPhotoItem.path` / `PhotoRef::api_path()` | `albums/{album}/{rel}` | 与列表 API 同一套；内部结构也用这个 |
| AI 索引的 `AiIndexedPhotoRecord.path` / 设计文档示例 | `albums/{album}/{rel}`，同时又带 `album` 字段 | 冗余；稳定键 `(owner, album, path)` 里 album 出现两次。要改成相册相对，必须迁移稳定键和缓存 |
| `PhashCacheFile.source` | `{rel}`（相册相对） | 正确；连拍直接复用，不要另存一份 |
| 相册封面 `AlbumMeta.cover`、人脸 cover 的 `path` | `albums/{album}/{rel}` | 和列表/封面接口一致；暂不改 |
| `storage-manager`、`resource-manager` 的路径 | 数据根相对（如 `albums/__resource__/...`、`cache/...`） | 正确；它们描述跨相册的文件系统位置，不是相册照片 API |
| `dashboard-plan` 的预览 `path` | 跟随现有列表 API | 保持用户数据根相对；如果以后统一，和列表一起迁移 |
| 本设计的 stack cache / stack API | 相册相对 | 新接口统一用这个，不再带 `albums/{album}/` |

结论：

- 本连拍功能只把**新接口**统一成相册相对路径；
- 不改现有 `/api/albums/photos` 和 `AlbumPhotoItem.path`，否则前端和缓存会大面积断；
- AI 索引的 `(album, path)` 是历史设计里最明显的“路径已经包含 album、又单独带 album”的冗余点，应该单独做迁移，不塞进连拍；
- 如果以后要全局统一，建议新增 `rel_path` 字段并保留 `path` 兼容，缓存 key 按版本迁移，不要直接改字段语义。

## 14. 待定

- `BURST_MAX_HAMMING` 初值是否用 10，还是更保守的 8；
- 过滤后封面被隐藏时是否允许改用剩余成员作为排序键；
- 是否需要“这组不是连拍”的负反馈；
- 是否把 AI embedding 作为第二信号（首版不需要，避免依赖模型）；
- 是否把完全重复的照片交给“重复图像”而不是连拍（首版会合并，可能需要在 UI 上区分）；
- 是否把现有 `AlbumPhotoItem.path` 的用户数据根相对路径迁移成相册相对路径（会破坏现有 API/缓存，未列入首版）。
