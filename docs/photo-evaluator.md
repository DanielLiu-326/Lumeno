# 照片质量排序 Evaluator

状态：后端核心和重复组最优标记已实现；连拍组推荐、批量清理和对比界面尚未接入。

适用场景：

- 重复图像组中推荐保留哪一张；
- 连拍组中推荐保留哪一张；
- 多选照片中按自动质量排序。

Evaluator 只负责**读取信号、计算质量、稳定排序和给出推荐理由**。它不删除、不移动、不修改原片或 XMP。

## 1. 不做的事情

第一版不提供以下策略切换：

- 最早拍摄；
- 最新拍摄；
- 手动指定；
- “保留文件最大的一张”单独模式。

调用方始终使用自动质量排序。手动选择仍可由界面完成，但不进入 Evaluator 的参数模型。

也不在第一版引入：

- BRISQUE / NIQE 等无参考质量模型；
- CLIP / SigLIP 审美评分；
- LLM 判断哪张好看。

这些模型不可解释、依赖额外推理，且对“重复文件清理”没有明显收益。

## 2. 分层结构

```text
上层 API / UI
  → Evaluator
      → 读取元数据与 XMP 信号
      → 读取或计算质量指标
      → 纯函数排序
  → 上层确认
      → 移动 / 删除 / 合并元数据
```

Evaluator 不持有删除权限，也不执行文件操作。

## 3. 类型

```rust
pub struct Evaluator {
    cfg: Arc<AppState>,
    params: EvalParams,
}

#[derive(Clone, Debug)]
pub struct EvalParams {
    /// 有 XMP 编辑的版本优先保护。
    pub protect_edits: bool,
    /// 有评分、标签或色标的版本优先保护。
    pub protect_metadata: bool,
    pub weights: QualityWeights,
}

#[derive(Clone, Debug)]
pub struct QualityWeights {
    pub sharpness: f32,
    pub resolution: f32,
    pub exposure: f32,
    pub compression: f32,
    pub format: f32,
}

#[derive(Clone, Debug)]
pub struct EvalCandidate {
    pub photo: AlbumPhotoItem,
    /// 重复组或连拍组 id；只用于分组输出，不参与评分。
    pub group_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EvaluatedPhoto {
    pub path: String,
    pub score: f32,
    pub rank: usize,
    pub recommended: bool,
    pub reasons: Vec<EvalReason>,
}

#[derive(Clone, Debug)]
pub struct Evaluation {
    pub ranked: Vec<EvaluatedPhoto>,
    pub recommended_path: String,
    pub confidence: f32,
    pub exact_duplicates: bool,
}
```

没有 `KeeperPolicy` 枚举。排序始终是自动质量排序。

## 4. 参数校验

```rust
impl EvalParams {
    pub fn normalized(mut self) -> Result<Self, EvalError> {
        let sum = self.weights.sharpness
            + self.weights.resolution
            + self.weights.exposure
            + self.weights.compression
            + self.weights.format;
        if sum <= f32::EPSILON {
            return Err(EvalError::InvalidWeights);
        }
        self.weights.sharpness /= sum;
        self.weights.resolution /= sum;
        self.weights.exposure /= sum;
        self.weights.compression /= sum;
        self.weights.format /= sum;
        Ok(self)
    }
}
```

默认权重：

```text
sharpness     0.35
resolution    0.25
exposure      0.15
compression   0.15
format        0.10
```

## 5. 输入信号

从现有 `AlbumPhotoItem` 和相册目录读取：

```rust
pub struct PhotoSignals {
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub datetime: Option<NaiveDateTime>,
    pub has_xmp: bool,
    pub is_raw: bool,
    pub rating: u8,
    pub has_tags: bool,
    pub has_color_labels: bool,
}
```

已有字段：

- `width` / `height`
- `size`
- `datetime`
- `rating`
- `tags`
- `color_labels`

额外读取：

- XMP sidecar 是否存在；
- 扩展名是否属于 RAW；
- 必要时读取格式/位深。

## 6. 画质指标

```rust
pub struct QualityMetrics {
    pub sharpness: f32,
    pub highlight_clip: f32,
    pub shadow_clip: f32,
    pub contrast: f32,
    pub bits_per_pixel: f32,
}
```

### 6.1 清晰度

使用 Laplacian variance：

```text
preview / thumbnail
→ 灰度化
→ 缩放到固定长边（例如 1024）
→ 轻度高斯模糊
→ Laplacian 卷积
→ 方差
```

