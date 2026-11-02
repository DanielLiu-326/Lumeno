# 调色管线设计

## 1. 目标

做一条非破坏的调色管线：原片只读，编辑结果是一份可序列化的节点文档。同一份文档既能出交互预览，也能出全尺寸导出。节点在固定的颜色域之间传递缓冲，参数用强类型保存，版本可演进。

首版节点：

| 节点 | 作用 |
| --- | --- |
| Input | 读入 RAW 马赛克或已编码光栅图 |
| WhiteBalance | 缩放 RAW 的 RGB 通道，平衡白色并辅助去马赛克 |
| Demosaic | 已白平衡的马赛克到线性相机 RGB |
| Input Color Profile | 线性相机 RGB 到线性 Rec.2020，可用相机自带矩阵或导入的配置 |
| Rotate | 旋转 |
| Crop | 裁切 |
| Exposure | 曝光与黑电平偏移 |
| LocalContrast | 局部对比 |
| Dehaze | 去雾 |
| Velvia | 按饱和度加权的增色，低饱和区域动得更多 |
| Denoise | 保边降噪，亮度和色度分开衰减 |
| Sharpen | 小半径反锐化，只抬边缘 |
| Sigmoid | 场景线性到显示线性 |
| Lut | 三维 LUT |
| Blend | 把多路图像合成回一条缓冲，每路可带自己的掩码 |
| MaskDraw | 用形状生成连续掩码 |
| MaskCreator | 按像素的亮度、色度或色相生成连续掩码 |
| MaskCombine | 把多张掩码做连续并、交、差，或羽化 |
| Export | 输出编码，写成文件 |

工作空间固定为线性 Rec.2020。白平衡留在 RAW 马赛克上，不进入工作空间。去马赛克得到线性相机 RGB。Input Color Profile 再把它转到工作空间：缺省用这张 RAW 的相机矩阵，也可以导入一份相机色彩配置。光栅图在 Input 里直接进入工作空间，不经过这个节点。

## 2. 范围

首版包含：

- 文档模型、校验、规范化 JSON
- 四种数据：马赛克、线性 RGB、显示 RGB、连续掩码
- 带类型的 `InputDesc` / `OutputDesc`，以及管线的执行顺序和环检测
- 上述节点的参数、公式，以及可分叉的管线
- 同步执行器，预览与导出共用
- 预览代理、导出任务、按文档哈希缓存
- 只读资源管理：LUT、预设等大对象按内容哈希存放，文档只存引用
- 库的模块划分和对外接口

首版不包含：

- 独立的图层栈。叠化由分叉、Blend 和连续掩码完成
- 画笔涂抹。形状掩码用参数光栅化，不记录笔触
- 用户自定义工作空间
- 视频
- GPU 内核
- 分块流水线（内存模型按整幅图设计，接口预留瓦片）

## 3. 处理模型

一份文档是一条管线。节点按上下游串起来，可以从一个输出分出几路，再由 Blend 汇合。图像默认只有一个输入、一个输出，名字都是 `image`，分别用 `InputDesc` 和 `OutputDesc` 描述。`types` 列出这一个输入或输出能承载的种类，一次运行只放其中一种，不按马赛克、线性 RGB、显示 RGB 拆成多项。同一个 `image` 输出可以接到多个下游的 `image` 输入。掩码是另一项输入或输出。要把多路合成成一路时，才按参数展开额外的 `InputDesc`。

Blend 用来代替图层栈：底图是一路输入，其余路是叠上去的图像。掩码是另一类数据，用来控制某一路效果的强弱，不代替这条图像分叉。

```text
Input → WhiteBalance → Demosaic
                      ├─ image → Exposure ─────────────┐
                      │            ▲ mask              │
                      │            │                   ├─ Blend → Sigmoid → Lut → Export
                      └─ image → MaskCreator ───────┘
                      └─ image → LocalContrast ─ Dehaze ┘
```

没有分叉、也没有掩码时，每个节点就是一条 `image` 进、一条 `image` 出。

### 3.1 输入与输出

```rust
pub enum DataType {
    Mosaic,
    LinearRgb,
    DisplayRgb,
    Mask,
}

pub struct InputDesc {
    pub name: &'static str,
    /// 这一个输入能接受的种类。一次运行只接到其中一种。
    pub types: &'static [DataType],
}

pub struct OutputDesc {
    pub name: &'static str,
    /// 这一个输出可能写出的种类。一次运行只写出其中一种。
    pub types: &'static [DataType],
}
```

图像输入只有一条 `InputDesc`，名字是 `image`，只接一条上游。`types` 里可以同时有 `Mosaic`、`LinearRgb`、`DisplayRgb`。连线时看上游这次实际写出的那一种是否落在下游 `InputDesc.types` 里，落在里面就允许。节点不因为能接两种缓冲就再写一条图像输入。

要合并多路时，不在同一条输入上叠多条上游，而是由参数决定 `InputDesc` 的条数。例如 Blend 的 `layers` 有 `n` 项，`inputs()` 就给出 `base`、`over_0` … `over_{n-1}`，以及可选的 `mask_0` … `mask_{n-1}`。每一路是独立的 `InputDesc`，各接一条上游。MaskCombine 用自己的路数参数同样展开 `in_0`、`in_1`。其余节点的图像输入不随参数变化，始终是一个 `image`。

输入和输出由 `GradeNode::inputs(param)`、`GradeNode::outputs(param)` 根据参数算出。节点不声明它们可以和谁相连。能不能连，只看类型：上游 `OutputDesc` 这次的类型落在下游 `InputDesc.types` 里就允许，否则拒绝这条连接。创建节点只是往 `nodes` 里加一项，不要求当时就接上。

连接不属于节点。文档另有一张边表。节点的参数和实现都读不到这张表。加载顺序是固定的：

1. 反序列化全部 `Node`。此时边还没有解析。
2. 对每个节点调用 `inputs(param)` 和 `outputs(param)`，得到输入输出表。这里只传入参数。
3. 再反序列化 `Edge`，并按这张表接上管线。边指向不存在的输入或输出，或一条输入被两条边占用，都是文档错误。

```rust
pub struct Edge {
    pub from_node: NodeId,
    pub from_output: String,
    pub to_node: NodeId,
    pub to_input: String,
}
```

有 `InputDesc`、却一条输入边都没有的节点，执行时不调用 `run`，不写任何输出，因此不生效。Input 没有输入，不在此列。多条输入里只缺一部分时，执行器仍然调用 `run`，`inputs` 里只有接上了、并且上游确实写出了值的那些名字。缺哪个、怎么继续，由该节点自己决定。曝光的 `mask` 不在表里时，效果作用在整幅图上。Blend 的 `mask_i` 不在表里时，这一层的权重只有 `opacity`。Blend 的 `base` 或某个 `over_i` 不在表里时，这一路不参与合成。图像效果节点缺了 `image` 时不写输出。

执行器按上下游排序：一条边是「上游节点 → 下游节点」，同一对节点之间的多条连接只算一条依赖。没有上游的节点先执行。排序结束后仍有节点没排到，说明管线绕回了自己，`validate` 返回 `Cycle`，并带上这些节点 id。自己连自己也算绕回。

### 3.2 掩码

掩码是一张与图像等大的 `f32` 单通道图，值域 `[0, 1]`。`0` 表示该像素不施加效果，`1` 表示完全施加，中间值按比例混合。它不是二值图，也不在使用前做阈值。

消费端把非有限值当成 `0`，再夹到 `[0, 1]`。产生端应直接写出这个范围内的有限值。

掩码和算法的关系分三种。下表是全部节点的约定。

| 节点 | 掩码 | 语义 |
| --- | --- | --- |
| Input、WhiteBalance、Demosaic、Input Color Profile、Export | 不接受 | 白平衡必须整幅使用同一组通道系数，局部混合会让去马赛克看到不一致的增益；输入色彩配置必须整幅使用同一组矩阵，否则工作空间不再一致。其余节点在这里改变数据类型 |
| Rotate、Crop | 变换后从 `mask` 输出 | 掩码和图像做同一次旋转或裁切，像素仍对齐。不在这里把效果混回原图 |
| Exposure、LocalContrast、Dehaze、Velvia、Denoise、Sharpen、Lut | 可选 `mask` 输入 | 先对整幅图算效果，再 `out = in * (1 - w) + processed * w`，`w` 为掩码 |
| Sigmoid | 可选 `mask` 输入 | 输入输出类型不同。混合发生在显示线性：`out = display_from_linear(in) * (1 - w) + sigmoid(in) * w` |
| Blend | 每层可选 `mask_i` | 掩码乘上该层的 `opacity`，只影响这一层的叠化权重，不再把整个 Blend 的结果和底图混一次 |
| MaskDraw、MaskCreator、MaskCombine | 产生 `mask` | 这些节点的输出就是掩码 |

`w` 在整幅图上逐像素计算。没接掩码时 `w = 1`。节点关闭时，`image` 输出等于 `image` 输入，掩码不参与。

几何节点关闭时，`image` 原样通过；若接了掩码，`mask` 输出也原样通过。没接掩码却有人读取 `mask` 输出，校验失败。

缓冲有四种：

| 缓冲 | 数据 | 含义 |
| --- | --- | --- |
| `Mosaic` | `u16`，单通道 | 传感器马赛克，带 CFA、黑白电平 |
| `LinearRgb` | `f32`，RGB 打包 | 场景线性，线性 Rec.2020 |
| `DisplayRgb` | `f32`，RGB 打包 | 显示参考线性光，名义范围 0–1，尚未做 sRGB OETF |
| `Mask` | `f32`，单通道 | 连续权重，`0` 不施加效果，`1` 完全施加 |

光栅输入在 Input 内完成逆 OETF，直接得到 `LinearRgb`。RAW 输入得到传感器码值形式的 `Mosaic`。WhiteBalance 把它变成线性、RAW、场景参考的 `Mosaic`。Demosaic 得到线性相机 RGB。Input Color Profile 再进入工作空间里的 `LinearRgb`。Sigmoid 进入 `DisplayRgb`。Export 读取 `DisplayRgb`，做 OETF 和编码，不把像素交回管线。

