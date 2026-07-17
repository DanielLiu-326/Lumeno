# Dashboard 实施计划

## 目标

将 Dashboard 作为 Lumeno 用户登录后的默认主页，提供当前用户可访问内容的运营概览、照片回顾、快捷操作、活动任务与管理员专属概览。

Dashboard 使用单一的权限感知聚合 API。前端不分别扫描或组合相册接口，以避免重复读取索引、状态不一致，以及意外暴露无权限的共享相册内容。

## 范围

首版包含：

- Dashboard 导航项与默认主页
- 用户范围内的相册、照片、存储、任务和站点在线时间统计
- 最近照片、最近相册、那年今日与每日精选预览
- 新建相册、上传、导入和跳转相册等快捷操作
- 当前任务进度
- 仅管理员可见的用户、WebDAV 分享和安全设置概览
- 中英文文案、后端测试、前端构建和手动验收

首版不包含：

- 人脸识别、相似照片分析
- 历史统计或时间序列存储
- 新数据库依赖
- 修改完整的“那年今日”或“每日精选”页面
- Dashboard 发起的破坏性操作
- 路由器构建与部署变更

## 后端设计

### 聚合接口

在 `src/backend/album.rs` 的 Dashboard helpers 附近增加序列化响应类型，并实现：

```text
GET /api/dashboard
```

接口使用现有认证、`create_response` 响应格式与本地化错误处理。响应契约应包含：

```json
{
  "summary": {
    "own_album_count": 0,
    "shared_album_count": 0,
    "accessible_photo_count": 0,
    "total_bytes": 0,
    "dated_photo_count": 0,
    "active_task_count": 0,
    "uptime_seconds": 0
  },
  "recent_photos": [],
  "recent_albums": [],
  "on_this_day": { "date": "MM-DD", "count": 0, "photos": [] },
  "daily_picks": { "date": "YYYY-MM-DD", "photos": [] },
  "active_tasks": [],
  "admin": null
}
```

新增 `DashboardPhoto`、`DashboardRecentAlbum`、`DashboardTaskSummary` 与 `DashboardResponse` 类型。照片预览必须包含生成授权缩略图所需的 `path`、`owner`、`album` 与展示名称。

### 站点在线时间

站点在线时间定义为当前 Lumeno 服务进程从启动到当前请求的持续时间，不代表服务器操作系统在线时间，也不跨进程重启累计。

在 `src/backend/common.rs` 的 `AppState` 中增加 `started_at: Instant`，并在 `src/main.rs` 创建应用状态时使用 `Instant::now()` 初始化。Dashboard 请求通过 `started_at.elapsed().as_secs()` 生成 `summary.uptime_seconds`。使用单调时钟可以避免系统时间调整导致在线时间倒退。

前端将秒数格式化为紧凑的本地化文本，例如 `3 天 4 小时` 或 `3d 4h`。不足一分钟时显示“小于 1 分钟”，不在浏览器端自行推测服务启动时间。Dashboard 每次刷新时使用服务端返回值更新；服务重启后该指标从零重新计算。

### 权限与索引

`api_dashboard` 通过 `AppState::get_session_and_root` 验证会话，并复用：

- `collect_accessible_album_sources`
- `load_or_queue_album_index`

聚合范围仅限当前用户拥有或已共享给当前用户的相册。沿用现有相册引用格式：自有相册使用 `dir_name`，共享相册使用 `owner/dir_name`。

在索引条目遍历中计算照片数量与字节数，不重新扫描文件系统。按拍摄日期排序最近照片，并使用路径作为稳定的第二排序键。结果必须在服务端限制数量，例如最多 12 张最近照片和 6 个最近相册。

### 照片回顾与任务

提取或共享 `api_on_this_day`、`api_daily_picks` 所需的纯选择逻辑，保留现有接口的外部行为不变。Dashboard 使用相同的日期规则和稳定每日抽样规则，但仅返回小型预览集合。

复用 `TaskManager::get_task_status_map_for_user`，或在 `src/backend/tasks.rs` 添加一个有序的窄查询接口。仅返回当前用户的 Pending、Running、Paused 任务及其既有进度字段，并限制返回数量。

### 管理员摘要

仅在认证用户具有 `Role::Admin` 时填充 `admin` 字段，内容包括：

- 用户总数
- WebDAV 分享总数
- 可安全展示的全局设置值