Rust 侧使用现有 `image` crate 解码和缩放，自己实现 3×3 卷积即可，不引入 OpenCV。

注意：

- 必须在同一缩放尺寸下比较；
- 对过于锐化的图片，轻度模糊可以抑制噪声；
- RAW 使用内嵌 preview，不解码整张 RAW；
- 清晰度只做组内相对比较。

### 6.2 曝光与 clipping

```text
亮度直方图
→ 高光 clipping 比例
→ 阴影 clipping 比例
→ 平均亮度 / 对比度
```

**clipping 不是“亮度超过亮度空间”，而是像素值已经到达当前表示范围的上限或下限。**

照片最终是一个有界量化空间：

- 8-bit JPEG：`0..255`；
- 16-bit TIFF/PNG：`0..65535`；
- RAW：传感器/ADC 的某个黑电平到饱和电平。

场景亮度可以比传感器范围更亮。高光一旦超过传感器饱和点，ADC 只能把它记成最大码值；之后无论场景再亮多少，像素都还是那个最大值。于是原本有细节的云、灯罩或白色衣服，变成一整片相同的 `255`。

同理，阴影低于黑电平/噪声底后，细节被记成接近 `0` 的值。

所以：

```text
高光 clipping 比例 = 接近最大码值的像素数 / 总像素数
阴影 clipping 比例 = 接近最小码值的像素数 / 总像素数
```

它是“多少像素已经贴到边界、丢失了层次”，不是“亮度超过了 255”。

实际计算时不会只数精确的 `255/0`，而会留阈值和容差：

```text
高光：Y >= 250（8-bit）或接近白点
阴影：Y <= 5（8-bit）或接近黑点
```

更细的做法是分 RGB 通道统计。RAW 的高光常常先在一个通道饱和，导致颜色偏移；只看灰度会漏掉这种 clipping。

注意：

- 预览图通常已经过相机/RAW 的 tone curve，预览上的 clipping 比例不等于 RAW 传感器 clipping；
- 小面积镜面反光、灯泡本身可以合法地 clip，不能只凭一个比例就判差；
- 评估器应把 clipping 作为**惩罚项**，并对小面积高光设上限，而不是追求 0 clipping；
- 同一组照片用同一套预览/阈值比较，相对排序才有意义。

过曝或欠曝扣分，但不把刻意的低调/高调照片直接判死；曝光只占 0.15 权重。

### 6.3 压缩质量

```text
bits_per_pixel = file_size * 8 / (width * height)
```

同分辨率、同内容下作为辅助指标。文件大小也受元数据影响，不能单独决定 keeper。

### 6.4 格式

排序时参考：

```text
RAW / TIFF / PNG > 高质量 JPEG > WebP > 低质量 JPEG
```

RAW + JPEG 应被识别为“同一拍摄的不同格式”，不能直接按普通重复文件清理。

## 7. 排序流程

### 7.1 保护层

先判断：

```text
has_xmp
rating > 0
tags / color_labels
is_raw
```

推荐保留有编辑的版本。保护层优先于画质分，避免删除带 XMP 的版本后丢失调色。

### 7.2 exact / near 分开

```text
exact：
  画质基本相同
  → 保护层
  → 更早拍摄时间
  → 更短/更稳定的路径
  → stable id

near：
  → 计算质量指标
  → 组内归一化
  → 加权评分
```

### 7.3 归一化

```text
sharpness_norm = minmax(sharpness)
resolution_norm = minmax(log(width * height))
exposure_norm = 1 - clipping_penalty
compression_norm = minmax(bits_per_pixel)
format_norm = format_priority
```

### 7.4 评分

```rust
score = 100.0 * (
    w.sharpness * sharpness_norm
  + w.resolution * resolution_norm
  + w.exposure * exposure_norm
  + w.compression * compression_norm
  + w.format * format_norm
);
```

### 7.5 稳定排序

```rust
ranked.sort_by(|a, b| {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.path.cmp(&b.path))
});
```

不能依赖 HashMap 顺序或异步完成顺序。

### 7.6 置信度

```text
confidence = clamp((top.score - second.score) / 10.0, 0.0, 1.0)
```

第一和第二差距很小时仍给出推荐，但界面显示“建议对比后决定”。

## 8. 推荐理由

返回结构化 code，由前端翻译：

```rust
pub enum EvalReasonCode {
    HasEdits,
    HigherResolution,
    Sharper,
    BetterExposure,
    LessCompression,
    RawOriginal,
    EarlierCapture,
    LargerFile,
}

pub struct EvalReason {
    pub code: EvalReasonCode,
    pub value: Option<f32>,
}
```