```mermaid
flowchart LR
  mosaic[Mosaic]
  linear[LinearRgb]
  display[DisplayRgb]
  file[编码文件]
  mosaic --> linear
  linear --> display
  display --> file
```

点运算节点（白平衡、曝光、Velvia、Sigmoid、LUT、Blend、掩码组合）逐样本，无邻域。白平衡按 CFA 颜色取系数，仍然是点运算。邻域节点（去马赛克、局部对比、去雾、降噪、锐化、掩码羽化）声明光环半径。旋转和裁切改变分辨率，是几何屏障：它们祖先上的检查点不能给尺寸已经不同的下游复用。接到同一个节点的图像必须同一域、同一宽高；掩码必须和被它控制的那张图像等大。

## 4. 库结构

独立库 `grade`。宿主负责读原片、存文档、存缓存、跑任务队列。库不依赖具体 HTTP 框架和相册目录。

```text
grade/
  Cargo.toml
  src/
    lib.rs                 对外再导出
    document.rs            文档、节点枚举
    validate.rs            结构与域校验
    canon.rs               规范化 JSON 与内容哈希
    buffer.rs              四种数据、几何信息
    pipeline.rs            输入输出、执行顺序、环检测
    preset_policy.rs       按照片条件选择缺省预设
    color.rs               矩阵、OETF、亮度系数、色温
    resource.rs            资源标识、解析缓存、引用计数
    exec.rs                执行器与执行上下文
    cache.rs               渲染缓存键、检查点接口
    error.rs               错误类型
    cancel.rs              取消令牌
    nodes/
      mod.rs               Node trait，按类型名构造
      input.rs
      demosaic.rs
      white_balance.rs
      rotate.rs
      crop.rs
      exposure.rs
      local_contrast.rs
      dehaze.rs
      velvia.rs
      denoise.rs
      sharpen.rs
      sigmoid.rs
      lut.rs
      blend.rs
      mask_draw.rs
      mask_creator.rs
      mask_combine.rs
      export.rs
    io/
      mod.rs
      raw.rs               RawSource trait
      raster.rs            RasterSource trait
      cube.rs              .cube 解析
      encode.rs            编码 trait
```

依赖保持少：`serde`、`serde_json`、图像编码可用宿主注入，库内只定义 `EncodeSink`。去马赛克的高质量算法可以放在 `nodes/demosaic/` 子模块（`bilinear.rs`、`rcd.rs`），不要拆成第二个 crate。

### 4.1 模块职责

| 模块 | 职责 | 不做的事 |
| --- | --- | --- |
| `document` | 拥有节点参数和边表 | 不读文件、不内嵌 LUT 或预设正文；节点结构体里不放边 |
| `pipeline` | 读边表、检查类型、求执行顺序、找出绕回，再把值送到输入 | 不把边或其他节点交给节点实现 |
| `preset_policy` | 用过滤规则为尚未编辑的照片选择预设 | 不修改已经编辑过的照片文档 |
| `resource` | 按内容哈希存取共享只读资源，内存里复用已解析对象 | 不存渲染结果，不改文档 |
| `validate` | 保存前和执行前检查 | 不修改文档 |
| `canon` | 稳定字节序列，供哈希 | 不改变内存中的参数精度 |
| `buffer` | 拥有像素和元数据 | 不含算法 |
| `color` | 纯函数：矩阵、gamma、色适应 | 不分配整幅图 |
| `nodes::*` | 单节点公式 | 不决定调度 |
| `exec` | 按 `pipeline` 给出的顺序调用节点，按输出名存结果，响应取消 | 不解释 HTTP |
| `cache` | 定义渲染缓存键和检查点 trait | 不存放 LUT / 预设 |
| `io` | 把宿主字节变成 `Mosaic` / `LinearRgb`，把 `DisplayRgb` 写成编码字节；解析 `.cube` | 不包含节点参数 |

### 4.2 对外接口

宿主只需要这些入口：

```rust
pub struct GradeDocument { /* 见第 5 节 */ }

pub fn validate(doc: &GradeDocument) -> Result<(), GradeError>;
pub fn canonical_json(doc: &GradeDocument) -> Result<Vec<u8>, GradeError>;
pub fn content_hash(doc: &GradeDocument) -> Result<[u8; 32], GradeError>;

pub struct ExecRequest<'a> {
    pub document: &'a GradeDocument,
    pub resources: &'a ResourceManager,
    pub source: Source<'a>,
    pub target: ExecTarget,
    pub cancel: &'a CancelFlag,
    pub checkpoints: Option<&'a dyn CheckpointStore>,
}

pub enum Source<'a> {
    Raw(&'a dyn RawSource),
    Raster(&'a dyn RasterSource),
}

pub enum ExecTarget {
    /// 长边不超过 max_edge，去马赛克可降为 bilinear。
    Preview { max_edge: u32 },
    /// 按 Export 节点编码。
    Export,
}

pub fn execute(req: ExecRequest<'_>) -> Result<ExecOutput, GradeError>;

pub enum ExecOutput {
    Preview(EncodedImage),
    Export(EncodedImage),
    /// Export 没有运行，本次没有成片。文档仍然合法。
    Inactive,
}
```

`execute` 是同步函数。宿主把它放到阻塞线程池。取消通过 `CancelFlag`（内部 `AtomicBool`）在节点之间以及邻域节点的行循环里检查。

## 5. 文档模型

```rust
pub struct GradeDocument {
    pub version: u32,                 // 当前为 1
    pub working_space: WorkingSpace,  // 仅 LinearRec2020
    /// 链接套用时指向一份预设资源。钉死或已编辑时为 None，节点在 `nodes` 里。
    pub preset: Option<ResourceRef>,
    /// 仅照片文档使用。定义见第 5.1 节。预设资源序列化时省略，读入按 `Edited`。
    pub origin: GradeOrigin,
    pub nodes: Vec<GradeNode>,
    pub edges: Vec<Edge>,
}

pub enum WorkingSpace {
    LinearRec2020,
}

pub struct GradeNode {
    pub id: NodeId,             // 文档内唯一，稳定字符串。只作边表的键，节点逻辑不读取它
    pub enabled: bool,
    /// 参数在 `NodeParam` 上。端口由参数决定，公式在 `Node` 上。
    pub param: Box<dyn Node>,
}
```

`GradeNode` 序列化为带 `type` 标签的对象：

```json
{
  "nodes": [
    {
      "id": "blend-1",
      "enabled": true,
      "type": "blend",
      "params": { "layers": [{ "mode": "normal", "opacity": 0.8 }] }
    }
  ],
  "edges": [
    { "from_node": "exposure-1", "from_output": "image", "to_node": "blend-1", "to_input": "base" },
    { "from_node": "contrast-1", "from_output": "image", "to_node": "blend-1", "to_input": "over_0" },
    { "from_node": "mask-1", "from_output": "mask", "to_node": "blend-1", "to_input": "mask_0" }
  ]
}
```

`layers` 有几项，就有几个 `over_i`。`mask_i` 可以没有对应的边。没有输入的节点，例如 Input，在边表里不作为 `to_node` 出现。读文件时先构造节点并算出 `InputDesc` / `OutputDesc`，再解析这些边。

`enabled: false` 的节点保留在数组里，执行时跳过，校验时仍检查它若启用是否合法。这样关闭 Sigmoid 不会丢掉用户调过的对比度。改了参数的补丁会把该节点打开；只有参数与节点里现有的相同，才按补丁里的 `enabled`。节点用 `NodeParam::always_on` 声明自己不能关闭，缺省返回 false。去马赛克、输入色彩配置和导出覆盖为 true。补丁在参数不变时把这种节点设成 `enabled: false`，整次修改被拒绝，执行时它也照常跑。

文档只保存标量和 `ResourceRef`。LUT 表、预设正文都不写入这份 JSON。预设本身是一种资源，见第 10 节。照片文档通过引用指向它，套用时不复制预设正文。

### 5.1 缺省管线

管线可以先不接满。一张照片在缺省状态下却总是有一条完整管线：打开它时，若还没有这份照片自己的调色文档，就按规则套上一条预设，再显示和导出。

解析只发生在这一次。文档写入之后，再次打开直接读文档，不再重算规则。用户改过任意节点或连线之后，文档标记为已编辑，规则不再覆盖它。用户把这张照片恢复缺省时，删掉该文档，下次打开重新解析。

```rust
pub struct PresetPolicy {
    /// 靠前的规则先匹配。用户没写过规则时，这个数组为空。
    pub rules: Vec<PresetRule>,
}

pub struct PresetRule {
    pub id: String,
    pub enabled: bool,
    pub when: FilterExpr,
    pub preset: ResourceRef,
}

pub enum FilterExpr {
    All(Vec<FilterExpr>),
    Any(Vec<FilterExpr>),
    Not(Box<FilterExpr>),
    Kind(MediaKind),
    CameraMake(String),
    CameraModel(String),
    Iso { min: Option<u32>, max: Option<u32> },
    FocalLengthMm { min: Option<f32>, max: Option<f32> },
}

pub enum MediaKind {
    Raw,
    Jpeg,
    Heif,
    Tiff,
    Other,
}

pub struct PhotoFacts {
    pub kind: MediaKind,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f32>,
}
```

`resolve(policy, facts)` 从上到下找第一条启用且匹配的规则。机身和镜头按不区分大小写的包含关系比较。ISO 和焦距缺字段时，带范围的条件不匹配。`All` 为空视为真，`Any` 为空视为假。

没有任何规则命中时，使用系统预设。用户从来没有设计过预设，就是规则表为空，于是每张照片都走系统预设。规则表不空、但这张照片一条都没中，同样落到系统预设，保证缺省状态不会变成空图。