管理员的个人 Dashboard 统计仍只计算其可访问相册；不得展示其他用户的照片数量、存储占用或相册内容。

### 路由

在 `src/main.rs` 导入并注册 `api_dashboard`。

## 前端设计

### 导航与状态

在 `frontend/src/components/AppSidebar.vue` 增加 `dashboard` tab，复用 `frontend/src/utils/icons.js` 中已有的 `IconHome`。在双语资源中增加 `nav.dashboard`。

在 `frontend/src/composables/useAlbumApp.js` 增加：

- 响应式 Dashboard 数据
- `dashboardLoading`
- `refreshDashboard()`
- 与既有创建相册、上传、导入、选择相册、刷新任务方法衔接的快捷操作辅助函数

`refreshDashboard()` 必须复用现有 `apiFetch`，不重复认证、语言请求头或错误处理逻辑。

### DashboardPanel

创建 `frontend/src/components/DashboardPanel.vue`，沿用现有应用的紧凑工作台视觉风格。页面应包含：

- 统计区：相册、照片、存储、活动任务、站点在线时间
- 快捷操作区：新建相册、上传、导入、查看相册
- 最近照片网格与最近相册列表
- “那年今日”和每日精选的精简预览
- 带 `progressPercent`、`statusLabel`、`statusTone` 的任务列表
- 仅管理员可见的系统概览
- 加载、空数据与 API 错误状态

共享照片缩略图 URL 仅在需要时附加 `owner` 参数。所有预览点击都应通过现有的相册选择和详情流程处理，不能创建重复工作流。

### 应用集成与国际化

更新 `frontend/src/App.vue`：

1. 将 `activeTab` 的默认值设为 `dashboard`。
2. 在 `activeTabTitle` 中增加 Dashboard 标题。
3. 渲染 `DashboardPanel`。
4. 在登录成功、会话恢复及切换至 Dashboard 时刷新数据。

在 `frontend/src/i18n/en.js` 与 `frontend/src/i18n/zh.js` 增加 `dashboard.*` 分组，覆盖统计标签、站点在线时间及其单位、加载/空态/错误文案、回顾区标题、快捷操作、任务状态及管理员概览。

## 实施顺序

1. 定义响应类型并实现受权限约束的 Dashboard 聚合逻辑。
2. 注册 `GET /api/dashboard`，并补充后端测试。
3. 在 composable 中接入 Dashboard 状态与请求。
4. 创建 DashboardPanel 与导航项。
5. 将 Dashboard 设为默认主页并接入已有操作流。
6. 补充双语文案，执行构建和手动验收。
7. 功能上线后，按需更新 README 的用户功能列表。

## 验证清单

### 后端

- 未认证请求被拒绝。
- 自有和共享相册都能正确统计。
- 不会聚合无权访问或其他用户私有相册。
- 索引不存在或处于重建状态时，接口返回可用的降级结果。
- 空账户、日期边界、每日预览稳定性、结果数量上限均符合预期。
- `uptime_seconds` 单调递增，数值来自当前进程启动时刻，并在服务重启后重新计时。
- 任务过滤正确，普通用户不包含 `admin` 字段，管理员摘要不泄露其他用户媒体统计。

### 前端

```bash
cd frontend
npm run build
```

### Rust

```bash
cargo +nightly test
```

如果当前环境未安装 nightly，记录该限制并在配置的容器环境中运行对应检查。

### 手动验收

- 普通用户与管理员登录后均默认进入 Dashboard。
- 统计数字与其可访问相册一致。
- 共享相册预览不会访问未共享媒体。
- 空数据状态清晰可用。
- 快捷操作复用既有工作流。
- 任务在已有轮询机制下更新。
- 站点在线时间按当前语言正确格式化，并在 Dashboard 刷新后更新。
- 管理员卡片不会展示给普通用户。
- 桌面、移动端、明暗主题下均无文字溢出或界面重叠。

## 已确认决策

- Dashboard 是登录后的默认 tab。
- 首版包含运营概览、照片回顾、快捷操作和管理员概览。
- 使用单一聚合端点，以保证性能与权限一致性。
- 统计为基于现有相册索引的当前快照，不新增历史分析存储。
- 站点在线时间表示当前服务进程 uptime，服务重启后归零，不持久化累计。
- 共享相册只在当前用户已有访问权时参与聚合。