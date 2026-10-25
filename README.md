# Lumeno

Lumeno 是一个自托管的个人文件与照片管理应用。后端使用 Rust/Axum，前端使用 Vue 3、Vite 和 Tailwind CSS；照片、用户数据和应用配置均保存在本机。

## 功能

- 文件浏览、创建目录、上传、下载、复制、移动、重命名和删除
- 支持断点分块上传与后台任务进度查看
- 相册创建、编辑、导入、导出、封面设置和共享权限
- JPEG、PNG、GIF、BMP、WebP、TIFF 等常见图片的缩略图与 EXIF 信息读取
- RAW 格式识别；安装 `darktable-cli` 后支持 RAW 缩略图、预览和导出
- MP4、M4V、MOV、WebM 视频导入、封面和播放。封面需要 `ffmpeg`。浏览器播不了原片时，可按账号偏好整段转码并缓存，播放缓存需要 `ffmpeg` 和 `ffprobe`
- 标签、评分、色标等图片元数据，写入 XMP sidecar 文件
- 按日期、相机、镜头、标签、评分、色标和格式筛选；支持排序、时间轴和虚拟列表
- “那年今日”和每日精选照片视图
- 多用户、管理员、相册读写/删除权限和 WebDAV 共享
- 中文、英文界面，以及明暗主题

## 功能对照

| 功能 | 状态 |
| --- | --- |
| 上传和查看照片、视频 | 照片已有；常见视频可进相册。能播的原片直接播放，不能播的可转码缓存后再播放（封面和转码需要 `ffmpeg`，探测需要 `ffprobe`） |
| 打开应用时自动备份 | 未做 |
| 防止重复导入 | 部分：可找出相似照片，上传时不会自动跳过 |
| 按相册选择备份 | 未做 |
| 下载到本地 | 照片和视频原文件可下载 |
| 多用户 | 已有 |
| 相册与共享相册 | 已有 |
| 可拖动快速滚动条 | 未做 |
| RAW | 已有（需 `darktable-cli`） |
| 元数据（EXIF、地图） | 部分：有拍摄信息，无 GPS 与地图 |
| 按元数据、物体、人脸和语义搜索 | 部分：元数据、人脸和文字语义搜索已有；无物体标签 |
| 用户管理 | 已有 |
| 后台备份 | 未做 |
| 虚拟滚动 | 已有 |
| 实况照片 / Motion Photo | 未做 |
| 自定义存储目录 | 已有 |
| 公开分享 | 未做（可分享给已有账号，或使用 WebDAV） |
| 归档与收藏 | 部分：有评分和色标，无归档或收藏库 |
| 全局地图 | 未做 |
| 伙伴共享 | 未做（可按相册授权，不能共享整个图库） |
| 人脸识别与聚类 | 已有 |
| 回忆（那年今日） | 已有 |
| 离线查看 | 未做 |
| 只读相册 | 已有 |
| 连拍堆叠 | 未做 |
| 标签 | 已有 |
| 文件夹视图 | 已有 |

## 技术栈

- Rust nightly、Tokio、Axum
- Vue 3、Vite、Tailwind CSS、Virtua
- 文件系统作为照片和应用数据存储

## 快速开始

### 前置条件

- Rust nightly 工具链
- Node.js 与 npm
- 可选：`darktable-cli`，用于 RAW 格式的缩略图、预览与导出
- 可选：`exiftool`，用于 RAW 与视频的拍摄信息和时长
- 可选：`ffmpeg` 与 `ffprobe`。`ffmpeg` 抽取视频封面，并在播放缓存里转码；`ffprobe` 读取源编码

首次启动前，前端产物目录和应用数据目录必须存在。以下命令在项目根目录执行：

```bash
rustup toolchain install nightly

cd frontend
npm ci
npm run build
cd ..

mkdir -p dist/data
export BOOTSTRAP_ADMIN_USER=admin
export BOOTSTRAP_ADMIN_PASSWORD='replace-with-a-strong-password'
cargo +nightly run
```

服务监听在 `http://localhost:3000`。首次运行时会创建管理员账户，并在 `dist/data/` 写入用户、会话、设置和 HMAC 密钥等持久化数据。

> 未设置 `BOOTSTRAP_ADMIN_PASSWORD` 时，首次启动会生成随机管理员密码并只打印一次到标准错误。可用 `BOOTSTRAP_ADMIN_USER` 自定义用户名（默认 `admin`）。

## 开发与构建

构建前端静态资源：

```bash
cd frontend
npm run build
```

前端资源会输出到 `dist/resources/`，由 Rust 服务直接提供。启动后端：

```bash
cargo +nightly run
```

前端也提供独立开发服务器命令：

```bash
cd frontend
npm run dev
```

仓库包含 [DockerFile](DockerFile)，用于提供带 Rust nightly 的 Debian Bookworm 开发环境；VS Code Dev Container 配置位于 [.devcontainer](.devcontainer)。

## 配置

| 变量 | 说明 | 默认值 |
| --- | --- | --- |
| `BOOTSTRAP_ADMIN_USER` | 首次启动时创建的管理员用户名 | `admin` |
| `BOOTSTRAP_ADMIN_PASSWORD` | 首次启动时创建的管理员密码 | 未设置则生成随机密码并打印一次 |
| `TRUST_PROXY` | 设为 `1` 或 `true` 后信任反向代理的 `X-Forwarded-Proto`，用于 HTTPS Cookie 判断 | 未启用 |

只有在反向代理会正确覆盖客户端传入的 `X-Forwarded-Proto` 时，才应启用 `TRUST_PROXY`。

## 数据与备份

应用的持久化根目录为 `dist/data/`。部署时应将此目录挂载到持久卷，并定期备份：

- 用户、设置和会话数据
- HMAC 密钥文件
- 用户上传的文件和相册
- 图片 XMP sidecar 文件

不要丢失 HMAC 密钥：重建该密钥会使已有登录会话失效。相册索引和缩略图缓存可以重新生成，但原始图片和 XMP 元数据不可替代。

## 反向代理与安全

- 使用 HTTPS 暴露服务，并仅在可信代理后启用 `TRUST_PROXY`。
- 首次启动请从日志记下管理员密码（或自行设置 `BOOTSTRAP_ADMIN_PASSWORD`），并限制 `dist/data/` 的文件系统访问权限。
- WebDAV 路径为 `/dav/`；应与主站使用同一套认证和 HTTPS 策略。
- 不要将 `.env` 文件、Cloudflare 凭据或 `dist/data/` 内容提交到版本控制。

## 项目结构

```text
src/                 Rust 服务与 API
src/backend/         认证、文件、相册、设置和任务模块
frontend/            Vue 前端源码
locales/             后端国际化文案
dist/resources/      前端构建产物
dist/data/           运行时数据和用户文件
DockerFile           Rust nightly 开发镜像
```

## 验证

```bash
cd frontend && npm run build
cd .. && cargo +nightly test
```