| 照片种类 | 系统别名 | 管线 |
| --- | --- | --- |
| `Raw` | `alias:preset:system-raw` | Input → WhiteBalance(`as_shot`) → Demosaic(`rcd`) → Sigmoid(`contrast = 1.5`) → Export |
| 其余 | `alias:preset:system-raster` | Input → Exposure(0) → Sigmoid(关闭) → Export |

光栅身份链是 `system-raster`：编码线性化后再编码。曝光为 0、Sigmoid 关闭、无几何和 LUT 时，除 8 位量化外像素应回到原编码。测试允许 ±1 个编码量化级。

系统预设随库发布，启动时写入资源库并占住这两个别名。用户规则引用自己的预设别名，这样用户改预设后，尚未编辑的照片下次打开会看到新版本。已经编辑并脱离链接的照片保持脱离时的节点。

解析结果写成链接套用，不把预设正文抄进照片 JSON：

```rust
pub enum GradeOrigin {
    Rule { id: String },
    System { kind: MediaKind },
    Edited,
}
```

`GradeOrigin` 放在照片文档上。预设资源本身不带这个字段。缺省文档的 `preset` 指向选中的别名，`nodes` 为空，`origin` 为 `Rule` 或 `System`。用户第一次改节点时，把当时展开的预设抄进 `nodes`，清空 `preset`，`origin` 改为 `Edited`。

### 5.2 规范化与哈希

`canonical_json` 规则：

- 对象键按字典序
- 节点按 `id` 字典序写出。边按 `to_node`、`to_input`、`from_node`、`from_output` 排序
- 浮点用最短 round-trip 格式，再按 6 位小数对齐到文档值（保存时参数已经量化到 1e-6）
- 不含源路径、mtime、缓存路径

内容哈希是规范化 JSON 的 SHA-256。JSON 里的资源引用已经是内容哈希，因此换一张 LUT 或换一份预设会改变文档哈希。缓存键另加源身份（宿主提供的 32 字节）和 `ExecTarget` 的规范编码。别名引用在执行前先解析成内容哈希，解析结果计入缓存键，不写回文档。

## 6. 校验

`validate` 在 PUT 和 `execute` 之前运行。失败即不执行。

结构规则：

- `version == 1`，`working_space == LinearRec2020`
- 节点 `id` 非空且唯一，长度 ≤ 64
- 恰好一个 Input。它没有 `InputDesc`，只有一条 `OutputDesc`，名字是 `image`。这个输出的类型是 `Mosaic` 或 `LinearRgb`，由源决定，不拆成两项
- 恰好一个 Export。它有一条 `image` 输入，没有输出。这条边可以先不接
- 每条已写的连线都指向存在的 `InputDesc`；上游节点存在，`from_output` 是它的一条 `OutputDesc`，且上游这次的类型属于下游 `types`。类型不符拒绝这条边。一条输入被两条边占用，也是文档错误
- 节点可以没有任何边。缺线不是校验错误
- 管线不能绕回已经走过的节点。用 Kahn 算法：依赖按「上游节点 → 下游节点」去重。排不完即 `Cycle`
- 到不了 Export 的节点不执行，也不算错
- 禁用节点的已有连线仍然要类型合法
- `ResourceRef` 语法合法：`Pinned` 的种类与字段匹配（LUT 节点是 `Lut`，`preset` 字段是 `Preset`）。此步不读磁盘

类型沿 `InputDesc` / `OutputDesc` 检查，不靠节点在数组里的位置。一条没有分叉的 RAW 路径看起来是：

```text
Mosaic → LinearRgb → … → DisplayRgb → Export
```

Sigmoid 可以出现在不同支路上，每条支路各自最多一个，并且位于该支路的 LUT 之前。Export 的直接输入若仍是 `LinearRgb`，执行器在这条边上插入内部步骤 `display_from_linear`：把线性 Rec.2020 转到线性 sRGB 并裁到 0–1。这一步不出现在文档里，也不插进 Blend 的某一路。

尺寸不在校验阶段计算。执行时，Blend 的各路图像宽高不一致，或掩码和被它控制的图像宽高不一致，返回 `SizeMismatch`。Blend 不缩放、不平移。支路各自裁切时，要裁成同一矩形才能汇合；掩码要接在几何节点的 `mask` 输出上，才能和裁切后的图像对齐。`LinearRgb` 上的 Blend 若使用 `screen`、`overlay` 或 `difference`，校验失败。

参数规则见各节点。越界参数在校验阶段拒绝，执行阶段信任校验结果。

## 7. 缓冲

```rust
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

pub struct MosaicBuffer {
    pub size: ImageSize,
    pub cfa: CfaPattern,          // 2×2 或 6×6 相位
    pub black_level: f32,         // 每通道可再扩展；首版单值
    pub white_level: f32,
    pub camera_to_xyz_d50: [f32; 9], // DNG ColorMatrix，行主序；缺失时全 0
    pub as_shot_gains: [f32; 3],  // R,G,B，G = 1
    pub linear: bool,             // false：传感器码值；true：线性 RAW
    pub samples_code: Vec<u16>,   // linear = false，行主序
    pub samples_linear: Vec<f32>, // linear = true，行主序，场景参考
}

pub struct LinearBuffer {
    pub size: ImageSize,
    pub space: WorkingSpace,
    pub rgb: Vec<f32>,            // RGBRGB…，长度 width*height*3
}

pub struct DisplayBuffer {
    pub size: ImageSize,
    pub rgb: Vec<f32>,            // 显示线性，名义 0–1
}

pub struct MaskBuffer {
    pub size: ImageSize,
    pub samples: Vec<f32>,        // 单通道，名义 0–1，长度 width*height
}

pub enum Value {
    Mosaic(MosaicBuffer),
    Linear(LinearBuffer),
    Display(DisplayBuffer),
    Mask(MaskBuffer),
}
```

布局固定为交错 RGB，便于点运算一次扫描。邻域算法内部可以临时拆平面，节点返回前再打包。

归一化约定：

- 白平衡把码值减黑电平后除以 `(white_level - black_level)`，再乘通道系数，得到线性 RAW。高光重建之前允许大于 1。
- 工作空间里的值是场景线性，中灰不强制映射；曝光节点以 2 的幂作用在当前值上。
- 显示缓冲在 Sigmoid 输出处裁切到 `[0, 1]`。Export 再做 sRGB OETF。
- 掩码在消费前夹到 `[0, 1]`。`NaN` 和无穷当成 `0`。

内存：图像一层 `f32×3`，2400 万像素约 288MB。掩码一层约 96MB。每个输出上的值放进 `Arc`，下游只克隆指针。某个输出的全部下游都完成后，执行器丢掉这个 `Arc`。首版因此要求宿主对全尺寸导出做全局并发 1。预览长边默认 1600。

## 8. 执行器

```rust
pub struct ExecCtx<'a> {
    pub target: ExecTarget,
    pub cancel: &'a CancelFlag,
}

impl CancelFlag {
    pub fn cancel(&self);
    pub fn is_cancelled(&self) -> bool;
}
```

主循环按第 3.1 节的执行顺序进行。结果表是 `HashMap<(NodeId, OutputName), Arc<Value>>`，这张表只存在于 `pipeline` / `exec`。到不了 Export 的节点不进入这个循环。

```text
order = 执行顺序(能到达 Export 的节点)
for node in order
    若 ctx.cancel：返回 Cancelled
    若该节点有 InputDesc，且一条输入边都没有：跳过，不写输出
    inputs = 已接上且上游写出了值的输入名      // 缺的输入不在表里；节点看不到这些值来自谁
    若节点关闭：按直通规则从 inputs 抄到输出，不调用 run
    否则 outputs = node.run(inputs, ctx)
    按该节点的 OutputDesc 名字写入结果表
    丢掉已经没有未执行下游的输出
若 Export 没有写出编码结果：返回 Inactive
否则返回该编码结果
```

调用 `run` 时不传入文档、边表或其他节点。`ExecCtx` 只有取消标志和本次是预览还是导出。节点关闭且 `image` 在 `inputs` 里时，执行器把该值抄到 `image` 输出；掩码节点改为在 `mask` 输出放一张全 `1` 的掩码；Rotate / Crop 关闭时，`mask` 在 `inputs` 里就把该值送到 `mask` 输出。缺的输入不造值。节点函数本身不被调用。

Export 的 `image` 输入只接受 `DisplayRgb`。边上若仍是 `LinearRgb`，`pipeline` 在调用 Export 之前做一次 `display_from_linear`。这是送进 Export 之前的适配，不是 Export 或 Sigmoid 的逻辑。

`ExecTarget::Preview` 时，`pipeline` 在送到下游之前把相机 RGB 按整数倍抽取到长边以内（缩放因子取 2 的幂），光栅图则在线性化之后做面积平均缩小。文档里的参数不改。解码器（LibRaw）已经把马赛克插值完，Demosaic 节点只是透传，没有"预览降为 bilinear"这回事。

检查点：内存里只保留进入正在改的那个节点的一帧。再改这个节点时从这一帧算起，不重跑上游。改到另一个节点时，换成进入那个节点的帧。不给每个节点留一份图像。

```rust
pub trait NodeParam: Send + Sync {
    fn node_type(&self) -> &'static str;
    fn to_json(&self) -> serde_json::Value;
    fn always_on(&self) -> bool { false }
    fn fields(&self) -> Vec<FieldSpec>;
    fn from_json(value: &serde_json::Value) -> Result<Self, String>
    where
        Self: Sized;
    fn set_json(&mut self, value: &serde_json::Value) -> Result<(), String>;
}

pub trait Node: Send + Sync {
    fn params(&self) -> &dyn NodeParam;

    fn inputs(param: &dyn NodeParam) -> Vec<InputDesc>
    where
        Self: Sized;
    fn outputs(param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized;
    fn render(param: &dyn NodeParam, frame: &Frame) -> Frame
    where
        Self: Sized;
    fn run(
        &self,
        inputs: &BTreeMap<String, Arc<Value>>,
        ctx: &ExecCtx,
    ) -> Result<BTreeMap<String, Value>, GradeError>;
    fn set_param(&mut self, value: &serde_json::Value) -> Result<(), String>;
    fn defaults(&self) -> Box<dyn Node>;
}
```