界面示例：

```text
推荐保留 A.jpg
- 有 XMP 编辑
- 清晰度高 18%
- 分辨率相同
- 文件更大
```

## 9. Evaluator 接口

```rust
impl Evaluator {
    pub fn new(cfg: Arc<AppState>, params: EvalParams) -> Result<Self, EvalError> {
        Ok(Self { cfg, params: params.normalized()? })
    }

    pub async fn rank(
        &self,
        owner: &str,
        album: &str,
        candidates: &[AlbumPhotoItem],
    ) -> Result<Evaluation, EvalError> {
        let signals = self.load_signals(owner, album, candidates).await?;
        let metrics = self.load_quality_metrics(owner, album, candidates).await?;
        Ok(rank_candidates(candidates, &signals, &metrics, &self.params))
    }
}
```

`rank_candidates` 是同步纯函数，便于单元测试。

## 10. 画质缓存

```text
cache/ai/{owner}/albums/{cache_id}/quality/{content_signature}.json
```

```json
{
  "schema_version": 1,
  "sharpness": 0.82,
  "highlight_clip": 0.01,
  "shadow_clip": 0.04,
  "contrast": 0.31,
  "bits_per_pixel": 1.8
}
```

- key 使用内容签名；
- 文件变化后自动失效；
- 只缓存指标，不缓存推荐结果；
- 参数变化不需要清缓存，评分是纯函数；
- 不使用 AlbumIndex 保存质量指标。

## 11. 模块划分

```text
src/backend/album/evaluator.rs
  Evaluator
  EvalParams
  评分、排序、理由
  Laplacian / 曝光 / bpp
  quality cache 读写
```

质量指标和排序暂时在同一个源文件里，避免第一版就拆出没有调用方的抽象。

重复检测和连拍折叠只负责构造 `EvalCandidate`，调用 Evaluator。

## 12. API

重复组列表接口直接为当前页每组附加 `recommended_path`：

```text
GET /api/ai/duplicates?limit=20
```

```json
{
  "groups": [
    {
      "group_key": "near:...",
      "kind": "near",
      "photo_count": 3,
      "recommended_path": "albums/trip/A.jpg",
      "photos": [...]
    }
  ]
}
```

推荐在分页后的当前页计算，避免为全部重复组解码预览。失败或没有可用索引时省略该字段，列表仍然正常返回。

单独的评估接口和连拍组推荐接口还没接入。

## 12.1 重复图像界面

重复组缩略图右上角使用三角角标，内部显示黄色星星：

```text
┌──────────────┐
│           ◤★ │
│   缩略图      │
└──────────────┘
```

- 仅 `recommended_path` 对应的照片显示；
- 同时加一圈淡黄色描边；
- 不阻塞点击打开照片；
- 没有推荐结果时不显示任何标记。

连拍组：

```text
POST /api/albums/stack/evaluate
{
  "album": "...",
  "stack_id": "..."
}
```

权限在入口处检查：

- 读权限可以评估；
- 删除、移动需要写/删除权限；
- 不信任前端传来的任意路径。

## 13. 删除与清理

Evaluator 只给推荐，不自动删除。

真正清理前需要：

- 显示将删除的文件数和总大小；
- 二次确认；
- 保护 XMP；
- 优先移动到回收目录，而不是永久删除；
- 支持撤销；
- 逐张检查相册权限；
- 跨相册重复组不能用一个相册权限批量删除。

如果 keeper 和待删除照片有不同 XMP/评分/标签，应先提示合并或保留。

## 14. 测试

纯评分函数测试：

1. 有 XMP 的版本优先；
2. 同分辨率下更清晰的排第一；
3. 严重过曝扣分；
4. exact 组排序稳定；
5. 分数相同按路径稳定；
6. 权重归一化后结果可复现；
7. 空组/单张组有明确结果或错误；
8. 非法权重被拒绝；
9. 输入顺序变化不改变结果；
10. 同一输入重复运行结果一致。

## 15. 实现顺序

1. `EvalParams` / `EvalCandidate` / `Evaluation` 类型；
2. 纯函数 `rank_candidates` + 单测；
3. 元数据/XMP 信号加载；
4. quality metrics + 缓存；
5. 重复组 API 接入；
6. 连拍组 API 接入；
7. 前端推荐、理由、对比；
8. 安全清理流程。

第一版只做自动质量排序，不引入策略枚举，也不引入 AI 审美模型。