文档是宿主保存的值，`Pipeline::compile(&Document)` 把它变成执行器唯一接受的形状：从 input 到 export 的顺序、每个输入的生产者、每个节点能不能跑（`Runnable` / `Inactive` / `MissingInput`）、以及结构问题（重复 id、未知边、同一输入两个生产者、环、断链）。执行器只读 `Pipeline::order()`，不再自己从 export 回溯；"为什么没生效"先问 `Pipeline::status(id)`，不要靠猜。

`FieldSpec` 里的 `label`、`choices`、`display` 不装界面文案：凡是文字都是 `grade.*` 形式的 i18n key（节点标题用 `document::title_key`），由宿主查表翻译；数值和单位（`72px`、`0°`、`90`）以及 `X`/`Y`/`R`/`G`/`B`/`Omega`/`Epsilon` 这类符号原样显示。库本身不带任何一门语言的句子。

`Node` 不继承 `NodeParam`。同一个具体类型可以两个都实现，节点用 `params()` 把参数交出来。`fields` 在 `NodeParam` 上。`inputs` 和 `outputs` 只看参数，不接收 `self`，并标 `where Self: Sized`，因此不进 `dyn Node` 的虚表。文档保存 `Box<dyn Node>`。按类型名从 JSON 构造仍由 `param_from_json` 分发，因为那时还没有节点实例。

错误：

```rust
pub enum GradeError {
    InvalidDocument(String),
    TypeMismatch { node: NodeId, input: String },
    Cycle { nodes: Vec<NodeId> },
    MissingCameraMatrix,
    Resource(ResourceError),
    SizeMismatch { node: NodeId },
    Io(String),
    Cancelled,
}
```

库错误不带本地化文案。宿主把变体映射成自己的语言。

## 9. 节点规格

节点执行只读参数和输入上的值，往输出写值，并可以改自己的私有数据。它不读取边表，不按 id 查找别的节点，也不根据上游是哪种节点改变公式。输入和输出的条数只由 `GradeNode::inputs(param)` 和 `outputs(param)` 决定。`inputs` 缺键表示这条输入没有可用的值，节点按自己的语义处理，不向管线去要补线。下文若提到「接在某节点之后」，说的是管线的接法，不是该节点内部的分支。

公共约定：RGB 三通道同一公式时写成对 `v` 的标量函数。邻域半径向上取整到像素，且至少为 1。所有强度类参数在校验时夹紧到表格里的闭区间，执行时不再夹紧。掩码怎么混进各节点，以第 3.2 节的表为准。

### 9.1 Input

没有 `InputDesc`。输出只有一条 `OutputDesc`，名字是 `image`，`types` 为 `Mosaic` 与 `LinearRgb`。RAW 在这个输出上写传感器码值 `Mosaic`，光栅写 `LinearRgb`。两种来源共用这一条输出。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `source` | `raw` \| `raster` | — | 由宿主创建文档时写入 |

RAW：`RawSource` 提供马赛克、CFA、黑白电平、as-shot 增益、相机到 XYZ 的 ColorMatrix。矩阵缺失时校验可以通过（文档本身合法），执行到 `temperature` 模式的白平衡时返回 `MissingCameraMatrix`。`as_shot` 和 `gains` 不需要这张矩阵。Input Color Profile 在矩阵缺失时改用单位矩阵，不因此中断。

光栅：`RasterSource` 提供 8 或 16 位 RGB 和声明的编码。首版只接受 sRGB。逆 OETF（IEC 61966-2-1）：

```text
若 c <= 0.04045：  L = c / 12.92
否则：            L = ((c + 0.055) / 1.055) ^ 2.4
```

16 位先除以 65535，8 位除以 255，再用固定的 sRGB → 线性 Rec.2020 矩阵（D65）转到工作空间。输出 `LinearRgb`。光栅路径没有马赛克，也不经过白平衡。

`RawSource` / `RasterSource`：

```rust
pub trait RawSource: Send + Sync {
    fn mosaic(&self) -> Result<MosaicBuffer, GradeError>;
}

pub trait RasterSource: Send + Sync {
    fn encoded(&self) -> Result<RasterFrame, GradeError>;
}

pub struct RasterFrame {
    pub size: ImageSize,
    pub encoding: RasterEncoding,   // 首版 Srgb
    pub bit_depth: u8,              // 8 或 16
    pub rgb: Vec<u8>,               // 8 位交错；16 位用 Vec<u16> 的另一个变体
}
```

### 9.2 WhiteBalance

输入、输出都是 `Mosaic`。按 darktable 白平衡模块：缩放 RAW 的 RGB 通道以平衡白色，并辅助去马赛克。输入是传感器码值（`linear = false`），输出是线性、RAW、场景参考（`linear = true`）。已经是线性 RAW 时不写输出。CFA、尺寸、黑白电平和相机矩阵原样带走。本节点不插值，也不转到 Rec.2020。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `mode` | `as_shot` \| `temperature` \| `gains` | — | `as_shot` |
| `temperature` | f32，开尔文 | 2000–12000 | 仅 `temperature` |
| `tint` | f32 | −100–100 | 0 |
| `gains` | `[f32; 3]` | 每个 0.05–20 | 仅 `gains` |

像素操作只有一种：**raw channel scaling**。每个感光点乘以它的 CFA 颜色对应的系数：

```text
linear = (code - black_level) / (white_level - black_level) * gain[cfa(x, y)]
```

绿色系数归一为 1 的两组增益，红色、蓝色可以大于 1。这一步必须发生在插值之前：RCD 用的是通道比值，未平衡的通道会把色差方向判错。

三种模式只决定 `gain`，不改变上面的缩放：

`as_shot`：直接使用马赛克上的 `as_shot_gains`（文件里的 as-shot channel multipliers）。

`gains`：直接使用参数，不强制 G = 1。

`temperature`：用 **von Kries** 在相机 RGB 里做对角适应。光源色度取 **Planckian locus（CIE 1931 xy）** 上该色温的点；`tint` 沿垂直于轨迹的方向在 **CIE 1960 uv** 里偏移，±100 对应 0.05。该 xy 经 `camera_to_xyz_d50` 的逆矩阵映到相机 RGB，再与 D65 在同一矩阵下的相机 RGB 相除，得到三个系数，然后把 G 归一为 1。矩阵缺失或不可逆时返回 `MissingCameraMatrix`。这里不用 Bradford：Bradford 是后面 D50 到 D65 的适应，不是 RAW 通道缩放。

### 9.3 Demosaic

输入 `image` 是解码器（LibRaw）已经插值完的线性相机 RGB；这里没有马赛克数据，也没有 5×5 邻域可用来重建高光。因此本节点是**透传**：不改像素、不裁切、不写 `space`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `method` | `bilinear` \| `rcd` | — | `bilinear`（已无效果） |
| `highlight` | `clip` \| `reconstruct` | — | `clip`（已无效果） |

两个字段只为让旧文档能加载而保留，不再出现在节点界面里，也不影响输出。

**不要在这里裁到 `[0, 1]`。** 相机的白平衡系数会把亮部推出 1，这是场景参考数据的正常状态；在乘工作空间矩阵之前裁掉某个通道，会把载有色相的通道比例破坏掉。实测：一台 S5M2 的相机矩阵下，亮黄 `(1.40, 1.05, 0.28)` 在裁切后输出 `(230, 237, 0)`（红低于绿、发青），不裁切则输出 `(255, 231, 0)`（保持暖黄）。裁切只属于显示变换。

### 9.3a Input Color Profile

输入 `image` 接受线性相机 RGB。输出 `LinearRgb`，`space = LinearRec2020`。只出现在 RAW 文档里，接在 Demosaic 之后、影调节点之前。光栅图已经在 Input 里进入工作空间，文档里没有这个节点。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `source` | `embedded` \| `imported` | — | `embedded` |
| `name` | 字符串 | 显示用，最长 64 | 相机型号，或导入文件的名字 |

`embedded`：默认配置，和 LibRaw 把 `output_color` 设为 Rec.2020 时用的是同一组矩阵。`rgb_cam` 是相机 RGB → 线性 sRGB，再乘 LibRaw 的 `rec2020_rgb`（不是管线里另外那张 sRGB → Rec.2020）。矩阵缺失或接近 0 时，把 `rgb_cam` 当成单位矩阵。这个矩阵写在节点参数里，之后的参数提交不能把它改掉。高光不在这里裁到 16 位整数，所以超出范围的高光会比 LibRaw 直接出图多留一点。

`imported`：使用用户导入的配置。导入文件不落盘，解析后的 3×3 写进节点参数。两种文件：

- JSON：`{ "name": "可选", "camera_to_xyz_d50": [9 个数] }`。9 个数按行主序，是线性相机 RGB → XYZ D50，也就是 DNG `ColorMatrix` 标签的逆。
- ICC：只接受 RGB、PCS 为 XYZ 的矩阵配置（`mntr`、`scnr` 或 `spac`）。读取 `rXYZ`、`gXYZ`、`bXYZ`，当作相机 RGB → XYZ D50。不使用配置里的阶调曲线；输入缓冲已经是线性的。

XYZ D50 再经 Bradford 到 XYZ D65，然后按 ITU-R BT.2020 进入线性 Rec.2020。这两步合成进同一个 3×3。文件超过 1 MiB、不是上述两种、或矩阵里有非有限值、绝对值大于 16，导入失败，节点参数保持原样。

显示名只给界面。路径分隔符和控制字符会被去掉，名字不参与任何文件路径。

节点关闭时相机 RGB 原样通过。后面的节点仍会运行，预览会把这些数当成工作空间，颜色不正确。

### 9.4 Rotate

输入、输出 `LinearRgb`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `degrees` | f32 | −180–180，校验时归一化到该区间 | 0 |
| `expand` | bool | — | true |
| `inscribed` | bool | — | true |

节点不改像素。它把 `degrees` 加进帧上的 `Transform`，并按参数写下 `ROI`。`ROI` 相对旋转后的扩展画布，范围 0–1。

`inscribed` 打开时，`ROI` 是能放进旋转后画面、并且宽高比与旋转前相同的最大轴对齐矩形。`expand` 打开且未勾选内接时，`ROI` 是整块扩展画布。两者都关时，`ROI` 是画布中央、与旋转前同尺寸的窗口。

裁切再把参数叠进当前 `ROI`，同样不改像素。显示时才按 `Transform` 和 `ROI` 取景。

若将来支持在去马赛克之前旋转，仅 90 的倍数合法，并且必须按 CFA 相位表更新原点。首版旋转只出现在线性 RGB 上，不做这件事。

### 9.5 Crop

输入、输出 `LinearRgb`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `x`,`y`,`w`,`h` | f32 | 各自在 0–1，`x+w<=1`，`y+h<=1`，`w,h>0` | 整幅 |

坐标系是当前 `ROI` 里的归一化坐标，原点在左上。节点把矩形叠进已有 `ROI`，像素不变：

```text
x' = x0 + x * w0
y' = y0 + y * h0
w' = w * w0
h' = h * h0
```

### 9.6 Exposure

输入、输出 `LinearRgb`。点运算。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `ev` | f32 | −5–5 | 0 |
| `black` | f32 | −0.1–0.1 | 0 |

```text
out = in * 2^ev - black
```

允许负值和大于 1 的值进入后续节点。Sigmoid 或 `display_from_linear` 再负责显示域裁切。

### 9.7 LocalContrast

输入、输出 `LinearRgb`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `strength` | f32 | 0–2 | 0 |
| `radius` | f32 | 0.005–0.2 | 0.04 |
| `eps` | f32 | 1e-3–1 | 0.02 |

`radius` 是相对短边的比例。像素半径 `r = max(1, round(radius * min(width, height)))`。`eps` 是 log2 亮度的方差：比它小的起伏当成细节，比它大的边缘留在基底里。

亮度用 Rec.2020：

```text
Y = 0.2627 R + 0.6780 G + 0.0593 B
L = log2(max(Y, 1e-6))
base = guided_filter(L, r, eps)
detail = L - base
L' = L + strength * detail
scale = 2^(L' - L)
out = in * scale
```

`strength = 0` 时实现直接返回输入，不跑滤波。大于 0 时在原亮度上叠加细节，1 大约把抽出来的细节再加一层。引导滤波的引导图就是 `L` 本身。色相由 RGB 同比缩放保持。

### 9.8 Dehaze

暗通道先验（He 的 dark channel prior）。RAW 上接在 Demosaic 之后、Input Color Profile 之前：先验用的是 `min(R,G,B)`，工作空间矩阵会把三个通道混在一起，暗通道就不再成立。光栅图没有输入色彩配置，去雾仍接在局部对比之后。输入和输出与进入时的缓冲同一种，RAW 上是线性相机 RGB。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `strength` | f32 | 0–1 | 0 |
| `omega` | f32 | 0.5–1 | 0.95 |
| `t_min` | f32 | 0–0.5 | 0.1 |
| `radius` | f32 | 0–0.08 | 0.02 |
| `airlight` | `auto` 或 `[f32;3]` | 分量 0–4 | `auto` |

```text
dark(p) = min_{q in patch(p)} min(R,G,B)(q)
```

`auto` 大气光：取 `dark` 最大的 0.1% 像素，在这些像素里取原图最大通道的平均，得到 `A`。手填值直接作为 `A`。

```text
t = 1 - omega * dark / max(A_luma, 1e-4)
t = guided_filter(t, guide = (R+G+B) / 3, window = radius, coeff = radius, eps = 1e-2)
t = max(t, t_min)
J = (in - A) / t + A
out = in + strength * (J - in)
```

`strength = 0` 时返回输入。`A` 的亮度用与局部对比相同的系数。

透射率低于下限的像素被钳成同一个值。半径是暗通道的邻域。细纹和这个滑动窗口的周期一对拍，透射率上就会留下一块慢变化的增益，预览里就是摩尔纹。雾本身是慢的，所以暗通道和引导图都做在低频上，半径大约三分之一作为低通的 sigma；原图减去低频得到的细纹原样加回去，不乘这张增益。半径为 0 时没有邻域，每个像素仍用自己的 `min(R,G,B)`。

### 9.9 Velvia

输入、输出 `LinearRgb`。点运算。把颜色从亮度轴上推开，饱和度越低推得越多，已经很纯的颜色少动，用来做出反转片那种浓色，同时少把高饱和区域推出色域。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `strength` | f32 | 0–1 | 0 |
| `bias` | f32 | 0.25–4 | 1 |

`strength = 0` 时返回输入。

亮度用 Rec.2020 系数。饱和度用相对最大值的色差：

```text
Y  = 0.2627 R + 0.6780 G + 0.0593 B
mx = max(R, G, B)
mn = min(R, G, B)
sat = (mx - mn) / max(mx, 1e-6)
w = strength * (1 - sat)^bias
out = Y + (in - Y) * (1 + w)
```

`sat = 0`（灰色）时权重是 `strength`，灰色本身没有色差，结果仍是灰色。`sat = 1` 时权重是 0，该像素不变。`bias` 越大，效果越集中在低饱和颜色上。

`mx < 1e-6` 的像素直接返回输入。线性域允许负值和大于 1 的值，这里不裁切。

同一条支路上可以放多个 Velvia。它只接受 `LinearRgb`，所以位于 Sigmoid 之前。

### 9.10 Denoise

输入、输出 `LinearRgb`。邻域运算，光环为像素半径 `r`。亮度和色度分开平滑：亮度少动，保住边缘；色度多动，去掉彩色噪点。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `luma` | f32 | 0–1 | 0 |
| `chroma` | f32 | 0–1 | 0 |
| `radius` | f32 | 0.002–0.05 | 0.008 |
| `eps` | f32 | 1e-4–1e-1 | 1e-3 |

`luma` 和 `chroma` 都是 0 时返回输入。`radius` 是相对短边的比例，`r = max(1, round(radius * min(width, height)))`。

```text
Y = 0.2627 R + 0.6780 G + 0.0593 B
L = log2(max(Y, 1e-6))
base = guided_filter(L, r, eps)
L' = base + (L - base) * (1 - luma)
Y' = 2^L'

c = in / max(Y, 1e-6)
c_smooth = guided_filter(c, r, eps)
c' = c + (c_smooth - c) * chroma
out = Y' * c'
```

`c` 是 RGB 相对亮度的比例，灰色为 `(1, 1, 1)`。引导滤波对三个色度通道分别做，引导图用 `L`。`Y < 1e-6` 的像素返回输入。边界用反射延拓。线性域不裁切。

同一条支路上可以放多个 Denoise，并且只接受 `LinearRgb`。它不规定必须在 Sharpen 之前；需要先去噪再锐化时，把 Denoise 接在 Sharpen 上游。

### 9.11 Sharpen

输入、输出 `LinearRgb`。邻域运算，光环为 `3 * sigma`。在 log 亮度上做反锐化，再按比例乘回 RGB，色相保持不变。半径比局部对比小，用来抬边缘，而不是拉开大块明暗。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `amount` | f32 | 0–3 | 0 |
| `radius` | f32 | 0.001–0.02 | 0.003 |
| `threshold` | f32 | 0–0.2 | 0 |

`amount = 0` 时返回输入。`radius` 是相对短边的比例。

```text
sigma = max(0.5, radius * min(width, height))
Y = 0.2627 R + 0.6780 G + 0.0593 B
L = log2(max(Y, 1e-6))
blur = gaussian(L, sigma)
detail = L - blur
```

`abs(detail) < threshold` 的像素把 `detail` 置 0，用来压住平坦区域的噪声。然后：

```text
L' = L + amount * detail
scale = 2^(L' - L)
out = in * scale
```

高斯核截断在 `3 * sigma`。边界用反射延拓。线性域不裁切。同一条支路上可以放多个 Sharpen，并且只接受 `LinearRgb`。

### 9.12 Sigmoid

输入 `LinearRgb`，输出 `DisplayRgb`。曲线是 darktable sigmoid 的广义对数逻辑（胶片加相纸）。中灰固定，对比度只改变这一点的斜率。显示白是场景趋近无穷时的渐近线，场景线性 1 还在肩上。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `contrast` | f32 | 0.1–10 | 1.5 |
| `skew` | f32 | −1–1 | 0 |
| `white` | f32 | 20–1600，百分比 | 100 |
| `black` | f32 | 0–15，百分比 | 0.0152 |

三条锚：场景 0 落到 `black / 100`，场景中灰 `0.1845` 落到显示 `0.1845`，场景无穷大落到 `white / 100`。`skew = 0` 时 `paper_power = 1`，正偏移把肩部让给高光。`contrast` 先按无偏移、显示范围归一的斜率算出，再折进 `film_power`，所以改偏移不会挪动中灰。

先把工作空间转到线性 sRGB，负值拉回平均亮度，再对 sRGB 亮度做这条曲线，RGB 乘以 `Y'/Y`。亮度低于 `1e-9` 时三个通道都取曲线在 0 处的值。关闭本节点时，`image` 输出仍是输入的 `LinearRgb`，不在这里做显示转换。显示转换只发生在启用的 Sigmoid，或 Export 发现输入仍是 `LinearRgb` 时插入的那一次 `display_from_linear`。

### 9.13 Lut

输入、输出 `DisplayRgb`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `lut` | `ResourceRef` | 必须指向 `ResourceKind::Lut` | 无；启用时必填 |
| `strength` | f32 | 0–1 | 1 |
| `encoding` | `srgb` \| `adobe-rgb` \| `gamma-rec709` \| `linear-rec709` \| `linear-rec2020` \| `linear-prophoto` | — | `srgb` |

`.cube` 的解析在 `src/grade/cube.rs`，由资源库调用，不由节点读路径。`LUT_3D_SIZE` 取 2 到 256 的整数，原文不超过 256 MiB。`DOMAIN_MIN` / `DOMAIN_MAX` 默认 0 和 1。数据按 Adobe Cube 顺序：R 变化最快。解析结果是 `Lut3d { size, domain_min, domain_max, table: Vec<f32> }`，以 `Arc` 放在资源内存缓存里。节点只拿这个 `Arc`。

`encoding` 是查表时的应用色彩空间，不是 cube 文件里的标记。先把缓冲变到这个空间（原色加传递曲线），插值，再变回去，所以 Export 收到的仍是原来的缓冲。`srgb` 用 sRGB 原色和 sRGB OETF。`adobe-rgb` 用 Adobe RGB（1998）原色和 γ 2.19921875。`gamma-rec709` 用 Rec.709 原色和 BT.709 OETF。三个 `linear-*` 只换原色、不做伽马；ProPhoto 经 Bradford 从 D65 转到 D50。影调之后的缓冲是线性 sRGB，工作空间是线性 Rec.2020。

四面体插值：

```text
p = (c - domain_min) / (domain_max - domain_min) * (size - 1)
```

`p` 裁到 `[0, size-1]`。落在格子里的小数部分按大小分成六个四面体，只用其中四个角点。`000` 到 `111` 那条对角线被相邻四面体共用，渐变不会在这条对角线上折出三线性那种色带。`strength < 1` 时 `out = in + strength * (lut(in) - in)`。`strength = 0` 返回输入，不解析 LUT。

找不到引用、种类不对或表尺寸非法时，执行返回 `Resource(...)`。节点不打开文件路径。

### 9.13a ColorZones

按色相把画面分成若干段，分别调它们的明度、彩度和色相。输入、输出都是线性 Rec.2020。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `lightness` | 控制点数组 `[[hue, value], …]` | hue 0–360，value −100–100 | 两端 `0` |
| `chroma` | 同上 | value −100–100 | 两端 `0` |
| `hue` | 同上 | value −180–180（度） | 两端 `0` |
| `mix` | f32 | 0–100（%） | 100 |
| `process_mode` | `smooth` \| `strong` | — | `smooth` |
| `interpolation` | `centripetal_spline` \| `cubic_spline` \| `linear` | — | `centripetal_spline` |

控制点的 hue 先排序、夹到 `[0, 360]`；两端不足时用首尾点向外延伸，所以曲线在整圈色相上都有定义。`hue = 0` 与 `360` 是同一条线，保存时保留两个端点，执行时按环形取值。

色相取自 **HSV**：线性 Rec.2020 先转线性 sRGB，按 `V = max`、`S = (max-min)/max`、标准色相公式取 `(h, s, v)`。色相轮与界面画的渐变一一对应（红 0、黄 60、青 180、蓝 240），`V` 不设上限，所以相机高光超过 1 也能原样往返。三个曲线各自把 `h` 映射成一个调整量：

```text
V' = V * (1 + lightness(h) / 100)
S' = clamp(S * (1 + chroma(h) / 100), 0, 1)
h' = h + hue(h)
```

`V'` 不小于 0，`h'` 取模到 `[0, 360)`。转回 HSV → 线性 sRGB → 线性 Rec.2020。没有颜色（`V ≈ 0`）的像素原样返回。`process_mode = strong` 时把三个调整量按符号平方（`x * |x|`），选择更"硬"，过渡更窄；`smooth` 直接用插值结果。

`interpolation` 决定控制点之间怎么连：`linear` 是折线；`cubic_spline` 是普通三次样条；`centripetal_spline` 用向心 Catmull-Rom（α = 0.5），控制点不等距时不会打结。三者都只在相邻两点之间插值，不改变端点值。

`mix = 0` 直接返回输入。否则 `out = in + mix/100 * (adjusted - in)`，在**线性**空间混合，避免在编码空间混出灰边。节点不改缓冲的 `space`，也不读掩码以外的输入。

### 9.14 Blend

Blend 把多张已经算好的图像按层合成一张。它不负责把不同尺寸配准，也不负责把线性光和显示光变成同一种数。对不齐就拒绝，而不是缩放或自动插入 Sigmoid。

汇合要同时满足：

- `base` 和每一个 `over_i` 都是图像输入，数据类型相同，并且是 `LinearRgb` 或 `DisplayRgb`
- 每一路的宽和高相同。像素按左上角对齐，没有偏移参数
- `layers` 的长度就是 `over_i` 的个数。`mask_i` 不在 `inputs` 里时，这一层没有掩码。`base` 不在时，合成从第一路存在的 `over_i` 开始；某一路 `over_i` 不在时跳过这一层。一路图像都没有时不写输出

一路被裁切或旋转过、另一路没有时，宽高不同，执行返回 `SizeMismatch`。两路都要接过同一次几何变换，掩码也要接那次变换的 `mask` 输出。上游节点被关闭只是让那一路变成处理前的图，这一层仍然参与合成。不想要这一层时把 `opacity` 设为 0，或不要连接它。

调色分叉的汇合放在 Sigmoid 之前，各路保持 `LinearRgb`，Blend 之后再接一个 Sigmoid。只给其中一路接 Sigmoid 时，各路类型不再相同，校验直接拒绝这条连线。显示域里的汇合则要求每一路都已经是 `DisplayRgb`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `layers` | 数组 | 至少 1 项。有几项就生成几个 `over_i` 和 `mask_i` | 一项 `normal` / `1` |
| `layers[].mode` | 见下表 | 线性域只能是 `normal`、`multiply`、`add` | `normal` |
| `layers[].opacity` | f32 | 0–1 | 1 |

合成从 `base` 开始，记为 `B`。按 `i = 0 .. n-1` 读取输入 `over_i` 和可选的 `mask_i`，每层的结果成为下一层的 `B`：

```text
blended = mode(B, S)        // S 来自 over_i
w = opacity * mask          // mask_i 没有边时 mask = 1
B = B * (1 - w) + blended * w
```

该层 `opacity = 0` 时不读 `over_i`。`w` 逐像素。通道各自计算。

| mode | 可用的域 | 公式 |
| --- | --- | --- |
| `normal` | 线性、显示 | `S` |
| `multiply` | 线性、显示 | `B * S` |
| `add` | 线性、显示 | `B + S` |
| `screen` | 仅显示 | `1 - (1 - B) * (1 - S)` |
| `overlay` | 仅显示 | `B ≤ 0.5` 时 `2*B*S`，否则 `1 - 2*(1-B)*(1-S)` |
| `difference` | 仅显示 | `abs(B - S)` |

`screen`、`overlay`、`difference` 把 0 和 1 当成黑和白。场景线性可以远大于 1，也可以是负数，所以这三种模式在 `LinearRgb` 上校验失败。线性域不裁切。显示域在全部层算完后把结果裁到 `[0, 1]`。

关闭整个 Blend 节点时，`image` 输出等于 `base`，叠层和掩码都不计算。

### 9.15 MaskDraw

没有图像处理。`reference` 输入的类型可以是 `LinearRgb`、`DisplayRgb` 或 `Mask`，只读取宽高，不读像素。输出 `mask`。`reference` 不在 `inputs` 里时，这个节点没有输入边，执行器不会调用它。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `shape` | `gradient` \| `ellipse` | — | `gradient` |
| `feather` | f32 | 0.001–1 | 0.05 |
| `cx`,`cy` | f32 | 0–1，相对宽高 | 0.5, 0.5 |
| `angle` | f32 | −180–180，度 | 0 |
| `rx`,`ry` | f32 | 0.01–2，相对短边 | 0.4, 0.25 |

坐标原点在左上，`x` 向右，`y` 向下。`feather` 是过渡带的相对宽度，过渡用 smoothstep，不产生 0/1 的硬边。

线性渐变沿 `angle` 方向。以中心为 0.5，在 `feather` 宽度内从 0 过渡到 1，一侧为 0，另一侧为 1。

椭圆用归一化半径 `ρ`：

```text
ρ = sqrt(((x - cx) / rx)^2 + ((y - cy) / ry)^2)
mask = smoothstep(1, 1 - feather, ρ)
```

`ρ < 1 - feather` 时为 1，`ρ > 1` 时为 0，中间连续。

### 9.16 MaskCreator

`image` 的类型是 `LinearRgb` 或 `DisplayRgb`。输出 `mask`。`image` 不在 `inputs` 里时不写输出。按像素的一个通道做软区间，区间外是 0，区间内是 1，边缘用羽化过渡。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `channel` | `luma` \| `chroma` \| `hue` | — | `luma` |
| `lower`,`upper` | f32 | 见下 | 0, 1 |
| `feather_lo`,`feather_hi` | f32 | 0–1 | 0.05, 0.05 |

`luma` 用 Rec.2020 亮度，再除以 `1 + Y` 压到 0–1，这样场景线性的高光也落在参数范围内。`chroma` 是 `(max - min) / max(max, 1e-6)`。`hue` 是 0–1 的色相环；跨 0 的区间把环展开后再套同一段 smoothstep。

```text
t0 = lower - feather_lo
t1 = lower
t2 = upper
t3 = upper + feather_hi
mask = smoothstep(t0, t1, v) * (1 - smoothstep(t2, t3, v))
```

`lower` / `upper` 对 `luma` 和 `chroma` 在 0–1，对 `hue` 也在 0–1。结果保持连续，不量化成二值。

### 9.17 MaskCombine

输出一路 `mask`。参数 `count` 决定输入 `in_0` … `in_{count-1}`，类型都是 `Mask`。`inputs` 里缺的那一路不参与运算。一路都没有时，这个节点没有输入边，执行器不会调用它。在场的各路必须等大。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `count` | u8 | 1–4 | 2 |
| `op` | `union` \| `intersection` \| `difference` \| `invert` | — | `union` |
| `feather` | f32 | 0–0.1，相对短边 | 0 |
| `opacity` | f32 | 0–1 | 1 |
| `contrast` | f32 | 0.25–4 | 1 |

`invert` 只用第一路。其余操作按顺序归约：

```text
union        = 1 - (1 - a) * (1 - b)
intersection = a * b
difference   = abs(a - b)
```

然后羽化、不透明度和对比：

```text
blur = feather > 0 时对掩码做高斯，sigma = feather * min(width, height)
m = blur * opacity
m = clamp(0.5 + contrast * (m - 0.5), 0, 1)
```

`contrast = 1` 不改变形状。大于 1 把中间值推向两端，但仍然是连续函数，不是阈值。

### 9.18 Export

输入 `DisplayRgb`，无像素输出。`image` 有值时产生 `EncodedImage`。没有这条边，或上游没写出值时，本节点不编码，`execute` 返回 `Inactive`。

| 字段 | 类型 | 范围 | 默认 |
| --- | --- | --- | --- |
| `format` | `jpeg` \| `webp` \| `png` \| `tiff` | — | `jpeg` |
| `quality` | u8 | 1–100 | 90 |
| `long_edge` | u32 | 0–16384 | 0 |
| `bit_depth` | 8 \| 16 | png/tiff 可用 16 | 8 |

`long_edge = 0` 表示保持当前尺寸。大于 0 时在 OETF 之前用面积平均缩小。JPEG/WebP 固定 8 位。PNG/TIFF 的 16 位走线性量化：先 OETF 再乘以 65535。

sRGB OETF：

```text
若 L <= 0.0031308：  c = 12.92 * L
否则：              c = 1.055 * L^(1/2.4) - 0.055
```

预览目标忽略本节点的 `format` 和 `quality`，固定 WebP、质量 80、长边为 `ExecTarget::Preview.max_edge`。预览仍然遵守几何和调色，这样屏幕上的构图和导出一致。

```rust
pub struct EncodedImage {
    pub format: ExportFormat,
    pub bytes: Vec<u8>,
    pub size: ImageSize,
    pub warnings: Vec<ExecWarning>,
}
```

编码通过 `EncodeSink`，默认实现可用 `image` crate，宿主可以换成自己的编码器。

## 10. ResourceManager

调色里有两类体积差很多的数据。

| 种类 | 例子 | 放哪 |
| --- | --- | --- |
| 小型参数 | 曝光、裁切、Sigmoid 对比度 | 照片的 `GradeDocument` JSON |
| 共享只读资源 | LUT 表、预设正文 | ResourceManager，文档只存 `ResourceRef` |
| 派生渲染 | 预览图、去马赛克检查点 | 第 11 节的渲染缓存，可整块丢掉 |

资源一旦写入就不可改。改一张 LUT 或一份预设会得到新的内容哈希，旧引用继续指向旧字节。显示名可以改，它写在元数据里，不参与哈希。

### 10.1 标识

```rust
pub struct ResourceId {
    pub kind: ResourceKind,
    pub hash: [u8; 32], // 规范化载荷的 SHA-256
}

pub enum ResourceKind {
    Lut,     // 原始 .cube 字节
    Preset,  // 一份不含源文件绑定的 GradeDocument
}

pub enum ResourceRef {
    /// 钉死某一版字节。照片默认用这个。
    Pinned(ResourceId),
    /// 稳定别名，执行时再解析到当前 Pinned 目标。
    Alias(AliasId),
}

pub struct AliasId(pub String); // 1–64 字节，`[a-z0-9_-]`
```

`ResourceId` 序列化成 `lut:<hex>` 或 `preset:<hex>`。别名序列化成 `alias:<kind>:<alias_id>`。十六进制用小写，长度固定 64。

照片文档里的 LUT 节点使用 `ResourceRef`。预设资源内部的 LUT 节点只允许 `Pinned`，避免预设再套预设。

### 10.2 载荷边界

| 种类 | 载荷 | 上限 | 禁止 |
| --- | --- | --- | --- |
| `Lut` | `.cube` 原文 | 256 MiB；边长 2–256 | 把 `table` 浮点数组写进任何 JSON |
| `Preset` | 规范化后的文档 JSON | 64 KiB | 内嵌 LUT 原文、照片路径、渲染缓存键 |

预设 JSON 的节点参数和照片文档用同一套 schema。它引用的 LUT 必须已经是 `Pinned(ResourceId)`。预设不能再引用另一份预设，校验器拒绝 `Preset` 载荷里出现 `alias:preset:` 或嵌套预设字段。

一张 65³ LUT 的解析表约 3.2 MiB（`65³ × 3 × 4`）。一千张照片共用一张 LUT 时，磁盘上只留一份原文，内存里只留一份 `Arc<Lut3d>`。

### 10.3 存储

库定义 trait，宿主提供目录。布局：

```text
resources/
  blobs/{kind}/{hh}/{hash}          # 载荷，hh 为哈希前一字节
  meta/{kind}/{hash}.json           # 显示名、字节数、媒体类型、导入时间
  aliases/{kind}/{alias_id}.json    # { "target": "<hash>" }
```

元数据示例：

```json
{
  "kind": "lut",
  "hash": "…64 hex…",
  "byte_size": 184320,
  "display_name": "Film.cube",
  "media_type": "text/plain"
}
```

`display_name` 只给界面看。重命名覆盖这个文件，不移动 blob，不改变 `ResourceId`。

写入步骤：

1. 校验种类和大小。
2. 对 LUT 先解析，确认边长和域合法，再对原文做哈希。解析失败不落盘。
3. 对预设先 `validate`，确认其中的 LUT 引用都是已存在的 `Pinned`，再对规范化 JSON 做哈希。
4. blob 写到临时文件，再原子改名到 `blobs/.../{hash}`。已存在同一哈希则跳过 blob，只补元数据。
5. 需要别名时，原子更新 `aliases/...`。

```rust
pub trait ResourceStore: Send + Sync {
    fn put_blob(&self, id: &ResourceId, bytes: &[u8]) -> Result<(), GradeError>;
    fn get_blob(&self, id: &ResourceId) -> Result<Option<Vec<u8>>, GradeError>;
    fn put_meta(&self, id: &ResourceId, meta: &ResourceMeta) -> Result<(), GradeError>;
    fn get_meta(&self, id: &ResourceId) -> Result<Option<ResourceMeta>, GradeError>;
    fn put_alias(&self, kind: ResourceKind, alias: &AliasId, target: &ResourceId) -> Result<(), GradeError>;
    fn get_alias(&self, kind: ResourceKind, alias: &AliasId) -> Result<Option<ResourceId>, GradeError>;
    fn list_ids(&self, kind: ResourceKind) -> Result<Vec<ResourceId>, GradeError>;
}
```

### 10.4 内存缓存

```rust
pub struct ResourceManager {
    store: Arc<dyn ResourceStore>,
    parsed: Mutex<LruCache<ResourceId, Arc<ParsedResource>>>,
    inflight: Mutex<HashMap<ResourceId, WaitList>>,
}

pub enum ParsedResource {
    Lut(Arc<Lut3d>),
    Preset(Arc<GradeDocument>),
}
```

`open_lut` / `open_preset` 的顺序：

1. 若引用是别名，读 alias 文件得到 `ResourceId`。别名不存在返回 `Resource(DanglingAlias)`。
2. 查 LRU。命中则克隆 `Arc` 返回。
3. 未命中时用 `inflight` 合并并发解析，同一哈希只解析一次。
4. 读 blob，解析，放入 LRU。缓存按解析后的字节计，默认预算 256 MiB。超出时从最久未用的条目删起。删的是缓存条目，调用方已经克隆出去的 `Arc` 继续有效。

执行期间 LUT 节点持有 `Arc<Lut3d>`。缓存淘汰不会拆掉这个 `Arc`。磁盘 blob 不受 LRU 影响。

### 10.5 引用计数与回收

磁盘不做即时引用计数。照片文档、预设 blob 都可能在管理器进程之外被引用。回收是扫描：

```text
live = 空
for 每份照片文档:
    解析其中的 ResourceRef（别名展开成当前 target）
    把 Pinned id 加入 live
    若 id 是 Preset，再把它内部的 LUT id 加入 live
for blobs 与 meta:
    不在 live 中的标记为垃圾
```

扫描由宿主显式调用 `collect_garbage(roots)`，不在每次保存时做。返回值是待删 id 列表，宿主确认后再 `delete`。正在被 `open_*` 读的 id 即使不在 live 里也延后删除：`ResourceManager` 用一个 `pin` 计数（执行开始 +1，结束 −1）挡住这次回收。

别名文件不参与 live 判定。没有任何文档指向的别名可以由宿主另做一次别名扫描。删掉别名不会删 blob。

### 10.6 和文档、执行的关系

照片文档增加一个可选字段：

`GradeDocument.preset` 只允许指向 `Preset`。两种用法：

- **钉死套用**：`preset` 为空，`nodes` 是完整链。套用预设时，宿主把预设节点拷进 `nodes`，LUT 仍是原来的 `Pinned` id。预设正文不进入照片 JSON。之后改照片参数不影响预设。
- **链接套用**：`preset` 为 `Alias` 或 `Pinned`，`nodes` 只存覆盖项（同一 `id` 的节点替换预设中的同 id 节点）。执行前先 `open_preset`，再按 id 覆盖。覆盖项里的新 LUT 同样只写 `ResourceRef`。

链接套用时，缓存键使用展开后的完整规范化 JSON（含解析出来的预设哈希和 LUT 哈希），而不是只哈希照片上那几行覆盖。这样预设别名指向新版本后，预览会失效。

`validate(doc)` 只检查引用的语法和「LUT 节点的 kind 是 Lut、preset 字段的 kind 是 Preset」。不读磁盘。

`validate_resolved(doc, resources)` 额外要求目标存在、预设内部引用存在、预设没有再套预设。保存照片文档时跑 `validate`。执行时跑 `validate_resolved`。导入预设时对预设载荷跑 `validate_resolved`。

```rust
impl ResourceManager {
    pub fn import_lut(&self, display_name: &str, cube: &[u8]) -> Result<ResourceId, GradeError>;
    pub fn import_preset(&self, display_name: &str, doc: &GradeDocument) -> Result<ResourceId, GradeError>;
    pub fn open_lut(&self, reference: &ResourceRef) -> Result<Arc<Lut3d>, GradeError>;
    pub fn open_preset(&self, reference: &ResourceRef) -> Result<Arc<GradeDocument>, GradeError>;
    pub fn pin(&self, id: &ResourceId);
    pub fn unpin(&self, id: &ResourceId);
    pub fn collect_garbage(&self, roots: &[GradeDocument]) -> Result<Vec<ResourceId>, GradeError>;
}
```

`execute` 在跑 LUT 节点前 `open_lut`。链接了预设时，先展开成一份临时 `GradeDocument` 再走原来的主循环。临时文档不写回磁盘。

错误：

```rust
pub enum ResourceError {
    NotFound(ResourceId),
    DanglingAlias(AliasId),
    WrongKind { expected: ResourceKind, actual: ResourceKind },
    TooLarge { limit: u64, actual: u64 },
    InvalidLut(String),
    InvalidPreset(String),
    PresetCycle,
}
```

### 10.7 不放进 ResourceManager 的数据

去马赛克检查点和预览图是某张照片、某一版文档的派生物，键里含 `source_id`，丢了可以重算。它们走 `CheckpointStore` 和渲染缓存。放进资源库会让回收扫描把渲染垃圾当成用户资产。

相机矩阵只有 9 个 `f32`。嵌入矩阵和导入配置解析后的矩阵都留在 Input Color Profile 的参数里，不建资源，也不保存导入文件的原文。

## 11. 缓存

```rust
pub struct CacheKey {
    pub source_id: [u8; 32],
    pub document_hash: [u8; 32],
    pub target: TargetKey,
}

pub enum TargetKey {
    Preview { max_edge: u32 },
    Export,
}

pub trait CheckpointStore: Send + Sync {
    fn load(&self, key: &CheckpointKey) -> Result<Option<LinearBuffer>, GradeError>;
    fn store(&self, key: &CheckpointKey, buffer: &LinearBuffer) -> Result<(), GradeError>;
}
```

宿主把 `EncodedImage` 按 `CacheKey` 存下来。文档任一参数变化会改变 `document_hash`，旧预览自然失效。源文件变化由宿主更换 `source_id`（建议含路径、大小、mtime 或内容哈希）。

检查点缓冲用未压缩的小端 `f32` 加上一个 32 字节头（宽、高、space、magic）。体积大，只保留最近一次去马赛克检查点即可。

## 12. 并发与取消

- `GradeDocument` 在一次 `execute` 期间只读，可跨预览共享。每个输出上的 `Value` 由产生它的节点独占写入，之后只通过 `Arc` 共享。
- 同时进行的预览和全尺寸渲染个数占用宿主的 darktable 并发名额。
- 同一次打开的新请求取消正在跑的那次：宿主在节点之间置上 `CancelFlag`。小预览和全尺寸互不取消。改节点会取消正在跑的小预览、改节点和全尺寸。新的全尺寸会取消正在跑的改节点和上一次全尺寸。被取消的那次不写预览缓存。
- 节点之间必查取消。引导滤波和去马赛克每处理 32 行查一次。
- 取消返回 `GradeError::Cancelled`，不写检查点，不写导出缓存。

执行器按上下游逐个节点跑。每个节点内部按行切到调色线程池；点运算、盒式模糊、最小值滤波都这样。线程数是逻辑 CPU 个数。节点之间不并行，因为后一个节点要等前一个的图像。图像高度不足 48 行时仍在调用线程上跑。旋转和裁切只改变换和选区，不进这个池。

内存里只留进入正在改的那个节点的一帧。再改它时从这一帧算起，不重跑上游。改到另一个节点时，换成进入那个节点的帧。其余节点的图像算完即丢。打开照片时每个节点都要重算，不留下检查点。

## 13. 宿主集成

库不知道相册、用户和 HTTP。宿主侧建议三块，名称按宿主自己的模块来：

```text
host::grade_store       照片文档的读写（小型 JSON）
host::grade_resources   ResourceStore 的目录实现
host::grade_cache       CacheKey 到渲染文件
host::grade_api         校验、预览、导出任务、导入 LUT 与预设
```

最小操作：

| 操作 | 行为 |
| --- | --- |
| 打开照片 | 已有文档则直接用。没有文档则 `resolve`：先用户规则，否则系统预设，并写成链接文档 |
| 保存文档 | `validate` 通过后原子写入。JSON 里只有参数和 `ResourceRef` |
| 导入 LUT | `import_lut`，把返回的 `ResourceId` 写入节点 |
| 导入预设 | `import_preset`。预设载荷里的 LUT 必须已经导入 |
| 预览 | 算 `CacheKey`（链接预设要先展开），命中则返回已有字节，否则 `execute(Preview)` |
| 导出 | 建可取消任务，任务体内 `execute(Export)`，产物新文件，不覆盖原片 |
| 缺省套用 | 只写 `preset` 别名和 `origin`，不复制节点。用户一编辑就展开并改为 `Edited` |
| 手动套用 | 钉死：把预设节点写入 `nodes` 并标 `Edited`。链接：只写 `preset`，`origin` 仍为 `Rule` 或保持 `Edited` 由调用方指定 |
| 回收 | 宿主收集全部照片文档，调用 `collect_garbage`，确认后删除返回的 blob |

文档路径、权限和任务系统都属于宿主。预览请求应防抖后再调用库，防抖间隔由界面决定，库不内置时钟。

## 14. 测试

| 用例 | 断言 |
| --- | --- |
| 文档往返 | serde 后再 `canonical_json` 字节相同，且 JSON 不含浮点表 |
| 缺省规则 | 第一条命中的启用规则胜出；规则为空或全部不中时使用系统预设 |
| 缺省不覆盖 | `origin = Edited` 的文档再次打开时不被规则替换 |
| 非法管线 | 绕回、类型不符、一条输入两条边、两个 Export，均拒绝。未接线的节点合法 |
| 缺线 | 有 `InputDesc` 且没有输入边的节点不被调用，不产生输出。Export 因此未运行时结果为 `Inactive` |
| 部分缺线 | 曝光没接 `mask` 时整幅生效；Blend 缺一路 `over_i` 时其余路仍合成 |
| 掩码混合 | 掩码为 0 的像素等于处理前，为 1 的像素等于处理后，0.5 是两者中点 |
| 掩码连续 | 渐变掩码的相邻像素差有上界，不出现只有 0 和 1 的台阶 |
| 几何掩码 | 旋转 90 后，掩码角点跟着图像角点走 |
| 分叉共享 | 同一上游的两路读到的像素指针计数大于 1，上游缓冲只算一次 |
| Blend | `opacity = 0` 时输出等于底图；`normal` 且 `opacity = 1` 时输出等于上层 |
| 尺寸 | 两路宽高不同的 Blend 返回 `SizeMismatch` |
| 资源去重 | 同一 `.cube` 导入两次得到同一个 `ResourceId`，blob 只有一份 |
| 预设体积 | 预设 JSON 超过 64 KiB，或内部再引用预设，导入失败 |
| 链接展开 | 别名指向新预设后，照片 JSON 不变，展开后的缓存键变化 |
| 回收 | 没有文档引用的 LUT 出现在 `collect_garbage` 结果里；执行 pin 住的 id 不删除 |
| 曝光 +1 EV | 线性缓冲精确乘 2（相对误差 1e-6） |
| 旋转 90 | 角点像素搬到预期位置，无插值 |
| 裁切 | 整数边界与 floor/ceil 规则一致 |
| 恒等 LUT | `strength = 0` 或恒等表时缓冲不变 |
| Sigmoid | 亮度单调；纯灰保持中性 |
| Velvia | `strength = 0` 或纯灰时缓冲不变；提高 `strength` 后彩色像素的色差变大 |
| Denoise | `luma = 0` 且 `chroma = 0` 时缓冲不变；纯色块经过后仍是纯色 |
| Sharpen | `amount = 0` 时缓冲不变；平坦区域在 `threshold` 内不被抬起 |
| 光栅身份链 | 8 位往返误差 ≤ 1 |
| 取消 | 在第二个节点前取消，返回 `Cancelled`，检查点无写入 |
| 类型 | 把 `Mask` 接到 `image` 输入时返回 `TypeMismatch` |

算法测试用程序生成的小图（马赛克用已知纯色块），不依赖相机样张。RCD 与双线性的对比只检查绿色通道边缘能量，不锁死具体像素，避免算法微调把测试钉死。

## 15. 实现顺序

1. `document`、`pipeline`、`preset_policy`、`validate`、`canon`、`error`，加上身份链的光栅 Input / Export。执行顺序、环检测和缺省规则在这一步完成。
2. `resource`：导入、内容哈希、别名、LRU 解析缓存、垃圾扫描。预设可以先只存文档，LUT 解析接在后面。
3. `buffer`、`exec`、Exposure、Rotate、Crop。
4. 预览缩小与 `CacheKey`。
5. WhiteBalance（raw channel scaling）、Demosaic 末尾的 ColorMatrix / Bradford / BT.2020、Sigmoid、`display_from_linear`。
6. `RawSource` 接入。Demosaic 先完成 bilinear，再完成 RCD。白平衡在去马赛克之前接上。
7. LocalContrast、Dehaze、Velvia、Denoise、Sharpen、LUT 节点（只通过 `open_lut` 拿 `Arc`）。
8. 多输入执行、Blend，以及 MaskDraw / MaskCreator / MaskCombine。
9. 检查点与取消。

每一步都可以用第 14 节的测试验收，不需要界面。
